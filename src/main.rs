//! Blog 系统入口。
//!
//! 默认行为：启动 HTTP 服务器。通过子命令切换到其他操作。
//!
//! # 子命令
//!
//! - `server start`    启动服务器（默认）
//! - `server stop`     停止运行中的服务器
//! - `server restart`  重启服务器
//! - `server status`   查看运行状态
//! - `db migrate`      执行数据库迁移
//! - `db backup`       备份数据库

#![deny(unsafe_code)]

rust_i18n::i18n!("locales", fallback = "en");

use clap::Parser;
use raisfast::config::app::AppConfig;

mod cli;
mod logging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    let config = AppConfig::init();

    let _log_guard = logging::init(&config.log_dir);

    logging::cleanup_old_logs(&config.log_dir, config.log_max_files);

    let log_dir = config.log_dir.clone();
    let max_files = config.log_max_files;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            logging::cleanup_old_logs(&log_dir, max_files);
        }
    });

    cli::run(cli, &config).await?;

    Ok(())
}
