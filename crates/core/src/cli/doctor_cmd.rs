//! `raisfast doctor` CLI subcommand.
//!
//! Diagnose system health without starting the HTTP server.
//! Checks configuration, database, storage directories, search index,
//! security settings, and provides a summary report.

use std::path::Path;

use raisfast::config::app::AppConfig;
use raisfast::db::{DbDriver, Driver};

// ── Result types ─────────────────────────────────────────────

#[derive(Clone)]
enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

struct CheckResult {
    name: String,
    status: CheckStatus,
    message: String,
}

// ── ANSI helpers ─────────────────────────────────────────────

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";

fn status_icon(status: &CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "\u{2705}",
        CheckStatus::Warn => "\u{26A0}\u{FE0F}",
        CheckStatus::Fail => "\u{274C}",
    }
}

fn status_color(status: &CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => GREEN,
        CheckStatus::Warn => YELLOW,
        CheckStatus::Fail => RED,
    }
}

// ── Checks ───────────────────────────────────────────────────

fn check_env_files() -> Vec<CheckResult> {
    let mut results = Vec::new();

    let env_base = Path::new(".env");
    if env_base.exists() {
        results.push(CheckResult {
            name: ".env file".into(),
            status: CheckStatus::Ok,
            message: "found".into(),
        });
    } else {
        results.push(CheckResult {
            name: ".env file".into(),
            status: CheckStatus::Warn,
            message: "not found (using defaults)".into(),
        });
    }

    let env_profile = format!(
        ".env.{}",
        std::env::var("APP_ENV").unwrap_or_else(|_| "development".into())
    );
    if Path::new(&env_profile).exists() {
        results.push(CheckResult {
            name: format!("{env_profile} file"),
            status: CheckStatus::Ok,
            message: "found".into(),
        });
    } else {
        results.push(CheckResult {
            name: format!("{env_profile} file"),
            status: CheckStatus::Warn,
            message: "not found".into(),
        });
    }

    results
}

fn check_config(config: &AppConfig) -> Vec<CheckResult> {
    let mut results = Vec::new();

    results.push(CheckResult {
        name: "APP_ENV".into(),
        status: CheckStatus::Ok,
        message: config.env.clone(),
    });

    let db_display = if config.database_url.is_empty() {
        "not set".into()
    } else if config.database_url.len() > 60 {
        format!("{}...", &config.database_url[..57])
    } else {
        config.database_url.clone()
    };

    results.push(CheckResult {
        name: "DATABASE_URL".into(),
        status: if config.database_url.is_empty() {
            CheckStatus::Fail
        } else {
            CheckStatus::Ok
        },
        message: db_display,
    });

    results.push(CheckResult {
        name: "HOST:PORT".into(),
        status: CheckStatus::Ok,
        message: format!("{}:{}", config.host, config.port),
    });

    results.push(CheckResult {
        name: "BASE_URL".into(),
        status: CheckStatus::Ok,
        message: config.base_url.clone(),
    });

    results
}

fn check_security(config: &AppConfig) -> Vec<CheckResult> {
    let mut results = Vec::new();

    const DEFAULT_JWT_SECRET: &str = "change-me-in-production-at-least-32-chars";
    let is_default_jwt = config.jwt_secret == DEFAULT_JWT_SECRET;

    let jwt_status = if config.env == "production" && is_default_jwt {
        CheckStatus::Fail
    } else if is_default_jwt {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };

    results.push(CheckResult {
        name: "JWT_SECRET".into(),
        status: jwt_status,
        message: if is_default_jwt {
            "using default secret".into()
        } else {
            "custom secret set".into()
        },
    });

    let cors_status = match &config.cors_origins {
        None if config.env == "production" => CheckStatus::Fail,
        None => CheckStatus::Warn,
        Some(_) => CheckStatus::Ok,
    };

    results.push(CheckResult {
        name: "CORS_ORIGINS".into(),
        status: cors_status,
        message: config
            .cors_origins
            .clone()
            .unwrap_or_else(|| "not set (wildcard)".into()),
    });

    let tls_status = match (&config.tls_cert_path, &config.tls_key_path) {
        (Some(_), Some(_)) => CheckStatus::Ok,
        (Some(_), None) | (None, Some(_)) => CheckStatus::Warn,
        (None, None) if config.env == "production" => CheckStatus::Warn,
        (None, None) => CheckStatus::Ok,
    };

    results.push(CheckResult {
        name: "TLS".into(),
        status: tls_status,
        message: match (&config.tls_cert_path, &config.tls_key_path) {
            (Some(_), Some(_)) => "configured".into(),
            (Some(_), None) => "cert set but key missing".into(),
            (None, Some(_)) => "key set but cert missing".into(),
            (None, None) => "not configured (HTTP)".into(),
        },
    });

    results
}

