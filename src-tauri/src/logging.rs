use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Mutex, OnceLock,
    },
};
use zip::{write::SimpleFileOptions, ZipWriter};

const BACKEND_LOG_FILE: &str = "backend.log";
const FRONTEND_LOG_FILE: &str = "frontend.log";
const LOG_FILES: [&str; 2] = [BACKEND_LOG_FILE, FRONTEND_LOG_FILE];
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;
const ROTATED_FILES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            _ => Err("日志级别必须是 error、warn、info、debug 或 trace".into()),
        }
    }
}

struct Logger {
    directory: PathBuf,
    level: AtomicU8,
    backend_file: Mutex<Option<File>>,
    frontend_file: Mutex<Option<File>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();
static INIT_ERROR: OnceLock<String> = OnceLock::new();
static TASK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn init(level: &str) -> Result<PathBuf, String> {
    let result = init_inner(level);
    if let Err(error) = &result {
        let _ = INIT_ERROR.set(error.clone());
    }
    result
}

fn init_inner(level: &str) -> Result<PathBuf, String> {
    if let Some(logger) = LOGGER.get() {
        set_level(level)?;
        return Ok(logger.directory.clone());
    }
    let parsed = Level::from_str(level)?;
    let executable =
        std::env::current_exe().map_err(|error| format!("无法确定应用程序所在目录：{error}"))?;
    let parent = application_directory(&executable)?;
    let directory = parent.join("logs");
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "无法在应用程序目录创建日志文件夹 {}：{error}",
            directory.display()
        )
    })?;
    let backend_file = open_log_file(&directory, BACKEND_LOG_FILE).map_err(|error| {
        format!(
            "无法写入应用程序目录中的日志文件 {}：{error}",
            directory.join(BACKEND_LOG_FILE).display()
        )
    })?;
    let frontend_file = open_log_file(&directory, FRONTEND_LOG_FILE).map_err(|error| {
        format!(
            "无法写入应用程序目录中的日志文件 {}：{error}",
            directory.join(FRONTEND_LOG_FILE).display()
        )
    })?;
    LOGGER
        .set(Logger {
            directory: directory.clone(),
            level: AtomicU8::new(parsed as u8),
            backend_file: Mutex::new(Some(backend_file)),
            frontend_file: Mutex::new(Some(frontend_file)),
        })
        .map_err(|_| "日志系统已被初始化".to_string())?;
    info(
        "app",
        "logger_initialized",
        "日志系统已启动",
        json!({"directory": directory, "level": level}),
    );
    Ok(directory)
}

fn open_log_file(directory: &Path, name: &str) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(name))
}

pub fn set_level(level: &str) -> Result<(), String> {
    let parsed = Level::from_str(level)?;
    if let Some(logger) = LOGGER.get() {
        logger.level.store(parsed as u8, Ordering::Relaxed);
    }
    Ok(())
}

pub fn directory() -> Result<PathBuf, String> {
    LOGGER
        .get()
        .map(|logger| logger.directory.clone())
        .ok_or_else(|| {
            INIT_ERROR
                .get()
                .cloned()
                .unwrap_or_else(|| "日志系统尚未初始化".to_string())
        })
}

