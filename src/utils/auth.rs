//! Authentication utility functions

use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::AppError;
use crate::services::auth::Claims;

pub fn extract_claims(parts: &mut Parts, state: &AppState) -> Result<Claims, AppError> {
    let auth_header = parts
        .headers
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix(crate::constants::AUTH_BEARER_PREFIX)
        .ok_or(AppError::Unauthorized)?;

    crate::services::auth::verify_token(token, &state.jwt_decoding_key)
}
