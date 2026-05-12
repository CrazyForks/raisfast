//! Authentication and user service.
//!
//! Provides complete authentication business logic, including:
//!
//! - Password hashing and verification (Argon2id)
//! - JWT access token generation and verification (HS256)
//! - Refresh token generation and rotation
//! - User registration, login, logout

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::dto::{LoginResponse, RegisterRequest, UpdatePasswordRequest, UserResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::middleware::auth::AuthUser;
use crate::models::user::{UserRole, UserStatus};
use crate::models::user_credential::AuthType;
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::{RefreshTokenRepository, UserRepository};

/// JWT token claims (payload).
///
/// - `sub`: User ID.
/// - `role`: User role (e.g. `"admin"`, `"author"`).
/// - `tenant_id`: Tenant ID (defaults to `"default"`).
/// - `exp`: Expiration time (UNIX timestamp).
/// - `iat`: Issued-at time (UNIX timestamp).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub iid: i64,
    pub role: UserRole,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
    pub exp: usize,
    pub iat: usize,
}

fn default_tenant_id() -> String {
    crate::constants::DEFAULT_TENANT.to_string()
}

/// Validate password strength.
///
/// Requirements:
/// - At least 8 characters
/// - At least one uppercase letter
/// - At least one lowercase letter
/// - At least one digit
pub fn validate_password_strength(password: &str) -> AppResult<()> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    if !password.chars().any(char::is_uppercase) {
        return Err(AppError::BadRequest(
            "password must contain at least one uppercase letter".into(),
        ));
    }
    if !password.chars().any(char::is_lowercase) {
        return Err(AppError::BadRequest(
            "password must contain at least one lowercase letter".into(),
        ));
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "password must contain at least one digit".into(),
        ));
    }
    Ok(())
}

/// Hash a password using the Argon2id algorithm.
///
/// Generates a 32-byte random salt via `getrandom` and returns a PHC-format hash string.
pub fn hash_password(password: &str) -> AppResult<String> {
    let mut salt_bytes = [0u8; 32];
    getrandom::getrandom(&mut salt_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("salt generation failed: {e}")))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("salt encoding failed: {e}")))?;
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify that a plaintext password matches an Argon2 hash.
///
/// Returns `Ok(true)` if it matches, `Ok(false)` if it does not.
pub fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Generate an HS256-signed JWT access token.
pub(crate) fn generate_access_token_internal(
    user_id: &str,
    user_int_id: i64,
    role: UserRole,
    tenant_id: &str,
    secret: &str,
    expires_in: u64,
) -> AppResult<String> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() as usize) + (expires_in as usize);
    let claims = Claims {
        sub: user_id.to_string(),
        iid: user_int_id,
        role,
        tenant_id: tenant_id.to_string(),
        exp,
        iat,
    };
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("token encoding failed: {e}")))
}

/// Verify and decode a JWT token.
///
/// Returns [`AppError::Unauthorized`] uniformly for expired or invalid tokens.
pub fn verify_token(token: &str, key: &jsonwebtoken::DecodingKey) -> AppResult<Claims> {
    jsonwebtoken::decode::<Claims>(token, key, &Validation::default())
        .map(|data| data.claims)
        .map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::Unauthorized,
            _ => AppError::Unauthorized,
        })
}

/// Generate a 32-byte random refresh token, returned as a hex string.
pub(crate) fn generate_refresh_token_string_internal() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("refresh token generation failed: {e}")))?;
    Ok(hex::encode(bytes))
}

/// Test helper: generate a JWT token using a fixed secret.
#[allow(clippy::doc_lazy_continuation)]
#[must_use]
pub fn generate_access_token_for_test(user_id: &str, user_int_id: i64, role: UserRole) -> String {
    generate_access_token_internal(
        user_id,
        user_int_id,
        role,
        crate::constants::DEFAULT_TENANT,
        "test-secret-key-at-least-32-characters-long",
        900,
    )
    .unwrap()
}

