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
//! - `app new`        Scaffold a new project (no config required)

#![deny(unsafe_code)]

rust_i18n::i18n!("locales", fallback = "en");

mod cli;
mod logging;

pub(crate) mod db {
    pub use raisfast::db::*;
}

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI before config init: `--help`/`--version` and config-free
    // commands (`app new`) must work without DATABASE_URL set
    // (required on non-SQLite builds).
    let cli = cli::Cli::parse();
    cli::run(cli).await
}
