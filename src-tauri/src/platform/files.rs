use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use uuid::Uuid;

pub(crate) fn write_file_atomically(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Codex 目录无效。".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建 Codex 目录：{error}"))?;
    let temp_path = parent.join(format!(".codex-write-{}.tmp", Uuid::new_v4()));
    {
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| error.to_string())?;
        temporary
            .write_all(content.as_bytes())
            .map_err(|error| error.to_string())?;
        temporary.sync_all().map_err(|error| error.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temp_path, path).map_err(|error| error.to_string())
}
