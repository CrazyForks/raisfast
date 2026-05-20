//! `db` subcommand: database migration, backup, seed data.

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;
use raisfast::db::dialect;

// ── migrate ──────────────────────────────────────────────────────

/// `db migrate` — execute incremental schema changes.
///
/// Uses the `_migrations` table to track executed filenames, idempotent and safe.
/// Now runs the same logic as the auto-migration on startup.
pub async fn migrate(config: &AppConfig) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::ensure_schema(&pool).await?;

    Ok(())
}

// ── backup ───────────────────────────────────────────────────────

/// `db backup` — backup the database.
///
/// Delegates to `raisfast::db::backup::backup_database` which flushes WAL and copies the file.
pub async fn backup(config: &AppConfig, output_dir: &str, retention: usize) -> anyhow::Result<()> {
    raisfast::db::backup::backup_database(config, output_dir, retention).await
}

// ── seed ────────────────────────────────────────────────────────

/// `db seed` — create the initial admin user.
///
/// Idempotent: skips if email or username already exists.
pub async fn seed(
    config: &AppConfig,
    email: &str,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let existing: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM users WHERE username = {}",
        dialect::ph(1)
    ))
    .bind(username)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    if existing > 0 {
        println!("seed: admin user already exists ({username}), skipping");
        return Ok(());
    }

    let password_hash = raisfast::services::auth::hash_password(password)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;

    let (id, now) = raisfast::utils::id::new_id_and_timestamp();

    let tid = if cfg!(feature = "db-sqlite") {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM tenants WHERE id = 'default' LIMIT 1")
                .fetch_optional(&pool)
                .await?;
        row.map(|r| r.0)
    } else {
        None
    };

    match tid {
        Some(tid) => {
            sqlx::query(&format!(
                "INSERT INTO users (id, tenant_id, username, created_at, updated_at, role, status, registered_via) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
                dialect::ph(1),
                dialect::ph(2),
                dialect::ph(3),
                dialect::ph(4),
                dialect::ph(5),
                dialect::ph(6),
                dialect::ph(7),
                dialect::ph(8),
            ))
            .bind(id)
            .bind(&tid)
            .bind(username)
            .bind(now)
            .bind(now)
            .bind(raisfast::models::user::UserRole::Admin)
            .bind(raisfast::models::user::UserStatus::Active)
            .bind(raisfast::models::user::RegisteredVia::Email)
            .execute(&pool)
            .await?;
        }
        None => {
            sqlx::query(&format!(
                "INSERT INTO users (id, username, created_at, updated_at, role, status, registered_via) VALUES ({}, {}, {}, {}, {}, {}, {})",
                dialect::ph(1),
                dialect::ph(2),
                dialect::ph(3),
                dialect::ph(4),
                dialect::ph(5),
                dialect::ph(6),
                dialect::ph(7),
            ))
            .bind(id)
            .bind(username)
            .bind(now)
            .bind(now)
            .bind(raisfast::models::user::UserRole::Admin)
            .bind(raisfast::models::user::UserStatus::Active)
            .bind(raisfast::models::user::RegisteredVia::Email)
            .execute(&pool)
            .await?;
        }
    }

    let (user_id,): (i64,) = sqlx::query_as(&format!(
        "SELECT id FROM users WHERE id = {}",
        dialect::ph(1)
    ))
    .bind(id)
    .fetch_one(&pool)
    .await?;

    let cred_data = serde_json::json!({"password_hash": password_hash}).to_string();
    let (cred_id, cred_now) = raisfast::utils::id::new_id_and_timestamp();
    sqlx::query(&format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, 1, {}, {})",
        dialect::ph(1),
        dialect::ph(2),
        dialect::ph(3),
        dialect::ph(4),
        dialect::ph(5),
        dialect::ph(6),
        dialect::ph(7),
    ))
    .bind(cred_id)
    .bind(user_id)
    .bind("email")
    .bind(email)
    .bind(&cred_data)
    .bind(cred_now)
    .bind(cred_now)
    .execute(&pool)
    .await?;

    println!("seed: admin user created");
    println!("  email:    {email}");
    println!("  username: {username}");
    println!("  role:     admin");
    Ok(())
}
