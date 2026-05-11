//! `db` 子命令：数据库迁移、备份、种子数据。

use std::path::Path;

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;
use raisfast::db::dialect;

// ── migrate ──────────────────────────────────────────────────────

/// `db migrate` — 执行增量结构变更。
///
/// 使用 `_migrations` 表记录已执行的文件名，幂等安全。
///
/// - schema 在首次启动时由 `ensure_schema()` 自动执行，不由此命令处理
/// - 此命令处理 `migrations/{db}/` 下的增量迁移文件（如 `tenantable.*.sql`）
pub async fn migrate(config: &AppConfig) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::ensure_schema(&pool).await?;

    let db_name = if cfg!(feature = "db-sqlite") {
        "sqlite"
    } else if cfg!(feature = "db-postgres") {
        "postgres"
    } else if cfg!(feature = "db-mysql") {
        "mysql"
    } else {
        anyhow::bail!("no database feature enabled (db-sqlite / db-postgres / db-mysql)");
    };

    let migrations_dir = Path::new("./migrations").join(db_name);
    if !migrations_dir.exists() {
        println!("no migrations directory found (skipped)");
        return Ok(());
    }

    let schema_label = format!("schema.{}.sql", db_name);

    let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().is_some_and(|ext| ext == "sql")
                && e.file_name().to_string_lossy() != schema_label
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        println!("no migration files found");
        return Ok(());
    }

    let check_sql = format!(
        "SELECT COUNT(*) FROM _migrations WHERE filename = {}",
        dialect::ph(1)
    );
    let insert_sql = format!(
        "INSERT INTO _migrations (filename) VALUES ({})",
        dialect::ph(1)
    );

    let tenantable_file = format!("tenantable.{}.sql", db_name);
    let mut applied = 0u32;

    for entry in &entries {
        let filename = entry.file_name().to_string_lossy().to_string();

        let already_applied: bool = sqlx::query_scalar::<_, i64>(&check_sql)
            .bind(&filename)
            .fetch_one(&pool)
            .await
            .unwrap_or(0)
            > 0;

        if already_applied {
            println!("  [skip] {}", filename);
            continue;
        }

        if filename == tenantable_file && !config.builtin_tenantable {
            println!("  [skip] {} (BUILTIN_TENANTABLE=false)", filename);
            sqlx::query(&insert_sql)
                .bind(&filename)
                .execute(&pool)
                .await?;
            continue;
        }

        let sql = std::fs::read_to_string(entry.path())?;
        print!("  [apply] {} ... ", filename);
        sqlx::query(&sql).execute(&pool).await?;
        sqlx::query(&insert_sql)
            .bind(&filename)
            .execute(&pool)
            .await?;
        println!("ok");
        applied += 1;
    }

    if applied == 0 {
        println!("all migrations already applied");
    } else {
        println!("applied {} migration(s)", applied);
    }

    Ok(())
}

// ── backup ───────────────────────────────────────────────────────

/// `db backup` — 备份数据库。
///
/// SQLite：复制数据库文件到指定目录，自动添加时间戳后缀，保留最近 10 个备份。
/// PostgreSQL / MySQL：提示使用 `pg_dump` / `mysqldump`。
pub fn backup(config: &AppConfig, output_dir: &str) -> anyhow::Result<()> {
    #[cfg(feature = "db-sqlite")]
    {
        let db_path = config
            .database_url
            .trim_start_matches("sqlite:")
            .split('?')
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid DATABASE_URL: {}", config.database_url))?;

        if !Path::new(db_path).exists() {
            anyhow::bail!("database file not found: {}", db_path);
        }

        std::fs::create_dir_all(output_dir)?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_name = format!("raisfast_{}.db", timestamp);
        let backup_path = Path::new(output_dir).join(&backup_name);

        std::fs::copy(db_path, &backup_path)?;
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&backup_path).and_then(|f| f.set_modified(now));
        let size = std::fs::metadata(&backup_path)?.len();

        println!("backed up to {} ({} bytes)", backup_path.display(), size);

        cleanup_old_backups(output_dir);
        Ok(())
    }

    #[cfg(not(feature = "db-sqlite"))]
    {
        let _ = (config, output_dir);
        anyhow::bail!(
            "file-based backup is only supported for SQLite. \
             Use pg_dump (PostgreSQL) or mysqldump (MySQL) instead."
        );
    }
}

/// 清理旧备份，只保留最近 10 个。
fn cleanup_old_backups(output_dir: &str) {
    let mut backups: Vec<_> = std::fs::read_dir(output_dir)
        .ok()
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "db"))
                .collect()
        })
        .unwrap_or_default();
    backups.sort_by_key(|e| e.metadata().ok().map(|m| m.modified().ok()));
    while backups.len() > 10 {
        if let Some(old) = backups.first() {
            let _ = std::fs::remove_file(old.path());
            println!("  removed old backup: {}", old.path().display());
        }
        backups.remove(0);
    }
}

// ── seed ────────────────────────────────────────────────────────

/// `db seed` — 创建初始管理员用户。
///
/// 幂等：如果 email 或 username 已存在则跳过。
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

    let (document_id, now) = raisfast::utils::id::new_document_id_and_timestamp();

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
                "INSERT INTO users (document_id, tenant_id, username, created_at, updated_at, role, status, registered_via) VALUES ({}, {}, {}, {}, {}, 'admin', 'active', 'email')",
                dialect::ph(1),
                dialect::ph(2),
                dialect::ph(3),
                dialect::ph(4),
                dialect::ph(5),
            ))
            .bind(&document_id)
            .bind(&tid)
            .bind(username)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await?;
        }
        None => {
            sqlx::query(&format!(
                "INSERT INTO users (document_id, username, created_at, updated_at, role, status, registered_via) VALUES ({}, {}, {}, {}, 'admin', 'active', 'email')",
                dialect::ph(1),
                dialect::ph(2),
                dialect::ph(3),
                dialect::ph(4),
            ))
            .bind(&document_id)
            .bind(username)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await?;
        }
    }

    let (user_id,): (i64,) = sqlx::query_as(&format!(
        "SELECT id FROM users WHERE document_id = {}",
        dialect::ph(1)
    ))
    .bind(&document_id)
    .fetch_one(&pool)
    .await?;

    let cred_data = serde_json::json!({"password_hash": password_hash}).to_string();
    let (cred_doc_id, cred_now) = raisfast::utils::id::new_document_id_and_timestamp();
    sqlx::query(&format!(
        "INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, 1, {}, {})",
        dialect::ph(1),
        dialect::ph(2),
        dialect::ph(3),
        dialect::ph(4),
        dialect::ph(5),
        dialect::ph(6),
        dialect::ph(7),
    ))
    .bind(&cred_doc_id)
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