/// User registration.
///
/// Checks if the email is already registered; if unique, hashes the password and creates
/// the user record and email credential within a transaction.
#[tracing::instrument(skip(_user_repo, eventbus), fields(username = tracing::field::Empty))]
pub async fn register(
    _user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    req: RegisterRequest,
    tenant_id: Option<&str>,
    require_email_verification: bool,
    pool: &crate::db::Pool,
) -> AppResult<UserResponse> {
    if crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        AuthType::Email,
        &req.email,
    )
    .await?
    .is_some()
    {
        return Err(AppError::Conflict("email_registered".into()));
    }

    validate_password_strength(&req.password)?;
    let password_hash = hash_password(&req.password)?;
    let cred_data = crate::models::user_credential::wrap_password_hash(&password_hash);
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let registered_via = crate::models::user::RegisteredVia::Email;

    let user = in_transaction!(pool, tx, {
        match tenant_id {
            Some(tid) => {
                let vals = (1..=5)
                    .map(crate::db::dialect::ph)
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "INSERT INTO users (document_id, tenant_id, username, created_at, updated_at, role, status, registered_via) VALUES ({vals}, {}, {}, {})",
                    crate::db::dialect::ph(6),
                    crate::db::dialect::ph(7),
                    crate::db::dialect::ph(8)
                );
                sqlx::query(&sql)
                    .bind(&document_id)
                    .bind(tid)
                    .bind(&req.username)
                    .bind(now)
                    .bind(now)
                    .bind(UserRole::Reader)
                    .bind(UserStatus::Active)
                    .bind(registered_via)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {
                let vals = (1..=4)
                    .map(crate::db::dialect::ph)
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "INSERT INTO users (document_id, username, created_at, updated_at, role, status, registered_via) VALUES ({vals}, {}, {}, {})",
                    crate::db::dialect::ph(5),
                    crate::db::dialect::ph(6),
                    crate::db::dialect::ph(7)
                );
                sqlx::query(&sql)
                    .bind(&document_id)
                    .bind(&req.username)
                    .bind(now)
                    .bind(now)
                    .bind(UserRole::Reader)
                    .bind(UserStatus::Active)
                    .bind(registered_via)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let filter = crate::db::tenant::tenant_filter_ph(tenant_id, 2);
        let sql = format!(
            "SELECT * FROM users WHERE document_id = {}{filter}",
            crate::db::dialect::ph(1)
        );
        let mut q = sqlx::query_as::<_, crate::models::user::User>(&sql).bind(&document_id);
        if let Some(tid) = tenant_id {
            q = q.bind(tid);
        }
        let user = q.fetch_optional(&mut *tx).await?.ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("failed to fetch newly created user"))
        })?;

        let (cred_doc_id, cred_now) = crate::utils::id::new_document_id_and_timestamp();
        let verified = if !require_email_verification { 1 } else { 0 };
        let cred_sql = format!(
            "INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3),
            crate::db::dialect::ph(4),
            crate::db::dialect::ph(5),
            crate::db::dialect::ph(6),
            crate::db::dialect::ph(7),
            crate::db::dialect::ph(8)
        );
        sqlx::query(&cred_sql)
            .bind(&cred_doc_id)
            .bind(user.id)
            .bind(AuthType::Email)
            .bind(&req.email)
            .bind(&cred_data)
            .bind(verified)
            .bind(cred_now)
            .bind(cred_now)
            .execute(&mut *tx)
            .await?;

        Ok::<_, crate::errors::app_error::AppError>(user)
    })?;

    eventbus.emit(Event::UserRegistered {
        id: user.document_id.clone(),
        username: user.username.clone(),
        email: req.email.clone(),
    });

    if require_email_verification {
        let _ = crate::services::email_verification::trigger_email_verification(
            pool, eventbus, user.id, &req.email,
        )
        .await;
    }

    UserResponse::from_user(user)
}

