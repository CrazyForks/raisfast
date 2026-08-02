//! Logging initialization: outputs to both terminal and rolling files.
//!
//! Terminal uses a human-readable format (with colors), file uses JSON format
//! (for log analysis tools). Files roll daily (format `raisfast_YYYY-MM-DD.log`),
//! with expired files cleaned up periodically via a background task.
//!
//! # Environment variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `LOG_DIR` | `{STORAGE_ROOT_DIR}/logs` | Log file directory |
//! | `LOG_MAX_FILES` | `7` | Number of log files to retain |
//! | `RUST_LOG` | `raisfast=info,tower_http=info` | Log level filter |

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Custom daily-rolling log file writer.
///
/// Filename format: `{prefix}_YYYY-MM-DD.log`, automatically switches to a new file on date change.
struct DailyRollingWriter {
    dir: PathBuf,
    prefix: String,
    current_date: String,
    file: Option<File>,
}

impl DailyRollingWriter {
    fn new(dir: impl AsRef<Path>, prefix: &str) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
            prefix: prefix.to_string(),
            current_date: String::new(),
            file: None,
        }
    }

    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    fn filename(&self) -> String {
        format!("{}_{}.log", self.prefix, self.current_date)
    }

    fn ensure_file(&mut self) -> io::Result<()> {
        let today = Self::today();
        if today == self.current_date {
            if self.file.is_some() {
                return Ok(());
            }
        } else {
            self.current_date = today;
            self.file = None;
        }
        let path = self.dir.join(self.filename());
        self.file = Some(OpenOptions::new().append(true).create(true).open(path)?);
        Ok(())
    }
}

impl Write for DailyRollingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.ensure_file()?;
        match self.file {
            Some(ref mut f) => f.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut f) = self.file {
            f.flush()?;
        }
        Ok(())
    }
}

/// Initialize the logging system.
///
/// - Terminal: colored, human-readable format
/// - File: JSON format, daily rolling (`raisfast_YYYY-MM-DD.log`)
///
/// Returns the file appender guard. **The caller must hold it until program exit**,
/// otherwise file logging will stop prematurely.
pub fn init(log_dir: &str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("raisfast=info,tower_http=info"));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false);

    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("WARN: cannot create log dir '{log_dir}': {e}; file logging disabled");
        tracing_subscriber::registry()
            .with(filter)
            .with(stdout_layer)
            .init();
        return None;
    }

    let writer = DailyRollingWriter::new(log_dir, "raisfast");
    let (non_blocking, guard) = tracing_appender::non_blocking(writer);

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_writer(non_blocking)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    tracing::info!("logging initialized: stdout + file (dir={log_dir})");
    Some(guard)
}

/// Clean up expired log files, keeping only the latest `max_files`.
///
/// Sorts by filename (format `raisfast_YYYY-MM-DD.log`) and deletes the oldest.
/// Called once at startup, and can be called periodically thereafter.
pub fn cleanup_old_logs(log_dir: &str, max_files: usize) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };

    let mut files: Vec<String> = entries
        .filter_map(|e| {
            let entry = e.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("raisfast_") && name.ends_with(".log") {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    if files.len() <= max_files {
        return;
    }

    files.sort();
    let to_delete = files.len() - max_files;

    for name in &files[..to_delete] {
        let path = Path::new(log_dir).join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!(file = name, "removed old log file"),
            Err(e) => tracing::warn!(file = name, error = %e, "failed to remove old log file"),
        }
    }
}
