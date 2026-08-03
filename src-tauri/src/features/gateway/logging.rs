use chrono::Utc;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;

const LOG_SCHEMA_VERSION: u64 = 1;
const LOG_QUEUE_CAPACITY: usize = 1024;
const LOG_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;
const LOG_TOTAL_MAX_BYTES: u64 = 100 * 1024 * 1024;
const LOG_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const BODY_SNAPSHOT_MAX_BYTES: usize = 128 * 1024;
const SSE_CAPTURE_MAX_BYTES: usize = 256 * 1024;
const SSE_LINE_MAX_BYTES: usize = 1024 * 1024;
const SSE_EVENT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(super) struct GatewayLogger {
    sender: Option<SyncSender<LogCommand>>,
    dropped: Arc<AtomicU64>,
}

enum LogCommand {
    Record {
        value: Value,
        flush: bool,
    },
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}

#[derive(Clone)]
pub(super) struct RequestTrace {
    logger: Arc<GatewayLogger>,
    request_id: String,
    started: Instant,
    endpoint: String,
    method: String,
    account_alias: Option<String>,
    account_email: Option<String>,
    account_type: Option<String>,
    protocol: Option<String>,
    model: Option<String>,
    client_request_id: Option<String>,
    session_fingerprint: Option<String>,
}

impl GatewayLogger {
    pub(super) fn initialize(data_dir: &Path) -> Arc<Self> {
        let log_dir = data_dir.join("logs").join("gateway");
        if let Err(error) = prepare_log_dir(&log_dir) {
            eprintln!("Gateway log service unavailable: {error}");
            return Arc::new(Self {
                sender: None,
                dropped: Arc::new(AtomicU64::new(0)),
            });
        }
        let (sender, receiver) = mpsc::sync_channel(LOG_QUEUE_CAPACITY);
        thread::spawn(move || run_writer(receiver, log_dir));
        Arc::new(Self {
            sender: Some(sender),
            dropped: Arc::new(AtomicU64::new(0)),
        })
    }

    fn write(&self, mut value: Value, flush: bool) {
        let Some(sender) = &self.sender else {
            return;
        };
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            value["dropped_since_last"] = json!(dropped);
        }
        let command = LogCommand::Record { value, flush };
        if flush {
            if sender.send(command).is_err() {
                self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
            }
            return;
        }
        match sender.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn system_event(&self, level: &str, event: &str, fields: Value) {
        let mut value = fields.as_object().cloned().unwrap_or_default();
        insert_common(&mut value, level, event);
        self.write(redact(Value::Object(value)), level == "error");
    }

    #[cfg(test)]
    fn flush(&self) {
        let Some(sender) = &self.sender else {
            return;
        };
        let (done_tx, done_rx) = mpsc::channel();
        let _ = sender.send(LogCommand::Flush(done_tx));
        let _ = done_rx.recv_timeout(Duration::from_secs(2));
    }
}