fn check_directories(config: &AppConfig) -> Vec<CheckResult> {
    let mut results = Vec::new();

    let dirs = [
        ("storage_root_dir", &config.storage_root_dir),
        ("upload_dir", &config.upload_dir),
        ("log_dir", &config.log_dir),
        ("search_index_dir", &config.search_index_dir),
        ("plugin_vfs_root", &config.plugin_vfs_root),
    ];

    for (label, dir) in &dirs {
        let path = Path::new(dir);
        let exists = path.exists();
        let is_dir = path.is_dir();

        if exists && is_dir {
            let writable = check_writable(path);
            results.push(CheckResult {
                name: label.to_string(),
                status: if writable {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Fail
                },
                message: if writable {
                    format!("{} (writable)", dir)
                } else {
                    format!("{} (NOT writable)", dir)
                },
            });
        } else {
            let can_create = std::fs::create_dir_all(path).is_ok() && check_writable(path);
            if can_create {
                let _ = std::fs::remove_dir(path);
                results.push(CheckResult {
                    name: label.to_string(),
                    status: CheckStatus::Warn,
                    message: format!("{} (auto-creatable)", dir),
                });
            } else {
                results.push(CheckResult {
                    name: label.to_string(),
                    status: CheckStatus::Fail,
                    message: format!("{} (cannot create)", dir),
                });
            }
        }
    }

    if let Some(plugin_dir) = &config.plugin_dir {
        let path = Path::new(plugin_dir);
        if path.exists() && path.is_dir() {
            results.push(CheckResult {
                name: "plugin_dir".into(),
                status: CheckStatus::Ok,
                message: plugin_dir.clone(),
            });
        } else {
            results.push(CheckResult {
                name: "plugin_dir".into(),
                status: CheckStatus::Warn,
                message: format!("{plugin_dir} (not found)"),
            });
        }
    }

    results
}

fn check_writable(path: &Path) -> bool {
    let test_file = path.join(".doctor_write_test");
    let can_write = std::fs::write(&test_file, b"ok").is_ok();
    let _ = std::fs::remove_file(&test_file);
    can_write
}

async fn check_database(config: &AppConfig) -> CheckResult {
    match raisfast::db::connection::init_pool(&config.database_url, 1).await {
        Ok(pool) => match sqlx::query("SELECT 1").execute(&pool).await {
            Ok(_) => {
                let table_check = check_core_tables(&pool).await;
                CheckResult {
                    name: "database".into(),
                    status: if table_check.is_empty() {
                        CheckStatus::Ok
                    } else {
                        CheckStatus::Warn
                    },
                    message: if table_check.is_empty() {
                        "connected, schema OK".into()
                    } else {
                        format!("connected, missing tables: {}", table_check.join(", "))
                    },
                }
            }
            Err(e) => CheckResult {
                name: "database".into(),
                status: CheckStatus::Fail,
                message: format!("connection failed: {e}"),
            },
        },
        Err(e) => CheckResult {
            name: "database".into(),
            status: CheckStatus::Fail,
            message: format!("pool init failed: {e}"),
        },
    }
}

async fn check_core_tables(pool: &raisfast::db::Pool) -> Vec<String> {
    let core_tables = [
        "users",
        "posts",
        "categories",
        "tags",
        "media",
        "comments",
        "options",
        "rbac_roles",
    ];

    let mut missing = Vec::new();
    for table in &core_tables {
        let exists = Driver::table_exists(pool, table).await;
        if !exists {
            missing.push(table.to_string());
        }
    }
    missing
}

