//! Password reset service.

use chrono::Utc;

use crate::aspects::engine::AspectEngine;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::repositories::UserRepository;

pub async fn forgot_password(
    pool: &crate::db::Pool,
    _user_repo: &dyn UserRepository,
    aspect_engine: &AspectEngine,
    email: &str,
    _tenant_id: Option<&str>,
) -> AppResult<()> {
    let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        crate::models::user_credential::AuthType::Email,
        email,
    )
    .await?;
    let user = match cred {
        Some(c) => crate::models::user::find_by_pk(pool, c.user_id, None).await?,
        None => None,
    };

    let user = match user {
        Some(u) => u,
        None => return Ok(()),
    };

    crate::models::password_reset::delete_unused_by_user(pool, user.id).await?;

    let reset_token = crate::models::password_reset::create(pool, user.id, 3600).await?;

    aspect_engine.emit(Event::PasswordResetRequested {
        user: user.clone(),
        token: reset_token,
    });

    Ok(())
}

/// Reset a password.
///
/// Validates the token (unused and not expired), updates the credential password, marks the token as used,
/// and deletes all refresh tokens to invalidate old sessions.
pub async fn reset_password(
    _user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    token: &str,
    new_password: &str,
    _tenant_id: Option<&str>,
) -> AppResult<()> {
    let reset_token = crate::models::password_reset::find_by_token(pool, token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_or_expired_token".into()))?;

    if reset_token.expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid_or_expired_token".into()));
    }

    crate::services::auth::validate_password_strength(new_password)?;
    let new_hash = crate::services::auth::hash_password(new_password)?;

    in_transaction!(pool, tx, {
        let now = crate::utils::tz::now_utc();

        let sql = format!(
            "SELECT id, auth_type FROM user_credentials WHERE user_id = {} AND auth_type = {} LIMIT 1",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2)
        );
        let row: Option<(i64, crate::models::user_credential::AuthType)> = sqlx::query_as(&sql)
            .bind(reset_token.user_id)
            .bind(crate::models::user_credential::AuthType::Email)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some((cred_id, _)) = row {
            let sql = format!(
                "UPDATE user_credentials SET credential_data = {}, updated_at = {} WHERE id = {}",
                crate::db::dialect::ph(1),
                crate::db::dialect::ph(2),
                crate::db::dialect::ph(3)
            );
            sqlx::query(&sql)
                .bind(crate::models::user_credential::wrap_password_hash(
                    &new_hash,
                ))
                .bind(now)
                .bind(cred_id)
                .execute(&mut *tx)
                .await?;
        }

        let sql = format!(
            "UPDATE password_reset_tokens SET used_at = {} WHERE id = {}",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2)
        );
        sqlx::query(&sql)
            .bind(now)
            .bind(reset_token.id)
            .execute(&mut *tx)
            .await?;

        let del_sql = format!(
            "DELETE FROM refresh_tokens WHERE user_id = {}",
            crate::db::dialect::ph(1)
        );
        sqlx::query(&del_sql)
            .bind(reset_token.user_id)
            .execute(&mut *tx)
            .await?;
        Ok::<_, crate::errors::app_error::AppError>(())
    })?;

    Ok(())
}

