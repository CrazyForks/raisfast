//! 日志初始化：同时输出到终端和滚动文件。
//!
//! 终端使用人类可读格式（带颜色），文件使用 JSON 格式（便于日志分析工具解析）。
//! 文件按天滚动（格式 `raisfast_YYYY-MM-DD.log`），通过后台任务定期清理过期文件。
//!
//! # 环境变量
//!
//! | 变量 | 默认值 | 说明 |
//! |------|--------|------|
//! | `LOG_DIR` | `./logs` | 日志文件目录 |
//! | `LOG_MAX_FILES` | `7` | 保留的日志文件数量 |
//! | `RUST_LOG` | `raisfast=info,tower_http=info` | 日志级别过滤 |

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// 自定义按天滚动的日志文件写入器。
///
/// 文件名格式：`{prefix}_YYYY-MM-DD.log`，日期变更时自动切换到新文件。
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

/// 初始化日志系统。
///
/// - 终端：彩色、人类可读格式
/// - 文件：JSON 格式、按天滚动（`raisfast_YYYY-MM-DD.log`）
///
/// 返回文件 appender 的 guard，**调用者必须持有它直到程序退出**，
/// 否则文件日志会提前停止写入。
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

/// 清理过期的日志文件，只保留最新的 `max_files` 个。
///
/// 按文件名排序（格式 `raisfast_YYYY-MM-DD.log`），
/// 删除最旧的文件。启动时调用一次，之后可周期性调用。
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
