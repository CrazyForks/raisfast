//! Email verification service.

use crate::types::snowflake_id::SnowflakeId;
use chrono::Utc;

use crate::errors::app_error::{AppError, AppResult};
use crate::event::{Event, EventEmitter};

pub async fn trigger_email_verification(
    pool: &crate::db::Pool,
    emitter: &EventEmitter,
    user_id: SnowflakeId,
    email: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    crate::models::email_verification::delete_unused_by_user(pool, user_id).await?;

    let verification =
        crate::models::email_verification::create(pool, user_id, email, 86400).await?;

    emitter.emit(Event::EmailVerificationRequested {
        user_id,
        email: email.to_string(),
        tenant_id: tenant_id.map(|t| t.to_string()),
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
    emitter: &EventEmitter,
    email: &str,
) -> AppResult<()> {
    let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        crate::models::user_credential::AuthType::Email,
        email,
    )
    .await?
    .ok_or_else(|| AppError::not_found("user"))?;

    if cred.verified {
        return Err(AppError::BadRequest("email_already_verified".into()));
    }

    trigger_email_verification(pool, emitter, cred.user_id, &cred.identifier, None).await
}

#[cfg(test)]
mod tests {
    use crate::DbDriver;
    use crate::commands::CreateUserCmd;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn emitter() -> crate::event::EventEmitter {
        crate::event::EventEmitter::eventbus_only(crate::eventbus::EventBus::new(16))
    }

    async fn insert_user(pool: &crate::db::Pool, email: &str) -> crate::models::user::User {
        let user = crate::models::user::create(
            pool,
            &CreateUserCmd {
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
        let email = format!("verify_{}@test.com", crate::utils::id::new_id());
        let user = insert_user(&pool, &email).await;
        let ae = emitter();
        super::trigger_email_verification(&pool, &ae, user.id, &email, None)
            .await
            .unwrap();
        let row = crate::models::email_verification::create(&pool, user.id, &email, 3600)
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
        let email = format!("replace_{}@test.com", crate::utils::id::new_id());
        let user = insert_user(&pool, &email).await;
        let ae = emitter();
        super::trigger_email_verification(&pool, &ae, user.id, &email, None)
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
        super::trigger_email_verification(&pool, &ae, user.id, &email, None)
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
        let email = format!("v_{}@test.com", crate::utils::id::new_id());
        let user = insert_user(&pool, &email).await;
        let ae = emitter();
        super::trigger_email_verification(&pool, &ae, user.id, &email, None)
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
            &email,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(cred.verified);
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
        let email = format!("resend_{}@test.com", crate::utils::id::new_id());
        let user = insert_user(&pool, &email).await;
        let ae = emitter();
        super::resend_verification(&pool, &ae, &email)
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
        let email = format!("verified_{}@test.com", crate::utils::id::new_id());
        let user = insert_user(&pool, &email).await;
        let ae = emitter();
        super::trigger_email_verification(&pool, &ae, user.id, &email, None)
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
        let err = super::resend_verification(&pool, &ae, &email)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("email_already_verified"), "got: {msg}");
    }

    #[tokio::test]
    async fn resend_verification_user_not_found() {
        let pool = setup_pool().await;
        let ae = emitter();
        assert!(
            super::resend_verification(&pool, &ae, "nope@no.com")
                .await
                .is_err()
        );
    }
}
