//! Setup wizard handlers for first-time initialization.
//!
//! Provides endpoints for:
//! - Checking system status (database, storage, admin user)
//! - Configuring database connection (test + save + self-restart)
//! - Creating the initial admin user
//!
//! All endpoints return 403 once an admin user exists, preventing replay attacks.

use axum::Json;
use axum::extract::State;

use crate::dto::{
    SetupDatabaseRequest, SetupDatabaseResponse, SetupInitRequest, SetupStatusResponse,
    TestDatabaseResponse,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    _config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let r = axum::Router::new()
        .route("/setup/status", axum::routing::get(setup_status))
        .route("/setup/database/test", axum::routing::post(test_database))
        .route("/setup/database", axum::routing::post(setup_database))
        .route("/setup/init", axum::routing::post(setup_init));

    registry.record("GET", "/api/v1/setup/status", "system public", "setup");
    registry.record(
        "POST",
        "/api/v1/setup/database/test",
        "system public",
        "setup",
    );
    registry.record("POST", "/api/v1/setup/database", "system public", "setup");
    registry.record("POST", "/api/v1/setup/init", "system public", "setup");

    r
}

async fn ensure_no_admin(pool: &crate::db::Pool) -> AppResult<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(pool)
        .await
        .map_err(|_| AppError::BadRequest("database_not_connected".into()))?;
    if count > 0 {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub async fn setup_status(
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<SetupStatusResponse>> {
    let db_connected = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    let db_type = detect_db_type(&state.config.database_url);
    let url_masked = mask_db_url(&state.config.database_url);
    let (db_host, db_port, db_username, db_password, db_database) =
        parse_db_url_fields(&state.config.database_url);

    let storage_path = state.config.storage_root_dir.clone();
    let storage_writable = is_storage_writable(&storage_path);

    let ct_path = state.config.content_type_dir.clone();
    let plugin_path = state
        .config
        .plugin_dir
        .clone()
        .unwrap_or_else(|| "./extensions/plugins".into());
    let ext_writable = is_dir_writable(&ct_path) && is_dir_writable(&plugin_path);

    let admin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    Ok(ApiResponse::success(SetupStatusResponse {
        database: crate::dto::DatabaseStatusInfo {
            db_type,
            connected: db_connected,
            url_masked,
            host: db_host,
            port: db_port,
            username: db_username,
            password: db_password,
            database: db_database,
        },
        storage: crate::dto::StorageStatusInfo {
            writable: storage_writable,
            path: storage_path,
        },
        extensions: crate::dto::ExtensionsStatusInfo {
            writable: ext_writable,
            content_types_path: ct_path,
            plugins_path: plugin_path,
        },
        has_admin: admin_count > 0,
    }))
}

pub async fn test_database(
    State(state): State<crate::AppState>,
    Json(req): Json<SetupDatabaseRequest>,
) -> AppResult<ApiResponse<TestDatabaseResponse>> {
    ensure_no_admin(&state.pool).await?;

    let db_type = detect_db_type(&state.config.database_url);
    let url = req.build_url(&db_type).map_err(AppError::BadRequest)?;
    let url_masked = mask_db_url(&url);

    match test_db_connection(&url).await {
        Ok(()) => Ok(ApiResponse::success(TestDatabaseResponse {
            connected: true,
            db_type,
            url_masked,
            message: None,
        })),
        Err(e) => Ok(ApiResponse::success(TestDatabaseResponse {
            connected: false,
            db_type,
            url_masked,
            message: Some(e.to_string()),
        })),
    }
}

async fn test_db_connection(url: &str) -> AppResult<()> {
    let pool = crate::db::connection::init_pool(url, 1)
        .await
        .map_err(|e| AppError::BadRequest(format!("connection_failed: {e}")))?;

    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| AppError::BadRequest(format!("query_failed: {e}")))?;

    drop(pool);
    Ok(())
}

pub async fn setup_database(
    State(state): State<crate::AppState>,
    Json(req): Json<SetupDatabaseRequest>,
) -> AppResult<ApiResponse<SetupDatabaseResponse>> {
    ensure_no_admin(&state.pool).await?;

    let db_type = detect_db_type(&state.config.database_url);
    let url = req.build_url(&db_type).map_err(AppError::BadRequest)?;

    test_db_connection(&url).await?;

    persist_env_var("DATABASE_URL", &url);

    tracing::info!(
        old_url = mask_db_url(&state.config.database_url),
        new_url = mask_db_url(&url),
        "DATABASE_URL updated, scheduling self-restart"
    );

    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("raisfast"));
        let args: Vec<String> = std::env::args().skip(1).collect();
        tracing::info!("spawning new server process and exiting...");
        let _ = std::process::Command::new(&exe).args(&args).spawn();
        std::process::exit(0);
    });

    Ok(ApiResponse::success(SetupDatabaseResponse {
        restarting: true,
    }))
}

