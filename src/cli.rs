//! CLI definition and subcommand dispatch.
//!
//! Uses clap derive to define the command-line structure, dispatching each subcommand to its module.

mod app_cmd;
mod codegen_cmd;
mod ct_cmd;
mod db_cmd;
mod doctor_cmd;
mod plugin_cmd;
mod route_cmd;
mod server_cmd;
mod user_cmd;

use raisfast::config::app::AppConfig;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "raisfast",
    version,
    about = "Rust-powered high-performance BaaS and headless CMS"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project from template
    App {
        #[command(subcommand)]
        action: AppAction,
    },
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
    /// Route inspection
    Route {
        #[command(subcommand)]
        action: route_cmd::RouteAction,
    },
    /// System diagnostics
    Doctor,
    /// User account management
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Code generation
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
    /// Proxy management (multi-tenant reverse proxy)
    #[cfg(all(feature = "proxy", unix))]
    Proxy {
        #[command(subcommand)]
        action: ProxyAction,
    },
}

#[cfg(all(feature = "proxy", unix))]
#[derive(Subcommand)]
pub enum ProxyAction {
    /// Start the proxy server
    Start {
        /// Path to proxy config file
        #[arg(short, long, default_value = "/etc/raisfast/proxy.toml")]
        config: String,
    },
    /// Validate proxy configuration
    Check {
        /// Path to proxy config file
        #[arg(short, long, default_value = "/etc/raisfast/proxy.toml")]
        config: String,
    },
}

#[derive(Subcommand)]
pub enum AppAction {
    /// Create a new project directory with template files
    New {
        /// Project name (used as directory name)
        name: String,
        /// Template: blank, blog, ecommerce
        #[arg(short, long, default_value = "blank")]
        template: String,
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
    /// Rollback the last batch of migrations
    Rollback {
        /// Number of individual migrations to rollback (omit to rollback entire last batch)
        #[arg(short, long)]
        step: Option<u32>,
    },
    /// Backup the database to a timestamped file
    Backup {
        /// Output directory (default: {STORAGE_ROOT_DIR}/backups)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum UserAction {
    /// Create a new user account
    Create {
        /// Email address
        #[arg(long)]
        email: String,
        /// Username
        #[arg(long)]
        username: String,
        /// Password
        #[arg(long)]
        password: String,
        /// Role: admin, editor, author, reader (default: reader)
        #[arg(short, long, default_value = "reader")]
        role: String,
    },
    /// List all user accounts
    List,
    /// Change a user's password
    Passwd {
        /// Username
        username: String,
        /// New password
        #[arg(short, long)]
        password: String,
    },
    /// Delete a user account
    Delete {
        /// Username
        username: String,
        /// Force deletion (required for admin users)
        #[arg(long)]
        force: bool,
    },
    /// Disable (suspend) a user account
    Disable {
        /// Username
        username: String,
    },
    /// Enable (reactivate) a user account
    Enable {
        /// Username
        username: String,
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

#[derive(Subcommand)]
pub enum CodegenAction {
    /// Generate model scaffold from schema.sql
    Model {
        /// Table names to generate (omit to generate all)
        tables: Vec<String>,
        /// Overwrite existing files
        #[arg(long)]
        force: bool,
        /// Print generated code without writing files
        #[arg(long)]
        dry_run: bool,
    },
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

fn gradient_rgb(t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0).sqrt();
    let (r1, g1, b1) = (37, 99, 235);
    let (r2, g2, b2) = (204, 43, 28);
    (lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t))
}

fn max_chars(lines: &[&str]) -> usize {
    lines.iter().map(|l| l.chars().count()).max().unwrap_or(0)
}

fn print_gradient_line(line: &str, total_w: usize, offset: usize) {
    let reset = "\x1b[0m";
    let mut buf = String::new();
    let mut prev_rgb: Option<(u8, u8, u8)> = None;
    for (i, ch) in line.chars().enumerate() {
        if ch == ' ' {
            buf.push(ch);
            prev_rgb = None;
            continue;
        }
        let t = if total_w > 0 {
            (offset + i) as f32 / total_w as f32
        } else {
            0.0
        };
        let rgb = gradient_rgb(t);
        if prev_rgb != Some(rgb) {
            buf.push_str(&format!("\x1b[38;2;{};{};{}m", rgb.0, rgb.1, rgb.2));
            prev_rgb = Some(rgb);
        }
        buf.push(ch);
    }
    buf.push_str(reset);
    println!("{buf}");
}

fn terminal_width() -> usize {
    let output = std::process::Command::new("stty")
        .arg("size")
        .stdin(std::process::Stdio::inherit())
        .output()
        .ok();
    output
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut parts = s.split_whitespace();
            parts.next();
            parts.next().and_then(|w| w.parse::<usize>().ok())
        })
        .unwrap_or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(80)
        })
}

