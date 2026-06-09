//! Email verification service.

use crate::types::snowflake_id::SnowflakeId;
use chrono::Utc;

use crate::aspects::engine::AspectEngine;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;

pub async fn trigger_email_verification(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
    user_id: SnowflakeId,
    email: &str,
) -> AppResult<()> {
    crate::models::email_verification::delete_unused_by_user(pool, user_id).await?;

    let verification =
        crate::models::email_verification::create(pool, user_id, email, 86400).await?;

    aspect_engine.emit(Event::EmailVerificationRequested {
        user_id,
        email: email.to_string(),
        token: verification,
    });

    Ok(())
}

/// Verify an email address.
///
/// Validates the token, marks it as used, and updates user_credentials.verified = 1.
pub async fn verify_email(pool: &crate::db::Pool, token: &str) -> AppResult<()> {
    let verification = crate::models::email_verification::find_by_token(pool, token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_or_expired_token".into()))?;

    if verification.expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid_or_expired_token".into()));
    }

    in_transaction!(pool, tx, {
        crate::models::email_verification::tx_mark_verified(&mut tx, verification.id).await?;

        crate::models::user_credential::tx_verify_email_by_user(&mut tx, verification.user_id)
            .await?;
        Ok::<_, crate::errors::app_error::AppError>(())
    })?;
    Ok(())
}

/// Resend a verification email.
///
/// Only unverified users can request a resend. Rate limiting is handled similarly to sms_codes.
pub async fn resend_verification(
    pool: &crate::db::Pool,
    aspect_engine: &AspectEngine,
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

    trigger_email_verification(pool, aspect_engine, cred.user_id, &cred.identifier).await
}

#[cfg(test)]
mod tests {
    use crate::DbDriver;
    use crate::commands::CreateUserCmd;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn aspect_engine() -> crate::aspects::engine::AspectEngine {
        crate::aspects::engine::AspectEngine::new()
    }

    async fn insert_user(pool: &crate::db::Pool, email: &str) -> crate::models::user::User {
        let user = crate::models::user::create(
            pool,
            &CreateUserCmd {
                username: email.to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
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
        let ae = aspect_engine();
        super::trigger_email_verification(&pool, &ae, user.id, "verify@test.com")
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
        let ae = aspect_engine();
        super::trigger_email_verification(&pool, &ae, user.id, "replace@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
            crate::db::Driver::ph(1),
        );
        let (count_before,): (i64,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count_before, 1);
        super::trigger_email_verification(&pool, &ae, user.id, "replace@test.com")
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
        let ae = aspect_engine();
        super::trigger_email_verification(&pool, &ae, user.id, "v@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT token FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL LIMIT 1",
            crate::db::Driver::ph(1),
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
        let ae = aspect_engine();
        super::resend_verification(&pool, &ae, "resend@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
            crate::db::Driver::ph(1),
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
        let ae = aspect_engine();
        super::trigger_email_verification(&pool, &ae, user.id, "verified@test.com")
            .await
            .unwrap();
        let sql = format!(
            "SELECT token FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL LIMIT 1",
            crate::db::Driver::ph(1),
        );
        let (token_str,): (String,) = sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        super::verify_email(&pool, &token_str).await.unwrap();
        let err = super::resend_verification(&pool, &ae, "verified@test.com")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("email_already_verified"), "got: {msg}");
    }

    #[tokio::test]
    async fn resend_verification_user_not_found() {
        let pool = setup_pool().await;
        let ae = aspect_engine();
        assert!(
            super::resend_verification(&pool, &ae, "nope@no.com")
                .await
                .is_err()
        );
    }
}