impl RequestTrace {
    pub(super) fn new(
        logger: Arc<GatewayLogger>,
        method: &str,
        endpoint: &str,
        client_request_id: Option<String>,
        session_id: Option<&str>,
    ) -> Self {
        Self {
            logger,
            request_id: format!("gw_{}", Uuid::new_v4().simple()),
            started: Instant::now(),
            endpoint: endpoint.to_string(),
            method: method.to_string(),
            account_alias: None,
            account_email: None,
            account_type: None,
            protocol: None,
            model: None,
            client_request_id,
            session_fingerprint: session_id.map(fingerprint),
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.request_id
    }

    pub(super) fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    pub(super) fn set_account(
        &mut self,
        alias: &str,
        email: &str,
        account_type: &str,
        protocol: &str,
    ) {
        self.account_alias = Some(alias.to_string());
        self.account_email = (!email.is_empty()).then(|| email.to_string());
        self.account_type = Some(account_type.to_string());
        self.protocol = Some(protocol.to_string());
    }

    pub(super) fn set_model(&mut self, model: Option<&str>) {
        self.model = model.map(str::to_string);
    }

    pub(super) fn event(&self, level: &str, event: &str, fields: Value) {
        let mut value = fields.as_object().cloned().unwrap_or_default();
        insert_optional(&mut value, "account_alias", self.account_alias.as_deref());
        insert_optional(&mut value, "account_email", self.account_email.as_deref());
        insert_optional(&mut value, "account_type", self.account_type.as_deref());
        insert_optional(&mut value, "protocol", self.protocol.as_deref());
        insert_optional(&mut value, "model", self.model.as_deref());
        insert_optional(
            &mut value,
            "client_request_id",
            self.client_request_id.as_deref(),
        );
        insert_optional(
            &mut value,
            "session_fingerprint",
            self.session_fingerprint.as_deref(),
        );
        value.insert("gateway_request_id".to_string(), json!(self.request_id));
        value.insert("endpoint".to_string(), json!(self.endpoint));
        value.insert("method".to_string(), json!(self.method));
        insert_common(&mut value, level, event);
        self.logger.write(
            redact(Value::Object(value)),
            level == "error" || event.ends_with("failed") || event.ends_with("completed"),
        );
    }

    pub(super) fn payload(&self, event: &str, direction: &str, value: &Value) {
        self.event(
            "debug",
            event,
            json!({
                "direction": direction,
                "body": snapshot(value, BODY_SNAPSHOT_MAX_BYTES),
            }),
        );
    }

    pub(super) fn payload_bytes(&self, event: &str, direction: &str, bytes: &[u8]) {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => self.payload(event, direction, &value),
            Err(_) => self.event(
                "warn",
                event,
                json!({
                    "direction":direction,
                    "bytes":bytes.len(),
                    "sha256":hex_digest(Sha256::digest(bytes)),
                    "body_omitted":"non_json",
                }),
            ),
        }
    }
}

pub(super) struct ObservedReader<R: Read> {
    inner: R,
    observer: BodyObserver,
    finished: bool,
}

enum BodyObserver {
    Sse(SseObserver),
    Json(JsonObserver),
}

struct SseObserver {
    trace: RequestTrace,
    direction: &'static str,
    line: Vec<u8>,
    event_name: String,
    data: Vec<u8>,
    total_bytes: u64,
    captured_bytes: usize,
    event_count: u64,
    parse_errors: u64,
    terminal_event: Option<String>,
    truncated: bool,
    discard_line: bool,
    event_oversized: bool,
    digest: Sha256,
}

struct JsonObserver {
    trace: RequestTrace,
    direction: &'static str,
    body: Vec<u8>,
    total_bytes: u64,
    digest: Sha256,
}

impl<R: Read> ObservedReader<R> {
    pub(super) fn sse(inner: R, trace: RequestTrace, direction: &'static str) -> Self {
        Self {
            inner,
            observer: BodyObserver::Sse(SseObserver {
                trace,
                direction,
                line: Vec::new(),
                event_name: String::new(),
                data: Vec::new(),
                total_bytes: 0,
                captured_bytes: 0,
                event_count: 0,
                parse_errors: 0,
                terminal_event: None,
                truncated: false,
                discard_line: false,
                event_oversized: false,
                digest: Sha256::new(),
            }),
            finished: false,
        }
    }

    pub(super) fn json(inner: R, trace: RequestTrace, direction: &'static str) -> Self {
        Self {
            inner,
            observer: BodyObserver::Json(JsonObserver {
                trace,
                direction,
                body: Vec::new(),
                total_bytes: 0,
                digest: Sha256::new(),
            }),
            finished: false,
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        match &mut self.observer {
            BodyObserver::Sse(observer) => observer.finish(),
            BodyObserver::Json(observer) => observer.finish(),
        }
    }
}

impl<R: Read> Read for ObservedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(0) => {
                self.finish();
                Ok(0)
            }
            Ok(count) => {
                match &mut self.observer {
                    BodyObserver::Sse(observer) => observer.feed(&buffer[..count]),
                    BodyObserver::Json(observer) => observer.feed(&buffer[..count]),
                }
                Ok(count)
            }
            Err(error) => {
                let trace = match &self.observer {
                    BodyObserver::Sse(observer) => &observer.trace,
                    BodyObserver::Json(observer) => &observer.trace,
                };
                trace.event(
                    "error",
                    "stream.read_failed",
                    json!({"message": error.to_string()}),
                );
                self.finish();
                Err(error)
            }
        }
    }
}

