use filetime::{set_file_mtime, FileTime};
use rand::RngExt;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

const BUSY_ERROR: &str = "Pi 正在更新认证信息，暂时无法切换账号，请稍后重试。";

#[derive(Clone, Copy)]
struct LockPolicy {
    stale: Duration,
    heartbeat: Duration,
    timeout: Duration,
}

const DEFAULT_POLICY: LockPolicy = LockPolicy {
    stale: Duration::from_secs(30),
    // Pi also takes synchronous proper-lockfile locks with the 10s default stale time.
    heartbeat: Duration::from_secs(4),
    timeout: Duration::from_secs(30),
};

pub(crate) struct PiAuthLock {
    path: PathBuf,
    mtime: Arc<Mutex<SystemTime>>,
    stop: Option<Sender<()>>,
    heartbeat: Option<JoinHandle<()>>,
}

impl PiAuthLock {
    pub(crate) fn acquire(auth_path: &Path) -> Result<Self, String> {
        Self::acquire_with_policy(auth_path, DEFAULT_POLICY)
    }

    fn acquire_with_policy(auth_path: &Path, policy: LockPolicy) -> Result<Self, String> {
        ensure_parent(auth_path)?;
        let path = lock_path(auth_path);
        let started = Instant::now();
        loop {
            match fs::create_dir(&path) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|mtime| mtime.elapsed().ok())
                        .is_some_and(|age| age >= policy.stale);
                    if stale && fs::remove_dir(&path).is_ok() {
                        continue;
                    }
                    if started.elapsed() >= policy.timeout {
                        return Err(BUSY_ERROR.to_string());
                    }
                    thread::sleep(Duration::from_millis(rand::rng().random_range(20..=80)));
                }
                Err(error) => return Err(format!("无法锁定 Pi 认证文件：{error}")),
            }
        }

        let mtime = Arc::new(Mutex::new(
            fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .map_err(|error| format!("无法读取 Pi 认证锁：{error}"))?,
        ));
        let (stop, receiver) = mpsc::channel();
        let heartbeat_path = path.clone();
        let heartbeat_mtime = Arc::clone(&mtime);
        let heartbeat = thread::spawn(move || {
            while let Err(mpsc::RecvTimeoutError::Timeout) = receiver.recv_timeout(policy.heartbeat)
            {
                let Ok(mut owned) = heartbeat_mtime.lock() else {
                    break;
                };
                if fs::metadata(&heartbeat_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .as_ref()
                    != Some(&*owned)
                {
                    break;
                }
                if set_file_mtime(&heartbeat_path, FileTime::now()).is_err() {
                    break;
                }
                let Ok(updated) =
                    fs::metadata(&heartbeat_path).and_then(|metadata| metadata.modified())
                else {
                    break;
                };
                *owned = updated;
            }
        });

        Ok(Self {
            path,
            mtime,
            stop: Some(stop),
            heartbeat: Some(heartbeat),
        })
    }

    pub(crate) fn ensure_healthy(&self) -> Result<(), String> {
        let owned = self
            .mtime
            .lock()
            .map_err(|_| "Pi 认证锁状态不可用。".to_string())?;
        let current = fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .map_err(|_| "Pi 认证锁已失效，已停止操作。".to_string())?;
        if current != *owned {
            return Err("Pi 认证锁已被其他进程接管，已停止操作。".to_string());
        }
        Ok(())
    }
}

impl Drop for PiAuthLock {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        if self.ensure_healthy().is_ok() {
            let _ = fs::remove_dir(&self.path);
        }
    }
}

fn lock_path(auth_path: &Path) -> PathBuf {
    let mut value = auth_path.as_os_str().to_os_string();
    value.push(".lock");
    PathBuf::from(value)
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Pi 认证路径无效。".to_string())?;
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|error| format!("无法创建 Pi 配置目录：{error}"))?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_auth() -> PathBuf {
        std::env::temp_dir()
            .join(format!("cortana-pi-lock-{}", uuid::Uuid::new_v4()))
            .join("auth.json")
    }

    #[test]
    fn lock_is_exclusive_and_released() {
        let path = temp_auth();
        let policy = LockPolicy {
            stale: Duration::from_secs(1),
            heartbeat: Duration::from_millis(50),
            timeout: Duration::from_millis(100),
        };
        let guard = PiAuthLock::acquire_with_policy(&path, policy).unwrap();
        assert!(lock_path(&path).is_dir());
        assert!(PiAuthLock::acquire_with_policy(&path, policy).is_err());
        drop(guard);
        assert!(!lock_path(&path).exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
