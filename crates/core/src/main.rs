//! RaisFast: Rust-powered high-performance BaaS and headless CMS.
//!
//! Default behavior: starts the HTTP server. Use subcommands to switch to other operations.
//!
//! # Subcommands
//!
//! - `server start`    Start the server (default)
//! - `server stop`     Stop the running server
//! - `server restart`  Restart the server
//! - `server status`   Check running status
//! - `db migrate`      Run database migrations
//! - `db rollback`    Rollback last batch of migrations
//! - `db backup`      Backup the database

#![deny(unsafe_code)]

rust_i18n::i18n!("locales", fallback = "en");

use clap::Parser;
use raisfast::config::app::AppConfig;

mod cli;
mod logging;

pub(crate) mod db {
    pub use raisfast::db::*;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::init();
    cli::print_banner(&config);

    let cli = cli::Cli::parse();

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