pub async fn setup_init(
    State(state): State<crate::AppState>,
    Json(req): Json<SetupInitRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
    ensure_no_admin(&state.pool).await?;
    validation::validate(&req)?;

    crate::services::auth::validate_password_strength(&req.password)?;

    let user = crate::services::auth::register(
        &state.aspect_engine,
        crate::dto::RegisterRequest {
            email: req.email,
            username: req.username,
            password: req.password,
        },
        None,
        false,
        &state.pool,
    )
    .await?;

    let uid = parse_user_id(&user.id)?;

    crate::models::user::update_role(&state.pool, uid, crate::models::user::UserRole::Admin, None)
        .await?;

    let access_token = crate::services::auth::generate_access_token_internal(
        uid,
        crate::models::user::UserRole::Admin,
        crate::constants::DEFAULT_TENANT,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
    )?;
    let refresh_token_str = crate::services::auth::generate_refresh_token_string_internal()?;
    let expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(state.config.jwt_refresh_expires as i64);

    crate::models::refresh_token::create_token(
        &state.pool,
        uid,
        &refresh_token_str,
        &expires_at.to_rfc3339(),
    )
    .await?;

    let admin_user = crate::models::user::find_by_id(&state.pool, uid, None)
        .await?
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("admin user not found after creation"))
        })?;

    Ok(ApiResponse::success(crate::dto::LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: state.config.jwt_access_expires,
        user: crate::dto::UserResponse::from_user_with_contacts(&state.pool, admin_user).await?,
    }))
}

fn detect_db_type(url: &str) -> String {
    if url.starts_with("sqlite:") {
        "sqlite".into()
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        "postgres".into()
    } else if url.starts_with("mysql://") {
        "mysql".into()
    } else {
        "unknown".into()
    }
}

#[allow(clippy::type_complexity)]
fn parse_db_url_fields(
    url: &str,
) -> (
    Option<String>,
    Option<u16>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    if url.starts_with("sqlite:") {
        let path = url.strip_prefix("sqlite:").unwrap_or(url);
        let path = path
            .trim_start_matches("./")
            .split('?')
            .next()
            .unwrap_or(path);
        return (None, None, None, None, Some(path.to_string()));
    }
    let Some(scheme_end) = url.find("://") else {
        return (None, None, None, None, None);
    };
    let rest = &url[scheme_end + 3..];
    let (user_part, after_at) = if let Some(at_pos) = rest.find('@') {
        (&rest[..at_pos], &rest[at_pos + 1..])
    } else {
        return (None, None, None, None, None);
    };
    let (username, password) = if let Some(colon_pos) = user_part.find(':') {
        let u = &user_part[..colon_pos];
        let p = &user_part[colon_pos + 1..];
        (
            if u.is_empty() {
                None
            } else {
                Some(u.to_string())
            },
            if p.is_empty() {
                None
            } else {
                Some(p.to_string())
            },
        )
    } else if user_part.is_empty() {
        (None, None)
    } else {
        (Some(user_part.to_string()), None)
    };
    let host_port_db = after_at;
    let slash_pos = host_port_db.find('/').unwrap_or(host_port_db.len());
    let host_port = &host_port_db[..slash_pos];
    let database = if slash_pos < host_port_db.len() {
        let db = &host_port_db[slash_pos + 1..];
        let db = db.split('?').next().unwrap_or(db);
        if db.is_empty() {
            None
        } else {
            Some(db.to_string())
        }
    } else {
        None
    };
    let (host, port) = if let Some(colon_pos) = host_port.rfind(':') {
        let h = &host_port[..colon_pos];
        let p = &host_port[colon_pos + 1..];
        (Some(h.to_string()), p.parse().ok())
    } else {
        (Some(host_port.to_string()), None)
    };
    (host, port, username, password, database)
}

