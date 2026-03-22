use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{FixedOffset, Utc};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::{filter::LevelFilter, fmt::writer::MakeWriter, FmtSubscriber};

use crate::config::LoggingConfig;

const JST_OFFSET_SECS: i32 = 9 * 60 * 60;

#[derive(Clone)]
struct SharedFileWriter {
    file: Arc<Mutex<File>>,
}

impl<'a> MakeWriter<'a> for SharedFileWriter {
    type Writer = LockedFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LockedFileWriter {
            file: Arc::clone(&self.file),
        }
    }
}

struct LockedFileWriter {
    file: Arc<Mutex<File>>,
}

struct JstTimer;

impl Write for LockedFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log file lock poisoned"))?;
        file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log file lock poisoned"))?;
        file.flush()
    }
}

impl FormatTime for JstTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", current_timestamp_jst())
    }
}

pub fn init(config: &LoggingConfig) -> Result<PathBuf, String> {
    let path = PathBuf::from(&config.path);
    let file = open_log_file(&path)?;
    let level = parse_level(&config.level)?;
    let writer = SharedFileWriter {
        file: Arc::new(Mutex::new(file)),
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_ansi(false)
        .with_timer(JstTimer)
        .with_writer(writer)
        .with_target(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|err| format!("failed to set tracing subscriber: {err}"))?;

    Ok(path)
}

pub fn open_log_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create log directory {}: {err}", parent.display()))?;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("failed to open log file {}: {err}", path.display()))
}

fn parse_level(level: &str) -> Result<LevelFilter, String> {
    match level.to_ascii_lowercase().as_str() {
        "trace" => Ok(LevelFilter::TRACE),
        "debug" => Ok(LevelFilter::DEBUG),
        "info" => Ok(LevelFilter::INFO),
        "warn" => Ok(LevelFilter::WARN),
        "error" => Ok(LevelFilter::ERROR),
        _ => Err(format!("unsupported log level: {level}")),
    }
}

fn current_timestamp_jst() -> String {
    let offset = FixedOffset::east_opt(JST_OFFSET_SECS).expect("JST offset should be valid");
    Utc::now()
        .with_timezone(&offset)
        .format("%Y-%m-%d %H:%M:%S %z")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{current_timestamp_jst, open_log_file, parse_level};

    #[test]
    fn creates_parent_directories_for_log_path() {
        let dir = unique_test_dir("create-log-dir");
        let path = dir.join("nested").join("cranberry.log");

        open_log_file(&path).expect("log file should be created");

        assert!(path.exists());
    }

    #[test]
    fn rejects_invalid_log_level() {
        let err = parse_level("verbose").expect_err("level should be rejected");

        assert!(err.contains("unsupported log level"));
    }

    #[test]
    fn fails_when_log_path_is_a_directory() {
        let dir = unique_test_dir("directory-log-path");
        std::fs::create_dir_all(&dir).expect("directory should exist");

        let err = open_log_file(&dir).expect_err("opening a directory as a file should fail");
        assert!(err.contains("failed to open log file"));
    }

    #[test]
    fn formats_timestamps_in_jst() {
        assert!(current_timestamp_jst().ends_with("+0900"));
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        std::env::temp_dir().join(format!("cranberry-{prefix}-{}-{nanos}", std::process::id()))
    }
}