/// User login.
///
/// Looks up the user via email credential, verifies the password, and on success generates
/// an access token and a refresh token.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(_user_repo, refresh_token_repo, plugins, eventbus), fields(email = %req.email))]
pub async fn login(
    _user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    pool: &crate::db::Pool,
    req: &crate::dto::LoginRequest,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    tenant_id: Option<&str>,
    require_email_verification: bool,
) -> AppResult<LoginResponse> {
    let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        AuthType::Email,
        &req.email,
    )
    .await?
    .ok_or_else(|| AppError::Unauthorized)?;

    if !verify_password(
        &req.password,
        &crate::models::user_credential::extract_password_hash(&cred.credential_data)?,
    )? {
        plugins
            .dispatch_action(
                HookPoint::OnLogin,
                &serde_json::json!({"email": &req.email, "success": false}),
            )
            .await;
        return Err(AppError::Unauthorized);
    }

    if require_email_verification && cred.verified == 0 {
        return Err(AppError::BadRequest("email_not_verified".into()));
    }

    let user = crate::models::user::find_by_pk(pool, cred.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    if user.status != UserStatus::Active {
        return Err(AppError::BadRequest("account_disabled".into()));
    }

    if let Some(ref tid) = user.tenant_id
        && let Ok(Some(tenant)) = crate::models::tenant::find_by_id(pool, tid).await
        && tenant.status != crate::models::tenant::TenantStatus::Active
    {
        return Err(AppError::BadRequest("tenant_disabled".into()));
    }

    let user_role = user.role;
    let access_token = generate_access_token_internal(
        &user.document_id,
        user.id,
        user_role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let refresh_token_str = generate_refresh_token_string_internal()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token_repo
        .create_token(user.id, &refresh_token_str, &expires_at.to_rfc3339())
        .await?;

    plugins
        .dispatch_action(
            HookPoint::OnLogin,
            &serde_json::json!({"email": &req.email, "success": true, "user_id": &user.document_id}),
        )
        .await;

    eventbus.emit(Event::UserLoggedIn {
        id: user.document_id.clone(),
        success: true,
    });

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: UserResponse::from_user(user)?,
    })
}

/// Refresh token.
///
/// Validates the refresh token and performs token rotation: within a transaction, deletes
/// the old refresh token and generates new access and refresh tokens, ensuring atomicity.
#[allow(clippy::too_many_arguments)]
pub async fn refresh(
    _user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    pool: &crate::db::Pool,
    refresh_token_str: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    tenant_id: Option<&str>,
) -> AppResult<LoginResponse> {
    let stored = refresh_token_repo
        .find_by_token(refresh_token_str)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    if stored.expires_at < Utc::now() {
        let _ = refresh_token_repo.delete_by_token(refresh_token_str).await;
        return Err(AppError::Unauthorized);
    }

    let user = crate::models::user::find_by_pk(pool, stored.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    let user_role = user.role;
    let access_token = generate_access_token_internal(
        &user.document_id,
        user.id,
        user_role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let new_refresh_token = generate_refresh_token_string_internal()?;
    let new_expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    let new_expires_str = new_expires_at.to_rfc3339();
    let new_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_str();

    in_transaction!(pool, tx, {
        sqlx::query(&format!(
            "DELETE FROM refresh_tokens WHERE token = {}",
            crate::db::dialect::ph(1)
        ))
        .bind(refresh_token_str)
        .execute(&mut *tx)
        .await?;

        sqlx::query(&format!(
            "INSERT INTO refresh_tokens (document_id, user_id, token, expires_at, created_at) VALUES ({}, {}, {}, {}, {})",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3),
            crate::db::dialect::ph(4),
            crate::db::dialect::ph(5)
        ))
        .bind(&new_id)
        .bind(user.id)
        .bind(&new_refresh_token)
        .bind(&new_expires_str)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        Ok::<_, crate::errors::app_error::AppError>(())
    })?;

    Ok(LoginResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_in: jwt_access_expires,
        user: UserResponse::from_user(user)?,
    })
}

/// User logout.
///
/// Deletes all refresh tokens for the user, invalidating sessions across all devices.
pub async fn logout(
    pool: &crate::db::Pool,
    refresh_token_repo: &dyn RefreshTokenRepository,
    auth: &AuthUser,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;
    refresh_token_repo.delete_by_user(user.id).await
}

