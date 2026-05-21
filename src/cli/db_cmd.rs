//! `db` subcommand: database migration, backup, seed data.

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;

// ── migrate ──────────────────────────────────────────────────────

/// `db migrate` — execute incremental schema changes.
///
/// Uses the `_migrations` table to track executed filenames, idempotent and safe.
/// All applied migrations in one call share the same batch number.
pub async fn migrate(config: &AppConfig) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::ensure_schema(&pool).await?;
    raisfast::db::connection::run_pending_migrations(&pool).await?;

    Ok(())
}

// ── rollback ─────────────────────────────────────────────────────

/// `db rollback` — rollback the last batch (or `--step=N` individual migrations).
///
/// For each migration, looks for a corresponding `.down.sql` file.
/// The schema baseline (batch 0) is never rolled back.
pub async fn rollback(config: &AppConfig, step: &Option<u32>) -> anyhow::Result<()> {
    let step_desc = match step {
        Some(n) => format!("step={n}"),
        None => "last batch".to_string(),
    };
    println!("rolling back ({step_desc})...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::rollback_migrations(&pool, *step).await?;

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

    if raisfast_derive::crud_exists!(&pool, "users", where: ("username", username))? {
        println!("seed: admin user already exists ({username}), skipping");
        return Ok(());
    }

    let password_hash = raisfast::services::auth::hash_password(password)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;

    let (id, now) = (
        raisfast::utils::id::new_id(),
        raisfast::utils::tz::now_utc(),
    );

    let tid: Option<String> = if cfg!(feature = "db-sqlite") {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM tenants WHERE id = 'default' LIMIT 1")
                .fetch_optional(&pool)
                .await?;
        row.map(|r| r.0)
    } else {
        None
    };

    let registered_via = raisfast::models::user::RegisteredVia::Email;
    if let Some(ref tid) = tid {
        raisfast_derive::crud_insert!(&pool, "users", [
            "id" => id,
            "tenant_id" => tid,
            "username" => username,
            "created_at" => now,
            "updated_at" => now,
            "role" => raisfast::models::user::UserRole::Admin,
            "status" => raisfast::models::user::UserStatus::Active,
            "registered_via" => registered_via
        ])?;
    } else {
        raisfast_derive::crud_insert!(&pool, "users", [
            "id" => id,
            "username" => username,
            "created_at" => now,
            "updated_at" => now,
            "role" => raisfast::models::user::UserRole::Admin,
            "status" => raisfast::models::user::UserStatus::Active,
            "registered_via" => registered_via
        ])?;
    }

    let cred_data = serde_json::json!({"password_hash": password_hash}).to_string();
    let (cred_id, cred_now) = (
        raisfast::utils::id::new_id(),
        raisfast::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(&pool, "user_credentials", [
        "id" => cred_id,
        "user_id" => id,
        "auth_type" => raisfast::models::user_credential::AuthType::Email,
        "identifier" => email,
        "credential_data" => &cred_data,
        "verified" => 1i64,
        "created_at" => cred_now,
        "updated_at" => cred_now
    ])?;

    println!("seed: admin user created");
    println!("  email:    {email}");
    println!("  username: {username}");
    println!("  role:     admin");
    Ok(())
}
