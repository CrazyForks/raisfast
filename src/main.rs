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
use hello_axum::config::app::AppConfig;
use tracing_subscriber::EnvFilter;

mod cli;
mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    if let Err(e) = dotenvy::dotenv() {
        eprintln!(".env file not loaded: {}", e);
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = AppConfig::init();

    cli::run(cli, &config).await?;

    Ok(())
}