/// Change password.
///
/// After verifying the old password is correct, replaces the old hash with the new password hash,
/// and deletes all refresh tokens to invalidate all existing sessions.
pub async fn change_password(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    req: UpdatePasswordRequest,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    let _user = user_repo
        .find_by_id(user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let creds = crate::models::user_credential::find_by_user_id(pool, _user.id).await?;
    let password_cred = creds
        .iter()
        .find(|c| c.auth_type == crate::models::user_credential::AuthType::Email)
        .ok_or_else(|| AppError::BadRequest("no_password_credential".into()))?;

    if !verify_password(
        &req.old_password,
        &crate::models::user_credential::extract_password_hash(&password_cred.credential_data)?,
    )? {
        return Err(AppError::BadRequest("incorrect_password".into()));
    }

    validate_password_strength(&req.new_password)?;
    let new_hash = hash_password(&req.new_password)?;
    crate::models::user_credential::update_credential_data(
        pool,
        password_cred.id,
        &crate::models::user_credential::wrap_password_hash(&new_hash),
    )
    .await?;

    sqlx::query(&format!(
        "DELETE FROM refresh_tokens WHERE user_id = {}",
        crate::db::dialect::ph(1)
    ))
    .bind(_user.id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Bind an email/password credential
pub async fn bind_email_credential(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    email: &str,
    password: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    if crate::models::user_credential::find_by_auth_type_and_identifier(
        pool,
        AuthType::Email,
        email,
    )
    .await?
    .is_some()
    {
        return Err(AppError::Conflict("email_registered".into()));
    }

    validate_password_strength(password)?;
    let hash = hash_password(password)?;

    crate::models::user_credential::create(
        pool,
        user.id,
        AuthType::Email,
        email,
        &crate::models::user_credential::wrap_password_hash(&hash),
        false,
    )
    .await?;

    Ok(())
}

/// Delete a credential
pub async fn delete_credential(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    credential_id: i64,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let cred = crate::models::user_credential::find_by_id(pool, credential_id)
        .await?
        .ok_or_else(|| AppError::not_found("credential"))?;

    if cred.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    let count = crate::models::user_credential::count_by_user(pool, user.id).await?;
    if count <= 1 {
        return Err(AppError::BadRequest("cannot_remove_last_credential".into()));
    }

    crate::models::user_credential::delete_by_id(pool, credential_id).await?;
    Ok(())
}

/// List all credentials for the current user
pub async fn list_credentials(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<crate::models::user_credential::UserCredential>> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    crate::models::user_credential::find_by_user_id(pool, user.id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_strength_rejects_short() {
        assert!(validate_password_strength("Ab1").is_err());
    }

    #[test]
    fn password_strength_rejects_no_uppercase() {
        assert!(validate_password_strength("abcdefgh1").is_err());
    }

    #[test]
    fn password_strength_rejects_no_lowercase() {
        assert!(validate_password_strength("ABCDEFGH1").is_err());
    }

    #[test]
    fn password_strength_rejects_no_digit() {
        assert!(validate_password_strength("Abcdefgh").is_err());
    }

    #[test]
    fn password_strength_accepts_valid() {
        assert!(validate_password_strength("Password1").is_ok());
    }

    #[test]
    fn hash_and_verify_password_roundtrip() {
        let hash = hash_password("Secret123").unwrap();
        assert!(verify_password("Secret123", &hash).unwrap());
    }

    #[test]
    fn verify_password_rejects_wrong() {
        let hash = hash_password("Secret123").unwrap();
        assert!(!verify_password("WrongPass1", &hash).unwrap());
    }

    #[test]
    fn generate_and_verify_token() {
        let token =
            generate_access_token_internal("user-1", 1, UserRole::Admin, "default", "secret", 900)
                .unwrap();
        let key = jsonwebtoken::DecodingKey::from_secret("secret".as_bytes());
        let claims = verify_token(&token, &key).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.role, UserRole::Admin);
    }

    #[test]
    fn verify_token_rejects_wrong_secret() {
        let token = generate_access_token_internal(
            "user-1",
            1,
            UserRole::Admin,
            "default",
            "secret-a",
            900,
        )
        .unwrap();
        let key = jsonwebtoken::DecodingKey::from_secret("secret-b".as_bytes());
        assert!(verify_token(&token, &key).is_err());
    }

    #[test]
    fn verify_token_rejects_expired() {
        let now = chrono::Utc::now();
        let claims = Claims {
            sub: "user-1".into(),
            iid: 1,
            role: UserRole::Admin,
            tenant_id: "default".to_string(),
            exp: (now - chrono::Duration::seconds(120)).timestamp() as usize,
            iat: (now - chrono::Duration::seconds(180)).timestamp() as usize,
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret("secret".as_bytes()),
        )
        .unwrap();
        let key = jsonwebtoken::DecodingKey::from_secret("secret".as_bytes());
        assert!(verify_token(&token, &key).is_err());
    }

    #[test]
    fn generate_test_token_is_valid() {
        let token = generate_access_token_for_test("user-1", 1, UserRole::Author);
        assert!(token.len() > 20);
    }
}