// https://patorjk.com/software/taag/#p=display&f=Electronic&t=Rais
const RAIS: &[&str] = &[
    "     ▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄",
    "    ▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌",
    "    ▐░█▀▀▀▀▀▀▀█░▌▐░█▀▀▀▀▀▀▀█░▌ ▀▀▀▀█░█▀▀▀▀ ▐░█▀▀▀▀▀▀▀▀▀",
    "    ▐░▌       ▐░▌▐░▌       ▐░▌     ▐░▌     ▐░▌",
    "    ▐░█▄▄▄▄▄▄▄█░▌▐░█▄▄▄▄▄▄▄█░▌     ▐░▌     ▐░█▄▄▄▄▄▄▄▄▄",
    "    ▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌     ▐░▌     ▐░░░░░░░░░░░▌",
    "    ▐░█▀▀▀▀█░█▀▀ ▐░█▀▀▀▀▀▀▀█░▌     ▐░▌      ▀▀▀▀▀▀▀▀▀█░▌",
    "    ▐░▌     ▐░▌  ▐░▌       ▐░▌     ▐░▌               ▐░▌",
    "    ▐░▌      ▐░▌ ▐░▌       ▐░▌ ▄▄▄▄█░█▄▄▄▄  ▄▄▄▄▄▄▄▄▄█░▌",
    "    ▐░▌       ▐░▌▐░▌       ▐░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌",
    "     ▀         ▀  ▀         ▀  ▀▀▀▀▀▀▀▀▀▀▀  ▀▀▀▀▀▀▀▀▀▀▀",
];

// https://patorjk.com/software/taag/#p=display&f=Electronic&t=Fast
const FAST: &[&str] = &[
    "▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄  ▄▄▄▄▄▄▄▄▄▄▄",
    "▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌",
    "▐░█▀▀▀▀▀▀▀▀▀ ▐░█▀▀▀▀▀▀▀█░▌▐░█▀▀▀▀▀▀▀▀▀  ▀▀▀▀█░█▀▀▀▀",
    "▐░▌          ▐░▌       ▐░▌▐░▌               ▐░▌",
    "▐░█▄▄▄▄▄▄▄▄▄ ▐░█▄▄▄▄▄▄▄█░▌▐░█▄▄▄▄▄▄▄▄▄      ▐░▌",
    "▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌▐░░░░░░░░░░░▌     ▐░▌",
    "▐░█▀▀▀▀▀▀▀▀▀ ▐░█▀▀▀▀▀▀▀█░▌ ▀▀▀▀▀▀▀▀▀█░▌     ▐░▌",
    "▐░▌          ▐░▌       ▐░▌          ▐░▌     ▐░▌",
    "▐░▌          ▐░▌       ▐░▌ ▄▄▄▄▄▄▄▄▄█░▌     ▐░▌",
    "▐░▌          ▐░▌       ▐░▌▐░░░░░░░░░░░▌     ▐░▌",
    " ▀            ▀         ▀  ▀▀▀▀▀▀▀▀▀▀▀       ▀",
];