impl<R: Read> Drop for ObservedReader<R> {
    fn drop(&mut self) {
        self.finish();
    }
}

impl SseObserver {
    fn feed(&mut self, bytes: &[u8]) {
        self.total_bytes += bytes.len() as u64;
        self.digest.update(bytes);
        for byte in bytes {
            if *byte == b'\n' {
                if !self.discard_line {
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    self.finish_line();
                }
                self.line.clear();
                self.discard_line = false;
            } else if !self.discard_line {
                if self.line.len() < SSE_LINE_MAX_BYTES {
                    self.line.push(*byte);
                } else {
                    self.parse_errors += 1;
                    self.discard_line = true;
                }
            }
        }
    }

    fn finish_line(&mut self) {
        if self.line.is_empty() {
            self.finish_event();
            return;
        }
        if let Some(value) = self.line.strip_prefix(b"event:") {
            self.event_name = String::from_utf8_lossy(value).trim().to_string();
        } else if let Some(value) = self.line.strip_prefix(b"data:") {
            if self.event_oversized {
                return;
            }
            let value = trim_ascii_start(value);
            let separator = usize::from(!self.data.is_empty());
            if self.data.len() + separator + value.len() > SSE_EVENT_MAX_BYTES {
                self.data.clear();
                self.event_oversized = true;
                return;
            }
            if separator > 0 {
                self.data.push(b'\n');
            }
            self.data.extend_from_slice(value);
        }
    }

    fn finish_event(&mut self) {
        if self.event_oversized {
            self.event_count += 1;
            self.parse_errors += 1;
            self.trace.event(
                "warn",
                "stream.event_oversized",
                json!({"direction":self.direction,"limit_bytes":SSE_EVENT_MAX_BYTES}),
            );
            self.event_name.clear();
            self.event_oversized = false;
            return;
        }
        if self.data.is_empty() {
            self.event_name.clear();
            return;
        }
        self.event_count += 1;
        if self.data == b"[DONE]" {
            self.terminal_event = Some("[DONE]".to_string());
            self.event_name.clear();
            self.data.clear();
            return;
        }
        let parsed = serde_json::from_slice::<Value>(&self.data);
        let event_name = (!self.event_name.is_empty())
            .then(|| self.event_name.clone())
            .or_else(|| {
                parsed
                    .as_ref()
                    .ok()
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "message".to_string());
        if matches!(
            event_name.as_str(),
            "response.completed" | "response.failed" | "response.incomplete"
        ) {
            self.terminal_event = Some(event_name.clone());
        }
        match parsed {
            Ok(value) if self.captured_bytes < SSE_CAPTURE_MAX_BYTES => {
                let remaining = SSE_CAPTURE_MAX_BYTES - self.captured_bytes;
                let body = snapshot(&value, remaining);
                self.captured_bytes += serde_json::to_vec(&body)
                    .map(|bytes| bytes.len().min(remaining))
                    .unwrap_or(0);
                self.trace.event(
                    "debug",
                    "stream.event",
                    json!({
                        "direction": self.direction,
                        "stream_event": event_name,
                        "body": body,
                    }),
                );
            }
            Ok(_) => self.mark_truncated(),
            Err(error) => {
                self.parse_errors += 1;
                self.trace.event(
                    "warn",
                    "stream.parse_failed",
                    json!({
                        "direction": self.direction,
                        "stream_event": event_name,
                        "message": error.to_string(),
                        "data_bytes": self.data.len(),
                    }),
                );
            }
        }
        self.event_name.clear();
        self.data.clear();
    }

    fn mark_truncated(&mut self) {
        if self.truncated {
            return;
        }
        self.truncated = true;
        self.trace.event(
            "warn",
            "stream.truncated",
            json!({"direction":self.direction,"limit_bytes":SSE_CAPTURE_MAX_BYTES}),
        );
    }

    fn finish(&mut self) {
        if !self.line.is_empty() && !self.discard_line {
            self.finish_line();
        }
        if !self.data.is_empty() || self.event_oversized {
            self.finish_event();
        }
        self.trace.event(
            if self.terminal_event.is_some() {
                "info"
            } else {
                "warn"
            },
            "stream.summary",
            json!({
                "direction":self.direction,
                "bytes":self.total_bytes,
                "sha256":hex_digest(self.digest.clone().finalize()),
                "event_count":self.event_count,
                "parse_errors":self.parse_errors,
                "terminal_event":self.terminal_event,
                "truncated":self.truncated,
            }),
        );
    }
}

impl JsonObserver {
    fn feed(&mut self, bytes: &[u8]) {
        self.total_bytes += bytes.len() as u64;
        self.digest.update(bytes);
        if self.body.len() <= BODY_SNAPSHOT_MAX_BYTES {
            let remaining = BODY_SNAPSHOT_MAX_BYTES + 1 - self.body.len();
            self.body
                .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        }
    }