fn mask_db_url(url: &str) -> String {
    if url.starts_with("sqlite:") {
        return url.to_string();
    }
    let Some(at_pos) = url.find('@') else {
        return "***".into();
    };
    let Some(scheme_end) = url.find("://") else {
        return "***".into();
    };
    let scheme = &url[..scheme_end + 3];
    let rest = &url[scheme_end + 3..at_pos];
    let after_at = &url[at_pos..];
    let user_part = if let Some(colon_pos) = rest.find(':') {
        &rest[..colon_pos]
    } else {
        rest
    };
    format!("{scheme}{user_part}:***{after_at}")
}

fn is_storage_writable(path: &str) -> bool {
    let p = std::path::Path::new(path);
    if std::fs::create_dir_all(p).is_err() {
        return false;
    }
    let test_file = p.join(".setup_check");
    let can_write = std::fs::write(&test_file, b"ok").is_ok();
    let _ = std::fs::remove_file(&test_file);
    can_write
}

fn is_dir_writable(path: &str) -> bool {
    let p = std::path::Path::new(path);
    if std::fs::create_dir_all(p).is_err() {
        return false;
    }
    let test_file = p.join(".setup_check");
    let can_write = std::fs::write(&test_file, b"ok").is_ok();
    let _ = std::fs::remove_file(&test_file);
    can_write
}

fn persist_env_var(key: &str, value: &str) {
    let env_path = std::path::Path::new(".env");
    let line = format!("{key}={value}");
    if env_path.exists() {
        if let Ok(content) = std::fs::read_to_string(env_path) {
            let prefix = format!("{key}=");
            let updated = if content.lines().any(|l| l.starts_with(&prefix)) {
                content
                    .lines()
                    .map(|l| if l.starts_with(&prefix) { &line } else { l })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                format!("{content}\n{line}")
            };
            let _ = std::fs::write(env_path, updated);
        }
    } else {
        let _ = std::fs::write(env_path, format!("{line}\n"));
    }
}