pub fn print_banner(config: &AppConfig) {
    let d = "\x1b[2;37m";
    let r = "\x1b[0m";

    let tw = terminal_width();
    let rais_w = max_chars(RAIS);
    let fast_w = max_chars(FAST);
    let gap = 3;
    let side_by_side_w = rais_w + gap + fast_w;

    println!();

    if tw >= side_by_side_w {
        for i in 0..RAIS.len() {
            let combined = format!(
                "{:<rais_w$}{}{}",
                RAIS[i].trim_end(),
                " ".repeat(gap),
                FAST[i],
                rais_w = rais_w,
            );
            print_gradient_line(&combined, side_by_side_w, 0);
        }
    } else {
        let rais_pad = (tw as isize - rais_w as isize).max(0) as usize / 2;
        let fast_pad = (tw as isize - fast_w as isize).max(0) as usize / 2;
        let fast_offset = rais_w + gap;
        for line in RAIS {
            print!("{}", " ".repeat(rais_pad));
            print_gradient_line(line, side_by_side_w, 0);
        }
        for line in FAST {
            print!("{}", " ".repeat(fast_pad));
            print_gradient_line(line, side_by_side_w, fast_offset);
        }
    }

    println!();
    println!(
        "{d}    raisfast v{}  ·  http://{}:{}{r}",
        env!("CARGO_PKG_VERSION"),
        config.host,
        config.port
    );
    println!();
}

pub async fn run(cli: Cli, config: &AppConfig) -> anyhow::Result<()> {
    match cli.command {
        Some(Commands::App {
            action: AppAction::New { name, template },
        }) => {
            app_cmd::create_new(&name, &template)?;
        }

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
            action: DbAction::Rollback { step },
        }) => {
            db_cmd::rollback(config, &step).await?;
        }

        Some(Commands::Db {
            action: DbAction::Backup { output },
        }) => {
            let out = output.unwrap_or_else(|| format!("{}/backups", config.storage_root_dir));
            db_cmd::backup(config, &out, config.backup_retention).await?;
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

        Some(Commands::Route { action }) => {
            route_cmd::run(action, config);
        }

        Some(Commands::Doctor) => {
            doctor_cmd::run(config).await;
        }

        Some(Commands::User {
            action:
                UserAction::Create {
                    email,
                    username,
                    password,
                    role,
                },
        }) => {
            user_cmd::create(config, &email, &username, &password, &role).await?;
        }

        Some(Commands::User {
            action: UserAction::List,
        }) => {
            user_cmd::list(config).await?;
        }

        Some(Commands::User {
            action: UserAction::Passwd { username, password },
        }) => {
            user_cmd::passwd(config, &username, &password).await?;
        }

        Some(Commands::User {
            action: UserAction::Delete { username, force },
        }) => {
            user_cmd::delete(config, &username, force).await?;
        }

        Some(Commands::User {
            action: UserAction::Disable { username },
        }) => {
            user_cmd::disable(config, &username).await?;
        }

        Some(Commands::User {
            action: UserAction::Enable { username },
        }) => {
            user_cmd::enable(config, &username).await?;
        }

        Some(Commands::Codegen {
            action:
                CodegenAction::Model {
                    tables,
                    force,
                    dry_run,
                },
        }) => {
            codegen_cmd::run_model(&tables, force, dry_run)?;
        }

        #[cfg(all(feature = "proxy", unix))]
        Some(Commands::Proxy {
            action: ProxyAction::Start {
                config: proxy_config,
            },
        }) => {
            raisfast::proxy::start(&proxy_config).await?;
        }

        #[cfg(all(feature = "proxy", unix))]
        Some(Commands::Proxy {
            action: ProxyAction::Check {
                config: proxy_config,
            },
        }) => {
            match raisfast::proxy::config::ProxyConfig::load(std::path::Path::new(&proxy_config)) {
                Ok(c) => {
                    println!("proxy config OK");
                    println!("  listen_http: {}", c.proxy.listen_http);
                    println!("  listen_https: {}", c.proxy.listen_https);
                    println!("  tenants_dir: {}", c.proxy.tenants_dir.display());
                    let tenants = raisfast::proxy::config::load_all_tenants(&c.proxy.tenants_dir);
                    println!("  tenants loaded: {}", tenants.len());
                    for (_, t) in &tenants {
                        println!(
                            "    - {} (host={:?}, prefix={:?}, backend={})",
                            t.tenant.name, t.tenant.host, t.tenant.prefix, t.tenant.backend
                        );
                    }
                }
                Err(e) => {
                    eprintln!("proxy config error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
