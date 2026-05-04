//! CLI 定义与子命令分发。
//!
//! 使用 clap derive 定义命令行结构，将每个子命令分发到对应模块执行。

mod ct_cmd;
mod db_cmd;
mod plugin_cmd;
mod server_cmd;

use raisfast::config::app::AppConfig;

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
    /// Content type management
    Ct {
        #[command(subcommand)]
        action: CtAction,
    },
    /// Plugin management
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
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
    /// Seed initial data (admin user, default content types)
    Seed {
        /// Admin email (default: admin@raisfast.dev)
        #[arg(long, default_value = "admin@raisfast.dev")]
        email: String,
        /// Admin username (default: admin)
        #[arg(long, default_value = "admin")]
        username: String,
        /// Admin password (default: admin123)
        #[arg(long, default_value = "admin123")]
        password: String,
    },
}

#[derive(Subcommand)]
pub enum CtAction {
    /// Create a new content type TOML file
    New {
        /// Content type name (e.g. "product")
        name: String,
    },
    /// Validate content type TOML files
    Check {
        /// Path to check (default: content_type_dir)
        path: Option<String>,
    },
    /// Generate TypeScript types from content type TOML files
    Types {
        /// Specific content type singular name (e.g. "article"). Omit to generate all.
        singular: Option<String>,
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PluginAction {
    /// Create a new plugin from template
    New {
        /// Plugin ID (e.g. "my-plugin")
        id: String,
        /// Runtime: js, lua, or wasm
        #[arg(short, long, default_value = "js")]
        runtime: String,
    },
    /// Validate plugin manifests
    Check {
        /// Path to check (default: plugin_dir)
        path: Option<String>,
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

        Some(Commands::Db {
            action: DbAction::Seed { email, username, password },
        }) => {
            db_cmd::seed(config, &email, &username, &password).await?;
        }

        Some(Commands::Ct {
            action: CtAction::New { name },
        }) => {
            ct_cmd::create_new(config, &name)?;
        }

        Some(Commands::Ct {
            action: CtAction::Check { path },
        }) => {
            ct_cmd::check(config, path.as_deref())?;
        }

        Some(Commands::Ct {
            action: CtAction::Types { singular, output },
        }) => {
            ct_cmd::generate_types(config, singular.as_deref(), output.as_deref())?;
        }

        Some(Commands::Plugin {
            action: PluginAction::New { id, runtime },
        }) => {
            plugin_cmd::create_new(config, &id, &runtime)?;
        }

        Some(Commands::Plugin {
            action: PluginAction::Check { path },
        }) => {
            plugin_cmd::check(config, path.as_deref())?;
        }
    }

    Ok(())
}
