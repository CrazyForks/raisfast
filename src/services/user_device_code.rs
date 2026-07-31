//! User device code service
//!
//! Handles creation and exchange of one-time authorization codes for desktop IDE authentication.

use chrono::Utc;

use crate::dto::{ExchangeDeviceCodeResponse, UserResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::services::auth;

const DEVICE_CODE_EXPIRY_SECONDS: i64 = 120;

pub async fn create_device_code(
    pool: &crate::db::Pool,
    user_id: crate::types::snowflake_id::SnowflakeId,
    access_token: &str,
    refresh_token: &str,
) -> AppResult<String> {
    let code = crate::utils::id::new_id().to_string();
    let expires_at = Utc::now() + chrono::Duration::seconds(DEVICE_CODE_EXPIRY_SECONDS);

    crate::models::user_device_code::create(
        pool,
        user_id,
        &code,
        access_token,
        refresh_token,
        &expires_at.to_rfc3339(),
    )
    .await?;

    Ok(code)
}

pub async fn exchange_device_code(
    pool: &crate::db::Pool,
    code: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    tenant_id: Option<&str>,
) -> AppResult<ExchangeDeviceCodeResponse> {
    let stored = crate::models::user_device_code::find_by_code(pool, code)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid_device_code".into()))?;

    if stored.used_at.is_some() {
        return Err(AppError::BadRequest("device_code_already_used".into()));
    }

    let now = Utc::now();
    let expires_at: chrono::DateTime<Utc> = stored
        .expires_at
        .to_string()
        .parse()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid expires_at format")))?;
    if now > expires_at {
        return Err(AppError::BadRequest("device_code_expired".into()));
    }

    let user = crate::models::user::find_by_id(pool, stored.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    let new_refresh_token = crate::in_transaction!(pool, tx, {
        crate::models::user_device_code::tx_mark_used(&mut tx, stored.id).await?;

        crate::models::refresh_token::tx_delete_by_user(&mut tx, user.id).await?;

        let rt = auth::generate_refresh_token_string_internal()?;
        let new_expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
        crate::models::refresh_token::tx_create_token(
            &mut tx,
            user.id,
            &rt,
            &new_expires_at.to_rfc3339(),
        )
        .await?;

        Ok::<_, crate::errors::app_error::AppError>(rt)
    })?;

    let new_access_token = auth::generate_access_token_internal(
        user.id,
        user.role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;

    Ok(ExchangeDeviceCodeResponse {
        access_token: new_access_token,
        refresh_token: new_refresh_token,
        user: UserResponse::from_user_with_contacts(pool, user).await?,
    })
}
