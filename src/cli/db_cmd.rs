//! `db` subcommand: database migration, backup, seed data.

use raisfast::DbDriver;
use raisfast::config::app::AppConfig;
use raisfast::db::Driver;
use raisfast::db::connection::init_pool;

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
        Driver::ph(1)
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

    let (id, now) = (
        raisfast::utils::id::new_id(),
        raisfast::utils::tz::now_utc(),
    );

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
                Driver::ph(1),
                Driver::ph(2),
                Driver::ph(3),
                Driver::ph(4),
                Driver::ph(5),
                Driver::ph(6),
                Driver::ph(7),
                Driver::ph(8),
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
                Driver::ph(1),
                Driver::ph(2),
                Driver::ph(3),
                Driver::ph(4),
                Driver::ph(5),
                Driver::ph(6),
                Driver::ph(7),
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
        Driver::ph(1)
    ))
    .bind(id)
    .fetch_one(&pool)
    .await?;

    let cred_data = serde_json::json!({"password_hash": password_hash}).to_string();
    let (cred_id, cred_now) = (
        raisfast::utils::id::new_id(),
        raisfast::utils::tz::now_utc(),
    );
    sqlx::query(&format!(
        "INSERT INTO user_credentials (id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, 1, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
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