fn check_builtins(config: &AppConfig) -> Vec<CheckResult> {
    let mut results = Vec::new();

    let modules = [
        ("blog", config.builtins.blog),
        ("pages", config.builtins.pages),
        ("media", config.builtins.media),
        ("fulltext", config.builtins.fulltext),
        ("workflow", config.builtins.workflow),
    ];

    for (name, enabled) in &modules {
        results.push(CheckResult {
            name: format!("builtin::{name}"),
            status: CheckStatus::Ok,
            message: if *enabled { "enabled" } else { "disabled" }.to_string(),
        });
    }

    results
}

fn check_search(config: &AppConfig) -> CheckResult {
    if config.search_engine == "noop" {
        return CheckResult {
            name: "search engine".into(),
            status: CheckStatus::Warn,
            message: "noop (search disabled)".into(),
        };
    }

    let index_path = Path::new(&config.search_index_dir);
    if index_path.exists() {
        CheckResult {
            name: "search engine".into(),
            status: CheckStatus::Ok,
            message: format!(
                "{} (index: {})",
                config.search_engine, config.search_index_dir
            ),
        }
    } else {
        CheckResult {
            name: "search engine".into(),
            status: CheckStatus::Warn,
            message: format!(
                "{} (index dir not found: {})",
                config.search_engine, config.search_index_dir
            ),
        }
    }
}

// ── Report printing ──────────────────────────────────────────

fn print_section(title: &str) {
    println!();
    println!("{BOLD}{CYAN}[{title}]{RESET}");
    println!("{DIM}{}{RESET}", "-".repeat(50));
}

fn print_result(result: &CheckResult) {
    let icon = status_icon(&result.status);
    let color = status_color(&result.status);
    println!(
        "  {icon} {color}{:<25}{RESET} {}",
        result.name, result.message,
    );
}

fn print_summary(results: &[CheckResult]) {
    let ok_count = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Ok))
        .count();
    let warn_count = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Warn))
        .count();
    let fail_count = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Fail))
        .count();

    println!();
    println!("{BOLD}{DIM}{}{RESET}", "=".repeat(50));
    println!(
        "{BOLD}Summary:{RESET}  {GREEN}{ok_count} passed{RESET}  {YELLOW}{warn_count} warnings{RESET}  {RED}{fail_count} failures{RESET}",
    );

    let overall = if fail_count > 0 {
        CheckStatus::Fail
    } else if warn_count > 0 {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };

    let icon = status_icon(&overall);
    let color = status_color(&overall);
    let label = match &overall {
        CheckStatus::Ok => "ALL CHECKS PASSED",
        CheckStatus::Warn => "PASSED WITH WARNINGS",
        CheckStatus::Fail => "SOME CHECKS FAILED",
    };
    println!("{BOLD}{color}{icon} {label}{RESET}");
    println!();
}

// ── Public entry point ───────────────────────────────────────

pub async fn run(config: &AppConfig) {
    let mut all_results: Vec<CheckResult> = Vec::new();

    println!("{BOLD}{CYAN}raisfast doctor{RESET} — system diagnostics");
    println!("{DIM}version {}{RESET}", env!("CARGO_PKG_VERSION"));

    // Section 1: Environment files
    print_section("Environment Files");
    let env_results = check_env_files();
    for r in &env_results {
        print_result(r);
    }
    all_results.extend(env_results);

    // Section 2: Configuration
    print_section("Configuration");
    let config_results = check_config(config);
    for r in &config_results {
        print_result(r);
    }
    all_results.extend(config_results);

    // Section 3: Security
    print_section("Security");
    let security_results = check_security(config);
    for r in &security_results {
        print_result(r);
    }
    all_results.extend(security_results);

    // Section 4: Storage directories
    print_section("Storage Directories");
    let dir_results = check_directories(config);
    for r in &dir_results {
        print_result(r);
    }
    all_results.extend(dir_results);

    // Section 5: Database
    print_section("Database");
    let db_result = check_database(config).await;
    print_result(&db_result);
    all_results.push(db_result);

    // Section 6: Search
    print_section("Search");
    let search_result = check_search(config);
    print_result(&search_result);
    all_results.push(search_result);

    // Section 7: Builtins
    print_section("Built-in Modules");
    let builtin_results = check_builtins(config);
    for r in &builtin_results {
        print_result(r);
    }
    all_results.extend(builtin_results);

    // Summary
    print_summary(&all_results);
}