fn parse_user_id(id: &str) -> AppResult<crate::types::snowflake_id::SnowflakeId> {
    id.parse::<i64>()
        .map(crate::types::snowflake_id::SnowflakeId)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid user id format")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_db_type_sqlite() {
        assert_eq!(detect_db_type("sqlite:./test.db"), "sqlite");
        assert_eq!(detect_db_type("sqlite:./test.db?mode=rwc"), "sqlite");
    }

    #[test]
    fn detect_db_type_postgres() {
        assert_eq!(detect_db_type("postgres://user:pass@host/db"), "postgres");
        assert_eq!(detect_db_type("postgresql://user:pass@host/db"), "postgres");
    }

    #[test]
    fn detect_db_type_mysql() {
        assert_eq!(detect_db_type("mysql://root@localhost/db"), "mysql");
    }

    #[test]
    fn detect_db_type_unknown() {
        assert_eq!(detect_db_type("something://host"), "unknown");
    }

    #[test]
    fn mask_db_url_sqlite() {
        assert_eq!(
            mask_db_url("sqlite:./storage/db/raisfast.db?mode=rwc"),
            "sqlite:./storage/db/raisfast.db?mode=rwc"
        );
    }

    #[test]
    fn mask_db_url_postgres() {
        assert_eq!(
            mask_db_url("postgres://admin:secret123@db.example.com:5432/mydb"),
            "postgres://admin:***@db.example.com:5432/mydb"
        );
    }

    #[test]
    fn mask_db_url_mysql() {
        assert_eq!(
            mask_db_url("mysql://root:password@localhost/raisfast"),
            "mysql://root:***@localhost/raisfast"
        );
    }

    #[test]
    fn mask_db_url_no_password() {
        assert_eq!(
            mask_db_url("postgres://admin@localhost/db"),
            "postgres://admin:***@localhost/db"
        );
    }

    #[test]
    fn parse_user_id_valid() {
        let result = parse_user_id("12345");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, 12345);
    }

    #[test]
    fn parse_user_id_invalid() {
        assert!(parse_user_id("not-a-number").is_err());
    }

    #[test]
    fn build_url_sqlite_from_database() {
        let req = crate::dto::SetupDatabaseRequest {
            host: None,
            port: None,
            username: None,
            password: None,
            database: Some("./data/test.db".into()),
            url: None,
        };
        assert_eq!(
            req.build_url("sqlite").unwrap(),
            "sqlite:./data/test.db?mode=rwc"
        );
    }

    #[test]
    fn build_url_sqlite_default() {
        let req = crate::dto::SetupDatabaseRequest {
            host: None,
            port: None,
            username: None,
            password: None,
            database: None,
            url: None,
        };
        assert_eq!(
            req.build_url("sqlite").unwrap(),
            "sqlite:./storage/db/raisfast.db?mode=rwc"
        );
    }

    #[test]
    fn build_url_postgres_with_password() {
        let req = crate::dto::SetupDatabaseRequest {
            host: Some("db.example.com".into()),
            port: Some(5432),
            username: Some("admin".into()),
            password: Some("secret".into()),
            database: Some("mydb".into()),
            url: None,
        };
        assert_eq!(
            req.build_url("postgres").unwrap(),
            "postgres://admin:secret@db.example.com:5432/mydb"
        );
    }

    #[test]
    fn build_url_postgres_default_port() {
        let req = crate::dto::SetupDatabaseRequest {
            host: Some("localhost".into()),
            port: None,
            username: Some("user".into()),
            password: None,
            database: Some("testdb".into()),
            url: None,
        };
        assert_eq!(
            req.build_url("postgres").unwrap(),
            "postgres://user@localhost:5432/testdb"
        );
    }

    #[test]
    fn build_url_mysql_with_password() {
        let req = crate::dto::SetupDatabaseRequest {
            host: Some("localhost".into()),
            port: Some(3306),
            username: Some("root".into()),
            password: Some("pass123".into()),
            database: Some("raisfast".into()),
            url: None,
        };
        assert_eq!(
            req.build_url("mysql").unwrap(),
            "mysql://root:pass123@localhost:3306/raisfast"
        );
    }

    #[test]
    fn build_url_raw_url_passthrough() {
        let req = crate::dto::SetupDatabaseRequest {
            host: None,
            port: None,
            username: None,
            password: None,
            database: None,
            url: Some("postgres://user:pass@host/db?sslmode=require".into()),
        };
        assert_eq!(
            req.build_url("postgres").unwrap(),
            "postgres://user:pass@host/db?sslmode=require"
        );
    }

    #[test]
    fn build_url_postgres_missing_host() {
        let req = crate::dto::SetupDatabaseRequest {
            host: None,
            port: None,
            username: Some("user".into()),
            password: None,
            database: Some("db".into()),
            url: None,
        };
        assert!(req.build_url("postgres").is_err());
    }
}
