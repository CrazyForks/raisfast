//! 短信验证码服务。

use chrono::Utc;

use crate::dto::LoginResponse;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::repositories::{RefreshTokenRepository, UserRepository};

/// 发送短信验证码。
///
/// 检查配置是否启用、限流、生成验证码、入库、通过 Worker 发送。
pub async fn send_sms_code(
    pool: &crate::db::Pool,
    config: &crate::config::app::AppConfig,
    phone: &str,
    purpose: &str,
) -> AppResult<()> {
    if !config.registration_sms_enabled {
        return Err(AppError::BadRequest("sms_not_enabled".into()));
    }

    crate::models::sms_code::is_rate_limited(pool, phone, purpose, config.sms_rate_limit_secs)
        .await?
        .then_some(())
        .ok_or_else(|| AppError::BadRequest("sms_rate_limited".into()))?;

    let code = crate::models::sms_code::generate_code(config.sms_code_length);
    crate::models::sms_code::create(
        pool,
        phone,
        &code,
        purpose,
        config.sms_code_expires_in,
        None,
    )
    .await?;

    tracing::info!("[sms] code generated for phone={phone} purpose={purpose}");

    Ok(())
}

/// 验证短信验证码并自动注册/登录。
///
/// 验证通过后：若手机号已注册则直接登录，否则自动创建用户（无密码）并登录。
#[allow(clippy::too_many_arguments)]
pub async fn verify_sms_and_auth(
    user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    pool: &crate::db::Pool,
    phone: &str,
    code: &str,
    purpose: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let sms = crate::models::sms_code::find_latest_unverified(pool, phone, purpose)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_code".into()))?;

    let result = crate::models::sms_code::verify_code(pool, sms.id, code).await?;

    match result {
        crate::models::sms_code::VerifyResult::Verified => {}
        crate::models::sms_code::VerifyResult::WrongCode => {
            return Err(AppError::BadRequest("wrong_code".into()));
        }
        crate::models::sms_code::VerifyResult::Expired => {
            return Err(AppError::BadRequest("code_expired".into()));
        }
        crate::models::sms_code::VerifyResult::AlreadyUsed => {
            return Err(AppError::BadRequest("code_already_used".into()));
        }
        crate::models::sms_code::VerifyResult::MaxAttempts => {
            return Err(AppError::BadRequest("max_attempts".into()));
        }
    }

    let user = match user_repo.find_by_phone(phone).await? {
        Some(u) => u,
        None => {
            let username = format!(
                "user_{}",
                &phone.replace(|c: char| !c.is_ascii_alphanumeric(), "")
            );
            let password_hash = format!("!sms:{phone}");
            user_repo
                .create(
                    crate::commands::CreateUserCmd {
                        email: format!("!sms:{phone}"),
                        username,
                        password_hash,
                    },
                    None,
                )
                .await?
        }
    };

    let access_token = crate::services::auth::generate_access_token_internal(
        &user.document_id,
        user.id,
        &user.role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let refresh_token_str = crate::services::auth::generate_refresh_token_string_internal()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token_repo
        .create_token(user.id, &refresh_token_str, &expires_at.to_rfc3339())
        .await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: user.into(),
    })
}

/// 已登录用户绑定手机号。
pub async fn bind_phone(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    phone: &str,
    code: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    if user_repo.find_by_phone(phone).await?.is_some() {
        return Err(AppError::Conflict("phone_already_bound".into()));
    }

    let sms = crate::models::sms_code::find_latest_unverified(pool, phone, "bind_phone")
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_code".into()))?;

    let result = crate::models::sms_code::verify_code(pool, sms.id, code).await?;

    match result {
        crate::models::sms_code::VerifyResult::Verified => {}
        crate::models::sms_code::VerifyResult::WrongCode => {
            return Err(AppError::BadRequest("wrong_code".into()));
        }
        crate::models::sms_code::VerifyResult::Expired => {
            return Err(AppError::BadRequest("code_expired".into()));
        }
        crate::models::sms_code::VerifyResult::AlreadyUsed => {
            return Err(AppError::BadRequest("code_already_used".into()));
        }
        crate::models::sms_code::VerifyResult::MaxAttempts => {
            return Err(AppError::BadRequest("max_attempts".into()));
        }
    }

    user_repo.update_phone(user_id, phone, tenant_id).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn sms_code_generate_length() {
        let code = crate::models::sms_code::generate_code(6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
