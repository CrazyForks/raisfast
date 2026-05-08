//! 密码重置服务。

use chrono::Utc;

use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::repositories::UserRepository;

/// 请求密码重置。
///
/// 查找用户，删除旧令牌，创建新令牌，通过 EventBus 触发邮件发送。
/// 无论用户是否存在都返回成功（防止邮箱枚举）。
pub async fn forgot_password(
    pool: &crate::db::Pool,
    user_repo: &dyn UserRepository,
    eventbus: &crate::eventbus::EventBus,
    email: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let user = match user_repo.find_by_email(email, tenant_id).await? {
        Some(u) => u,
        None => return Ok(()),
    };

    crate::models::password_reset::delete_unused_by_user(pool, user.id).await?;

    let reset_token = crate::models::password_reset::create(pool, user.id, 3600).await?;

    eventbus.emit(crate::eventbus::Event::PasswordResetRequested {
        user_id: user.document_id,
        email: user.email,
        reset_token: reset_token.token,
    });

    Ok(())
}

/// 重置密码。
///
/// 验证令牌有效性（未使用且未过期），更新密码，标记令牌已使用，
/// 删除所有刷新令牌使旧会话失效。
pub async fn reset_password(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    token: &str,
    new_password: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let reset_token = crate::models::password_reset::find_by_token(pool, token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_or_expired_token".into()))?;

    let expires_at = chrono::DateTime::parse_from_rfc3339(&reset_token.expires_at)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid token expiry")))?;

    if expires_at < Utc::now() {
        return Err(AppError::BadRequest("invalid_or_expired_token".into()));
    }

    crate::services::auth::validate_password_strength(new_password)?;
    let new_hash = crate::services::auth::hash_password(new_password)?;

    let mut tx = pool.begin().await?;

    let sql = format!(
        "UPDATE users SET password_hash = {}, updated_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3)
    );
    let now = Utc::now().to_rfc3339();
    sqlx::query(&sql)
        .bind(&new_hash)
        .bind(&now)
        .bind(reset_token.user_id)
        .execute(&mut *tx)
        .await?;

    let sql = format!(
        "UPDATE password_reset_tokens SET used_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2)
    );
    sqlx::query(&sql)
        .bind(&now)
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

    tx.commit().await?;

    let _ = user_repo;
    let _ = tenant_id;
    Ok(())
}

/// OAuth 用户设置密码。
///
/// 已登录用户（通过 OAuth 注册、无密码）设置密码。不需要旧密码验证。
pub async fn set_password(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    new_password: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    let user = user_repo
        .find_by_id(user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    if !user.password_hash.starts_with("!oauth:") {
        return Err(AppError::BadRequest("password_already_set".into()));
    }

    crate::services::auth::validate_password_strength(new_password)?;
    let new_hash = crate::services::auth::hash_password(new_password)?;
    user_repo
        .update_password(user_id, &new_hash, tenant_id)
        .await?;

    let _ = pool;
    Ok(())
}