    fn finish(&mut self) {
        let digest = hex_digest(self.digest.clone().finalize());
        if self.total_bytes as usize <= BODY_SNAPSHOT_MAX_BYTES {
            match serde_json::from_slice::<Value>(&self.body) {
                Ok(value) => self.trace.event(
                    "debug",
                    "response.body",
                    json!({
                        "direction":self.direction,
                        "bytes":self.total_bytes,
                        "sha256":digest,
                        "body":redact(value),
                    }),
                ),
                Err(error) => self.trace.event(
                    "warn",
                    "response.parse_failed",
                    json!({
                        "direction":self.direction,
                        "bytes":self.total_bytes,
                        "sha256":digest,
                        "message":error.to_string(),
                    }),
                ),
            }
        } else {
            self.trace.event(
                "warn",
                "response.truncated",
                json!({
                    "direction":self.direction,
                    "bytes":self.total_bytes,
                    "sha256":digest,
                    "limit_bytes":BODY_SNAPSHOT_MAX_BYTES,
                }),
            );
        }
    }
}

fn insert_common(value: &mut Map<String, Value>, level: &str, event: &str) {
    value.insert("schema_version".to_string(), json!(LOG_SCHEMA_VERSION));
    value.insert("timestamp".to_string(), json!(Utc::now().to_rfc3339()));
    value.insert("level".to_string(), json!(level));
    value.insert("event".to_string(), json!(event));
}

fn insert_optional(value: &mut Map<String, Value>, key: &str, field: Option<&str>) {
    if let Some(field) = field.filter(|field| !field.is_empty()) {
        value.insert(key.to_string(), json!(field));
    }
}

fn snapshot(value: &Value, limit: usize) -> Value {
    let sanitized = redact(value.clone());
    let bytes = serde_json::to_vec(&sanitized).unwrap_or_default();
    if bytes.len() <= limit {
        return sanitized;
    }
    json!({
        "truncated":true,
        "original_bytes":bytes.len(),
        "sha256":hex_digest(Sha256::digest(&bytes)),
        "preview":String::from_utf8_lossy(&bytes[..limit]).into_owned(),
    })
}

fn redact(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(redact).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(&key) {
                        json!("[redacted]")
                    } else {
                        redact(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::String(value) if value.to_ascii_lowercase().starts_with("bearer ") => {
            json!("Bearer [redacted]")
        }
        value => value,
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        key.as_str(),
        "authorization"
            | "cookie"
            | "setcookie"
            | "xapikey"
            | "apikey"
            | "token"
            | "password"
            | "secret"
            | "clientsecret"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "authjson"
            | "chatgptaccountid"
    ) || key.ends_with("token")
        || key.ends_with("apikey")
}

fn fingerprint(value: &str) -> String {
    hex_digest(Sha256::digest(value.as_bytes()))[..16].to_string()
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn trim_ascii_start(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    value
}

fn prepare_log_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn run_writer(receiver: Receiver<LogCommand>, log_dir: PathBuf) {
    let mut writer = LogWriter::new(log_dir);
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(LogCommand::Record { value, flush }) => {
                if let Err(error) = writer.write(&value, flush) {
                    eprintln!("Gateway log write failed: {error}");
                }
            }
            #[cfg(test)]
            Ok(LogCommand::Flush(done)) => {
                let _ = writer.flush();
                let _ = done.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = writer.flush();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = writer.flush();
                break;
            }
        }
    }
}

struct LogWriter {
    directory: PathBuf,
    date: String,
    index: u32,
    bytes: u64,
    file: Option<BufWriter<File>>,
}

impl LogWriter {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            date: String::new(),
            index: 0,
            bytes: 0,
            file: None,
        }
    }

    fn write(&mut self, value: &Value, flush: bool) -> io::Result<()> {
        let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
        line.push(b'\n');
        let date = Utc::now().format("%Y-%m-%d").to_string();
        if self.file.is_none()
            || self.date != date
            || self.bytes + line.len() as u64 > LOG_FILE_MAX_BYTES
        {
            self.rotate(&date)?;
        }
        let file = self.file.as_mut().expect("opened by rotate");
        file.write_all(&line)?;
        self.bytes += line.len() as u64;
        if flush {
            file.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }

    fn rotate(&mut self, date: &str) -> io::Result<()> {
        self.flush()?;
        self.file = None;
        cleanup_logs(
            &self.directory,
            LOG_TOTAL_MAX_BYTES.saturating_sub(LOG_FILE_MAX_BYTES),
        )?;
        self.index = if self.date == date {
            self.index + 1
        } else {
            next_log_index(&self.directory, date)?
        };
        self.date = date.to_string();
        let path = self
            .directory
            .join(format!("gateway-{date}-{:03}.jsonl", self.index));
        let file = open_private_append(&path)?;
        self.bytes = file.metadata()?.len();
        self.file = Some(BufWriter::new(file));
        Ok(())
    }
}

