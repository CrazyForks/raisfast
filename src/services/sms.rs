//! SMS verification code service.

use chrono::Utc;

use crate::dto::LoginResponse;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;

/// Send an SMS verification code.
///
/// Checks if the feature is enabled in config, enforces rate limiting, generates the code, persists it, and sends it via a Worker.
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

/// Verify an SMS code and auto-register/login.
///
/// After successful verification: if the phone number is already registered, log in directly; otherwise, auto-create a user and log in.
#[allow(clippy::too_many_arguments)]
pub async fn verify_sms_and_auth(
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

    let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        crate::models::user_credential::AuthType::Phone,
        phone,
    )
    .await?;

    let user = match cred {
        Some(c) => crate::models::user::find_by_id(pool, c.user_id, None)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?,
        None => {
            let username = format!(
                "user_{}",
                &phone.replace(|c: char| !c.is_ascii_alphanumeric(), "")
            );
            let user = crate::models::user::create(
                pool,
                &crate::commands::CreateUserCmd {
                    username,
                    registered_via: crate::models::user::RegisteredVia::Phone,
                    role: None,
                },
                None,
            )
            .await?;
            crate::models::user_credential::create(
                pool,
                user.id,
                crate::models::user_credential::AuthType::Phone,
                phone,
                "",
                true,
            )
            .await?;
            user
        }
    };

    let user_role = user.role;
    let access_token = crate::services::auth::generate_access_token_internal(
        user.id,
        user_role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let refresh_token_str = crate::services::auth::generate_refresh_token_string_internal()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    crate::models::refresh_token::create_token(
        pool,
        user.id,
        &refresh_token_str,
        &expires_at.to_rfc3339(),
    )
    .await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: crate::dto::UserResponse::from_user_with_contacts(pool, user).await?,
    })
}

/// Bind a phone number for a logged-in user.
pub async fn bind_phone(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    phone: &str,
    code: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let tenant_id = auth.tenant_id();
    let _user = crate::models::user::find_by_id(pool, user_id, tenant_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        crate::models::user_credential::AuthType::Phone,
        phone,
    )
    .await?
    .is_some()
    {
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

    crate::models::user_credential::create(
        pool,
        _user.id,
        crate::models::user_credential::AuthType::Phone,
        phone,
        "",
        true,
    )
    .await?;
    Ok(())
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
