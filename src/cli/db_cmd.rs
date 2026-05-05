//! `db` 子命令：数据库迁移、备份、种子数据。

use std::path::Path;

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;
use raisfast::db::dialect;

// ── migrate ──────────────────────────────────────────────────────

/// `db migrate` — 执行 `migrations/` 目录中尚未应用的 SQL 文件。
///
/// 使用 `_migrations` 表记录已执行的文件名，幂等安全。
pub async fn migrate(config: &AppConfig) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = init_pool(&config.database_url, 1).await?;

    let create_sql =
        dialect::translate("CREATE TABLE IF NOT EXISTS _migrations (filename TEXT PRIMARY KEY)");
    sqlx::query(&create_sql).execute(&pool).await?;

    let migrations_dir = Path::new("./migrations");
    if !migrations_dir.exists() {
        anyhow::bail!("migrations directory not found: ./migrations");
    }

    let mut entries: Vec<_> = std::fs::read_dir(migrations_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "sql"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        println!("no migration files found");
        return Ok(());
    }

    let check_sql = dialect::translate("SELECT COUNT(*) FROM _migrations WHERE filename = ?");
    let insert_sql = dialect::translate("INSERT INTO _migrations (filename) VALUES (?)");

    let mut applied = 0u32;
    for entry in &entries {
        let filename = entry.file_name().to_string_lossy().to_string();
        let sql = std::fs::read_to_string(entry.path())?;

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
            .unwrap_or("./data/blog.db");

        if !Path::new(db_path).exists() {
            anyhow::bail!("database file not found: {}", db_path);
        }

        std::fs::create_dir_all(output_dir)?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_name = format!("blog_{}.db", timestamp);
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

    let existing: i64 = sqlx::query_scalar(&dialect::translate(
        "SELECT COUNT(*) FROM users WHERE email = ?",
    ))
    .bind(email)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    if existing > 0 {
        println!("seed: admin user already exists ({email}), skipping");
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

    sqlx::query(&dialect::translate(
        "INSERT INTO users (id, tenant_id, email, username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'admin', ?, ?)",
    ))
    .bind(&id)
    .bind(&tid)
    .bind(email)
    .bind(username)
    .bind(&password_hash)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await?;

    println!("seed: admin user created");
    println!("  email:    {email}");
    println!("  username: {username}");
    println!("  role:     admin");
    Ok(())
}