fn open_private_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn next_log_index(directory: &Path, date: &str) -> io::Result<u32> {
    let prefix = format!("gateway-{date}-");
    let mut maximum = None;
    for entry in fs::read_dir(directory)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        let Some(value) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".jsonl"))
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        maximum = Some(maximum.map_or(value, |current: u32| current.max(value)));
    }
    Ok(maximum.map_or(0, |value| value + 1))
}

fn cleanup_logs(directory: &Path, max_total_bytes: u64) -> io::Result<()> {
    let now = SystemTime::now();
    let mut files = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((path, metadata.len(), metadata.modified().ok()?))
        })
        .collect::<Vec<_>>();
    for (path, _, modified) in &files {
        if now
            .duration_since(*modified)
            .is_ok_and(|age| age > LOG_RETENTION)
        {
            let _ = fs::remove_file(path);
        }
    }
    files.retain(|(path, _, _)| path.exists());
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    for (path, size, _) in files {
        if total <= max_total_bytes {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_and_limits_snapshots() {
        let value = json!({
            "authorization":"Bearer secret-value",
            "nested":{
                "refresh_token":"secret",
                "client_secret":"secret",
                "apiKey":"secret",
                "accessToken":"secret",
                "input":"hello"
            }
        });
        let redacted = snapshot(&value, BODY_SNAPSHOT_MAX_BYTES);
        assert_eq!(redacted["authorization"], "[redacted]");
        assert_eq!(redacted["nested"]["refresh_token"], "[redacted]");
        assert_eq!(redacted["nested"]["client_secret"], "[redacted]");
        assert_eq!(redacted["nested"]["apiKey"], "[redacted]");
        assert_eq!(redacted["nested"]["accessToken"], "[redacted]");
        assert_eq!(redacted["nested"]["input"], "hello");
        assert_eq!(
            snapshot(&json!({"value":"x".repeat(32)}), 8)["truncated"],
            true
        );
    }

    #[test]
    fn keeps_terminal_events_when_the_detail_queue_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let logger = Arc::new(GatewayLogger {
            sender: Some(sender),
            dropped: Arc::new(AtomicU64::new(0)),
        });
        logger.system_event("debug", "stream.event", json!({}));
        let worker = {
            let logger = logger.clone();
            thread::spawn(move || logger.system_event("error", "request.failed", json!({})))
        };
        let _ = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let terminal = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        let LogCommand::Record { value, .. } = terminal else {
            panic!("expected record");
        };
        assert_eq!(value["event"], "request.failed");
    }

    #[test]
    fn writes_correlated_jsonl_without_exposing_session_id() {
        let directory =
            std::env::temp_dir().join(format!("cortana-gateway-log-{}", Uuid::new_v4()));
        let logger = GatewayLogger::initialize(&directory);
        let trace = RequestTrace::new(
            logger.clone(),
            "POST",
            "/v1/responses",
            Some("client-1".to_string()),
            Some("session-secret"),
        );
        trace.event("info", "request.received", json!({}));
        logger.flush();
        let log_dir = directory.join("logs/gateway");
        let file = fs::read_dir(&log_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(file).unwrap();
        let value: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["gateway_request_id"], trace.id());
        assert_eq!(value["client_request_id"], "client-1");
        assert!(!text.contains("session-secret"));
    }

    #[test]
    fn observes_sse_terminal_event_without_changing_bytes() {
        let directory =
            std::env::temp_dir().join(format!("cortana-gateway-sse-{}", Uuid::new_v4()));
        let logger = GatewayLogger::initialize(&directory);
        let trace = RequestTrace::new(logger.clone(), "POST", "/v1/responses", None, None);
        let input = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n";
        let mut observed = ObservedReader::sse(input.as_slice(), trace, "upstream");
        let mut output = Vec::new();
        observed.read_to_end(&mut output).unwrap();
        drop(observed);
        logger.flush();
        assert_eq!(output, input);
        let log_dir = directory.join("logs/gateway");
        let text = fs::read_to_string(
            fs::read_dir(log_dir)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(text.contains("response.completed"));
        assert!(text.contains("stream.summary"));
    }

    #[test]
    fn caps_multiline_sse_event_capture() {
        let directory =
            std::env::temp_dir().join(format!("cortana-gateway-sse-cap-{}", Uuid::new_v4()));
        let logger = GatewayLogger::initialize(&directory);
        let trace = RequestTrace::new(logger.clone(), "POST", "/v1/responses", None, None);
        let chunk = "x".repeat(SSE_EVENT_MAX_BYTES / 2 + 1);
        let input = format!("data: {chunk}\ndata: {chunk}\n\n");
        let mut observed = ObservedReader::sse(input.as_bytes(), trace, "upstream");
        io::copy(&mut observed, &mut io::sink()).unwrap();
        drop(observed);
        logger.flush();
        let log_dir = directory.join("logs/gateway");
        let text = fs::read_to_string(
            fs::read_dir(log_dir)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(text.contains("stream.event_oversized"));
    }

    #[test]
    fn cleanup_removes_expired_and_oldest_oversized_logs() {
        let directory =
            std::env::temp_dir().join(format!("cortana-gateway-cleanup-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let now = SystemTime::now();
        let files = [
            (
                "expired.jsonl",
                1,
                now - LOG_RETENTION - Duration::from_secs(1),
            ),
            (
                "older.jsonl",
                60 * 1024 * 1024,
                now - Duration::from_secs(2),
            ),
            (
                "newer.jsonl",
                60 * 1024 * 1024,
                now - Duration::from_secs(1),
            ),
        ];
        for (name, size, modified) in files {
            let file = File::create(directory.join(name)).unwrap();
            file.set_len(size).unwrap();
            file.set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }

        cleanup_logs(
            &directory,
            LOG_TOTAL_MAX_BYTES.saturating_sub(LOG_FILE_MAX_BYTES),
        )
        .unwrap();

        assert!(!directory.join("expired.jsonl").exists());
        assert!(!directory.join("older.jsonl").exists());
        assert!(directory.join("newer.jsonl").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
