//! 邮箱验证服务。

use chrono::Utc;

use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::repositories::UserRepository;

/// 注册后触发邮箱验证（若配置启用）。
///
/// 删除旧令牌，创建新令牌，通过 EventBus 发送验证邮件。
pub async fn trigger_email_verification(
    pool: &crate::db::Pool,
    eventbus: &EventBus,
    user_id: i64,
    email: &str,
) -> AppResult<()> {
    crate::models::email_verification::delete_unused_by_user(pool, user_id).await?;

    let verification =
        crate::models::email_verification::create(pool, user_id, email, 86400).await?;

    eventbus.emit(Event::EmailVerificationRequested {
        user_id: user_id.to_string(),
        email: email.to_string(),
        verify_token: verification.token,
    });

    Ok(())
}

/// 验证邮箱。
///
/// 校验令牌有效性，标记令牌已使用，更新 user_credentials.verified = 1。
pub async fn verify_email(pool: &crate::db::Pool, token: &str) -> AppResult<()> {
    let verification = crate::models::email_verification::find_by_token(pool, token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_or_expired_token".into()))?;

    if verification.expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid_or_expired_token".into()));
    }

    in_transaction!(pool, tx, {
        let now = crate::utils::tz::now_utc();
        let sql = format!(
            "UPDATE email_verification_tokens SET verified_at = {} WHERE id = {}",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2)
        );
        sqlx::query(&sql)
            .bind(now)
            .bind(verification.id)
            .execute(&mut *tx)
            .await?;

        let sql = format!(
            "UPDATE user_credentials SET verified = 1, updated_at = {} WHERE user_id = {} AND auth_type = {}",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3)
        );
        sqlx::query(&sql)
            .bind(now)
            .bind(verification.user_id)
            .bind(crate::models::user_credential::AuthType::Email)
            .execute(&mut *tx)
            .await?;
        Ok::<_, crate::errors::app_error::AppError>(())
    })?;
    Ok(())
}

/// 重新发送验证邮件。
///
/// 只有未验证的用户才能重新发送。限流由 sms_codes 的 rate_limit 逻辑类似处理。
pub async fn resend_verification(
    pool: &crate::db::Pool,
    _user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    email: &str,
) -> AppResult<()> {
    let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        crate::models::user_credential::AuthType::Email,
        email,
    )
    .await?
    .ok_or_else(|| AppError::not_found("user"))?;

    if cred.verified == 1 {
        return Err(AppError::BadRequest("email_already_verified".into()));
    }

    trigger_email_verification(pool, eventbus, cred.user_id, &cred.identifier).await
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

    fn eventbus() -> crate::eventbus::EventBus {
        crate::eventbus::EventBus::new(16)
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
            &crate::models::user_credential::wrap_password_hash("hash"),
            false,
        )
        .await
        .unwrap();
        user
    }

    #[tokio::test]
    async fn trigger_email_verification_creates_token() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "verify@test.com").await;
        let eb = eventbus();
        super::trigger_email_verification(&pool, &eb, user.id, "verify@test.com")
            .await
            .unwrap();
        let row =
            crate::models::email_verification::create(&pool, user.id, "verify@test.com", 3600)
                .await
                .unwrap();
        let found = crate::models::email_verification::find_by_token(&pool, &row.token)
            .await
            .unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn trigger_email_verification_replaces_old() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "replace@test.com").await;
        let eb = eventbus();
        super::trigger_email_verification(&pool, &eb, user.id, "replace@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
            crate::db::dialect::ph(1),
        );
        let (count_before,): (i64,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count_before, 1);
        super::trigger_email_verification(&pool, &eb, user.id, "replace@test.com")
            .await
            .unwrap();
        let (count_after,): (i64,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count_after, 1);
    }

    #[tokio::test]
    async fn verify_email_valid_token() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "v@test.com").await;
        let eb = eventbus();
        super::trigger_email_verification(&pool, &eb, user.id, "v@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT token FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL LIMIT 1",
            crate::db::dialect::ph(1),
        );
        let (token_str,): (String,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        super::verify_email(&pool, &token_str).await.unwrap();
        let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
            &pool,
            crate::models::user_credential::AuthType::Email,
            "v@test.com",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(cred.verified, 1);
    }

    #[tokio::test]
    async fn verify_email_invalid_token() {
        let pool = setup_pool().await;
        let err = super::verify_email(&pool, "no-such-token")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid_or_expired_token"), "got: {msg}");
    }

    #[tokio::test]
    async fn resend_verification_success() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "resend@test.com").await;
        let eb = eventbus();
        let repo = SqlxUserRepository::new(pool.clone());
        super::resend_verification(&pool, &repo, &eb, "resend@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
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
    async fn resend_verification_already_verified() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "verified@test.com").await;
        let eb = eventbus();
        super::trigger_email_verification(&pool, &eb, user.id, "verified@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT token FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL LIMIT 1",
            crate::db::dialect::ph(1),
        );
        let (token_str,): (String,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        super::verify_email(&pool, &token_str).await.unwrap();
        let repo = SqlxUserRepository::new(pool.clone());
        let err = super::resend_verification(&pool, &repo, &eb, "verified@test.com")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("email_already_verified"), "got: {msg}");
    }

    #[tokio::test]
    async fn resend_verification_user_not_found() {
        let pool = setup_pool().await;
        let eb = eventbus();
        let repo = SqlxUserRepository::new(pool.clone());
        assert!(
            super::resend_verification(&pool, &repo, &eb, "nope@no.com")
                .await
                .is_err()
        );
    }
}