/// Set a password for an OAuth user.
///
/// A logged-in user (registered via OAuth, with no password) sets a password. No old password verification required.
pub async fn set_password(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    email: &str,
    new_password: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    let user = user_repo
        .find_by_id(user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let creds = crate::models::user_credential::find_by_user_id(pool, user.id).await?;
    let has_password = creds.iter().any(|c| {
        c.auth_type == crate::models::user_credential::AuthType::Email
            && !c.credential_data.is_empty()
    });

    if has_password {
        return Err(AppError::BadRequest("password_already_set".into()));
    }

    crate::services::auth::validate_password_strength(new_password)?;
    let new_hash = crate::services::auth::hash_password(new_password)?;

    if let Some(cred) = creds
        .iter()
        .find(|c| c.auth_type == crate::models::user_credential::AuthType::Email)
    {
        crate::models::user_credential::update_credential_data(
            pool,
            cred.id,
            &crate::models::user_credential::wrap_password_hash(&new_hash),
        )
        .await?;
    } else {
        crate::models::user_credential::create(
            pool,
            user.id,
            crate::models::user_credential::AuthType::Email,
            email,
            &crate::models::user_credential::wrap_password_hash(&new_hash),
            true,
        )
        .await?;
    }

    let _ = pool;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreateUserCmd;
    use crate::repositories::sqlx_user::SqlxUserRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn aspect_engine() -> crate::aspects::engine::AspectEngine {
        crate::aspects::engine::AspectEngine::new()
    }

    async fn insert_user(pool: &crate::db::Pool, email: &str) -> crate::models::user::User {
        let repo = SqlxUserRepository::new(pool.clone());
        let user = repo
            .create(
                CreateUserCmd {
                    username: email.to_string(),
                    registered_via: crate::models::user::RegisteredVia::Email,
                },
                None,
            )
            .await
            .unwrap();
        crate::models::user_credential::create(
            pool,
            user.id,
            crate::models::user_credential::AuthType::Email,
            email,
            &crate::models::user_credential::wrap_password_hash(
                "$argon2id$v=19$m=19456,t=2,p=1$test$test",
            ),
            true,
        )
        .await
        .unwrap();
        user
    }

    #[tokio::test]
    async fn forgot_password_existing_user() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "reset@test.com").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let ae = aspect_engine();
        super::forgot_password(&pool, &repo, &ae, "reset@test.com", None)
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM password_reset_tokens WHERE user_id = {} AND used_at IS NULL",
            crate::db::dialect::ph(1),
        );
        let (count,): (i64,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn forgot_password_nonexistent_user_ok() {
        let pool = setup_pool().await;
        let repo = SqlxUserRepository::new(pool.clone());
        let ae = aspect_engine();
        super::forgot_password(&pool, &repo, &ae, "noone@test.com", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reset_password_invalid_token() {
        let pool = setup_pool().await;
        let repo = SqlxUserRepository::new(pool.clone());
        let err = super::reset_password(&repo, &pool, "bad-token", "NewPass1", None)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid_or_expired_token"), "got: {msg}");
    }

    #[tokio::test]
    async fn reset_password_weak_password() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "weak@test.com").await;
        let ae = aspect_engine();
        let repo = SqlxUserRepository::new(pool.clone());
        super::forgot_password(&pool, &repo, &ae, "weak@test.com", None)
            .await
            .unwrap();
        let sql = format!(
            "SELECT token FROM password_reset_tokens WHERE user_id = {} AND used_at IS NULL LIMIT 1",
            crate::db::dialect::ph(1),
        );
        let (token_str,): (String,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let err = super::reset_password(&repo, &pool, &token_str, "short", None)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("password"), "got: {msg}");
    }

    #[tokio::test]
    async fn set_password_oauth_user() {
        let pool = setup_pool().await;
        let repo = SqlxUserRepository::new(pool.clone());
        let user = repo
            .create(
                CreateUserCmd {
                    username: "oauthu".into(),
                    registered_via: crate::models::user::RegisteredVia::Oauth,
                },
                None,
            )
            .await
            .unwrap();
        crate::models::user_credential::create(
            &pool,
            user.id,
            crate::models::user_credential::AuthType::Email,
            "oauth@test.com",
            "",
            true,
        )
        .await
        .unwrap();
        let a = AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            crate::models::user::UserRole::Author,
            None,
        );
        super::set_password(&repo, &pool, &a, "oauth@test.com", "StrongPass1")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn set_password_already_set_rejected() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "already@test.com").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let a = AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            crate::models::user::UserRole::Author,
            None,
        );
        let err = super::set_password(&repo, &pool, &a, "already@test.com", "NewPass1")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("password_already_set"), "got: {msg}");
    }
}
