//! CLI 定义与子命令分发。
//!
//! 使用 clap derive 定义命令行结构，将每个子命令分发到对应模块执行。

mod db_cmd;
mod server_cmd;

use hello_axum::config::app::AppConfig;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hello-axum", version, about = "Blog system built with Axum")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Server management
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// Database management
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Start the HTTP server (default if no subcommand given)
    Start,
    /// Stop the running server
    Stop,
    /// Restart the server
    Restart,
    /// Show server status
    Status,
}

#[derive(Subcommand)]
pub enum DbAction {
    /// Run pending database migrations
    Migrate,
    /// Backup the database to a timestamped file
    Backup {
        /// Output directory (default: ./backups)
        #[arg(short, long, default_value = "./backups")]
        output: String,
    },
}

pub async fn run(cli: Cli, config: &AppConfig) -> anyhow::Result<()> {
    match cli.command {
        None
        | Some(Commands::Server {
            action: ServerAction::Start,
        }) => {
            server_cmd::start(config).await?;
        }

        Some(Commands::Server {
            action: ServerAction::Stop,
        }) => {
            server_cmd::stop();
        }

        Some(Commands::Server {
            action: ServerAction::Restart,
        }) => {
            server_cmd::restart(config).await?;
        }

        Some(Commands::Server {
            action: ServerAction::Status,
        }) => {
            server_cmd::status();
        }

        Some(Commands::Db {
            action: DbAction::Migrate,
        }) => {
            db_cmd::migrate(config).await?;
        }

        Some(Commands::Db {
            action: DbAction::Backup { output },
        }) => {
            db_cmd::backup(config, &output)?;
        }
    }

    Ok(())
}
