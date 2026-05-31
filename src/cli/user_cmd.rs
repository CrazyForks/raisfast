//! `user` subcommand: user account management.

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;
use raisfast::models::user::{RegisteredVia, UserRole, UserStatus};
use raisfast::models::user_credential;

pub async fn create(
    config: &AppConfig,
    email: &str,
    username: &str,
    password: &str,
    role: &str,
) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    if raisfast_derive::crud_exists!(&pool, "users", where: ("username", username))? {
        eprintln!("error: username already exists ({username})");
        std::process::exit(1);
    }

    if user_credential::find_by_auth_type_and_identifier(
        &pool,
        user_credential::AuthType::Email,
        email,
    )
    .await?
    .is_some()
    {
        eprintln!("error: email already exists ({email})");
        std::process::exit(1);
    }

    let role = match role {
        "admin" => UserRole::Admin,
        "editor" => UserRole::Editor,
        "author" => UserRole::Author,
        _ => UserRole::Reader,
    };

    let password_hash = raisfast::services::auth::hash_password(password)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;

    let (id, now) = (
        raisfast::utils::id::new_snowflake_id(),
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

    if let Some(ref tid) = tid {
        raisfast_derive::crud_insert!(&pool, "users", [
            "id" => id,
            "tenant_id" => tid,
            "username" => username,
            "created_at" => now,
            "updated_at" => now,
            "role" => role,
            "status" => UserStatus::Active,
            "registered_via" => RegisteredVia::Email
        ])?;
    } else {
        raisfast_derive::crud_insert!(&pool, "users", [
            "id" => id,
            "username" => username,
            "created_at" => now,
            "updated_at" => now,
            "role" => role,
            "status" => UserStatus::Active,
            "registered_via" => RegisteredVia::Email
        ])?;
    }

    let cred_data = user_credential::wrap_password_hash(&password_hash);
    user_credential::create(
        &pool,
        id,
        user_credential::AuthType::Email,
        email,
        &cred_data,
        true,
    )
    .await?;

    println!("user created:");
    println!("  id:       {}", *id);
    println!("  email:    {email}");
    println!("  username: {username}");
    println!("  role:     {}", role.as_str());
    Ok(())
}

pub async fn list(config: &AppConfig) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let users = raisfast::models::user::find_all(&pool, 1, 100, None).await?;

    println!(
        "{:<20} {:<25} {:<10} {:<10}",
        "ID", "USERNAME", "ROLE", "STATUS"
    );
    println!("{}", "-".repeat(70));
    for u in &users.0 {
        println!(
            "{:<20} {:<25} {:<10} {:<10}",
            *u.id,
            u.username,
            u.role.as_str(),
            u.status.as_str(),
        );
    }
    println!();
    println!("total: {}", users.1);
    Ok(())
}

pub async fn passwd(config: &AppConfig, username: &str, password: &str) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let user = raisfast::models::user::find_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;

    let creds = user_credential::find_by_user_id(&pool, user.id).await?;
    let email_cred = creds
        .iter()
        .find(|c| c.auth_type == user_credential::AuthType::Email);

    let password_hash = raisfast::services::auth::hash_password(password)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;
    let cred_data = user_credential::wrap_password_hash(&password_hash);

    if let Some(cred) = email_cred {
        user_credential::update_credential_data(&pool, cred.id, &cred_data).await?;
    } else {
        user_credential::create(
            &pool,
            user.id,
            user_credential::AuthType::Email,
            username,
            &cred_data,
            true,
        )
        .await?;
    }

    println!("password updated for user: {username}");
    Ok(())
}

pub async fn delete(config: &AppConfig, username: &str, force: bool) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let user = raisfast::models::user::find_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;

    if user.role == UserRole::Admin && !force {
        eprintln!("error: refusing to delete admin user. Use --force to override.");
        std::process::exit(1);
    }

    let creds = user_credential::find_by_user_id(&pool, user.id).await?;
    for cred in &creds {
        user_credential::delete_by_id(&pool, cred.id).await?;
    }

    raisfast::models::user::delete_by_id(&pool, user.id, None).await?;

    println!("user deleted: {username}");
    Ok(())
}

pub async fn disable(config: &AppConfig, username: &str) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let user = raisfast::models::user::find_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;

    if user.status == UserStatus::Suspended {
        eprintln!("user already disabled: {username}");
        std::process::exit(1);
    }

    let now = raisfast::utils::tz::now_utc();
    raisfast_derive::crud_update!(&pool, "users",
        bind: ["status" => UserStatus::Suspended, "updated_at" => &now],
        where: ("id", user.id)
    )?;

    println!("user disabled: {username}");
    Ok(())
}

pub async fn enable(config: &AppConfig, username: &str) -> anyhow::Result<()> {
    let pool = init_pool(&config.database_url, 1).await?;

    let user = raisfast::models::user::find_by_username(&pool, username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;

    if user.status == UserStatus::Active {
        eprintln!("user already active: {username}");
        std::process::exit(1);
    }

    let now = raisfast::utils::tz::now_utc();
    raisfast_derive::crud_update!(&pool, "users",
        bind: ["status" => UserStatus::Active, "updated_at" => &now],
        where: ("id", user.id)
    )?;

    println!("user enabled: {username}");
    Ok(())
}
