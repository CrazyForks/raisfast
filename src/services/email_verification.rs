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
/// 校验令牌有效性，标记令牌已使用，更新 users.email_verified = 1。
pub async fn verify_email(pool: &crate::db::Pool, token: &str) -> AppResult<()> {
    let verification = crate::models::email_verification::find_by_token(pool, token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_or_expired_token".into()))?;

    let expires_at = chrono::DateTime::parse_from_rfc3339(&verification.expires_at)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid token expiry")))?;

    if expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid_or_expired_token".into()));
    }

    let mut tx = pool.begin().await?;

    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE email_verification_tokens SET verified_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2)
    );
    sqlx::query(&sql)
        .bind(&now)
        .bind(verification.id)
        .execute(&mut *tx)
        .await?;

    let sql = format!(
        "UPDATE users SET email_verified = 1, updated_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2)
    );
    sqlx::query(&sql)
        .bind(&now)
        .bind(verification.user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// 重新发送验证邮件。
///
/// 只有未验证的用户才能重新发送。限流由 sms_codes 的 rate_limit 逻辑类似处理。
pub async fn resend_verification(
    pool: &crate::db::Pool,
    user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    email: &str,
) -> AppResult<()> {
    let user = user_repo
        .find_by_email(email, None)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    if user.email_verified == 1 {
        return Err(AppError::BadRequest("email_already_verified".into()));
    }

    trigger_email_verification(pool, eventbus, user.id, &user.email).await
}