pub fn task_id() -> String {
    format!(
        "{}-{:04}",
        Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        TASK_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn error(target: &str, event: &str, message: &str, context: Value) {
    write(Level::Error, target, event, message, context)
}

pub fn warn(target: &str, event: &str, message: &str, context: Value) {
    write(Level::Warn, target, event, message, context)
}

pub fn info(target: &str, event: &str, message: &str, context: Value) {
    write(Level::Info, target, event, message, context)
}

pub fn debug(target: &str, event: &str, message: &str, context: Value) {
    write(Level::Debug, target, event, message, context)
}

pub fn trace(target: &str, event: &str, message: &str, context: Value) {
    write(Level::Trace, target, event, message, context)
}

pub fn frontend(level: &str, event: &str, message: &str, context: Value) -> Result<(), String> {
    let level = Level::from_str(level)?;
    write_to(
        level,
        "frontend",
        event,
        message,
        context,
        FRONTEND_LOG_FILE,
    );
    Ok(())
}

fn write(level: Level, target: &str, event: &str, message: &str, context: Value) {
    write_to(level, target, event, message, context, BACKEND_LOG_FILE);
}

fn write_to(
    level: Level,
    target: &str,
    event: &str,
    message: &str,
    context: Value,
    log_name: &str,
) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    if level as u8 > logger.level.load(Ordering::Relaxed) {
        return;
    }
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let event = clean_identifier(event);
    let message = clean_text(message, 800);
    let context = clean_value(context, 0);
    let line = format!(
        "{timestamp} {:<5} [{target}] {event} - {message} | {}\n",
        level.as_str(),
        context
    );
    let file = if log_name == FRONTEND_LOG_FILE {
        &logger.frontend_file
    } else {
        &logger.backend_file
    };
    let Ok(mut file) = file.lock() else {
        return;
    };
    if file
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map(|metadata| metadata.len() + line.len() as u64 > MAX_FILE_SIZE)
        .unwrap_or(false)
    {
        file.take();
        let _ = rotate(&logger.directory, log_name);
        *file = open_log_file(&logger.directory, log_name).ok();
    }
    if let Some(file) = file.as_mut() {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

fn clean_identifier(value: &str) -> String {
    value
        .chars()
        .take(80)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn application_directory(executable: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = executable.ancestors().find(|path| {
        path.extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("app"))
    }) {
        return bundle
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "无法确定 .app 所在目录".to_string());
    }
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "无法确定应用程序所在目录".to_string())
}

fn clean_text(value: &str, limit: usize) -> String {
    let single_line = value.replace(['\r', '\n'], " ");
    let mut chars = single_line.chars();
    let shortened = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn clean_value(value: Value, depth: usize) -> Value {
    if depth > 5 {
        return Value::String("[TRUNCATED]".into());
    }
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let hidden = is_sensitive_key(&key);
                    let value = if hidden {
                        Value::String("[REDACTED]".into())
                    } else if key.eq_ignore_ascii_case("error") {
                        match value {
                            Value::String(value) => Value::String(clean_error(&value)),
                            value => clean_value(value, depth + 1),
                        }
                    } else {
                        clean_value(value, depth + 1)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(50)
                .map(|value| clean_value(value, depth + 1))
                .collect(),
        ),
        Value::String(value) => Value::String(clean_text(&value, 800)),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey"
            | "authorization"
            | "password"
            | "passwd"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "secret"
            | "clientsecret"
    )
}

fn clean_error(value: &str) -> String {
    let value = value.trim_start();
    if value.starts_with("HTTP ") {
        return value.split(':').next().unwrap_or("HTTP error").to_string();
    }
    clean_text(value, 400)
}

fn rotate(directory: &Path, log_name: &str) -> io::Result<()> {
    let oldest = directory.join(format!("{log_name}.{ROTATED_FILES}"));
    if oldest.exists() {
        fs::remove_file(oldest)?;
    }
    for index in (1..ROTATED_FILES).rev() {
        let source = directory.join(format!("{log_name}.{index}"));
        if source.exists() {
            fs::rename(source, directory.join(format!("{log_name}.{}", index + 1)))?;
        }
    }
    let current = directory.join(log_name);
    if current.exists() {
        fs::rename(current, directory.join(format!("{log_name}.1")))?;
    }
    Ok(())
}

pub fn export(dest: &Path) -> Result<(), String> {
    let directory = directory()?;
    export_directory(&directory, dest)
}

fn export_directory(directory: &Path, dest: &Path) -> Result<(), String> {
    let file = File::create(dest).map_err(|error| format!("无法创建日志压缩包：{error}"))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for log_name in LOG_FILES {
        for index in 0..=ROTATED_FILES {
            let name = if index == 0 {
                log_name.to_string()
            } else {
                format!("{log_name}.{index}")
            };
            let path = directory.join(&name);
            if !path.exists() {
                continue;
            }
            let mut source = File::open(&path).map_err(|error| error.to_string())?;
            let mut contents = Vec::new();
            source
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            zip.start_file(name, options)
                .map_err(|error| error.to_string())?;
            zip.write_all(&contents)
                .map_err(|error| error.to_string())?;
        }
    }
    zip.finish().map_err(|error| error.to_string())?;
    Ok(())
}

pub fn open_directory() -> Result<(), String> {
    let directory = directory()?;
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(&directory)
        .spawn()
        .map_err(|error| format!("无法打开日志目录 {}：{error}", directory.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_log_levels() {
        assert_eq!(Level::from_str("debug").unwrap(), Level::Debug);
        assert!(Level::from_str("verbose").is_err());
    }

    #[test]
    fn removes_sensitive_structured_fields() {
        let value = clean_value(
            json!({
                "api_key":"abc",
                "nested":{"accessToken":"xyz","client-secret":"hidden"},
                "entry_key":"quest.title"
            }),
            0,
        );
        assert_eq!(value["api_key"], "[REDACTED]");
        assert_eq!(value["nested"]["accessToken"], "[REDACTED]");
        assert_eq!(value["nested"]["client-secret"], "[REDACTED]");
        assert_eq!(value["entry_key"], "quest.title");
    }

    #[test]
    fn removes_http_response_bodies_from_errors() {
        let value = clean_value(
            json!({"error":"  HTTP 429 Too Many Requests: echoed source text"}),
            0,
        );
        assert_eq!(value["error"], "HTTP 429 Too Many Requests");
    }

    #[test]
    fn rotation_keeps_the_active_and_previous_files_in_order() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(BACKEND_LOG_FILE), "current").unwrap();
        fs::write(
            directory.path().join(format!("{BACKEND_LOG_FILE}.1")),
            "previous",
        )
        .unwrap();
        rotate(directory.path(), BACKEND_LOG_FILE).unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("{BACKEND_LOG_FILE}.1"))).unwrap(),
            "current"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(format!("{BACKEND_LOG_FILE}.2"))).unwrap(),
            "previous"
        );
    }

    #[test]
    fn export_contains_both_frontend_and_backend_logs() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(BACKEND_LOG_FILE), "backend event").unwrap();
        fs::write(directory.path().join(FRONTEND_LOG_FILE), "frontend event").unwrap();
        let destination = directory.path().join("diagnostics.zip");
        export_directory(directory.path(), &destination).unwrap();
        let file = File::open(destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name(BACKEND_LOG_FILE).is_ok());
        assert!(archive.by_name(FRONTEND_LOG_FILE).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stores_logs_next_to_the_macos_app_bundle() {
        let executable =
            Path::new("/Applications/FTB Translator.app/Contents/MacOS/ftb-translator");
        assert_eq!(
            application_directory(executable).unwrap(),
            Path::new("/Applications")
        );
    }
}
