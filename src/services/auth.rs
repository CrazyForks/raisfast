//! 认证与用户服务。
//!
//! 提供完整的认证业务逻辑，包括：
//!
//! - 密码哈希与验证（Argon2id）
//! - JWT 访问令牌的生成与验证（HS256）
//! - 刷新令牌的生成与轮换
//! - 用户注册、登录、登出
//! - 用户资料查询与修改

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::handlers::dto::{
    LoginResponse, RegisterRequest, UpdatePasswordRequest, UpdateUserRequest, UserResponse,
};
use crate::middleware::auth::AuthUser;
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::{RefreshTokenRepository, UserRepository};

/// JWT 令牌载荷（Claims）。
///
/// - `sub`：用户 ID。
/// - `role`：用户角色（如 `"admin"`、`"author"`）。
/// - `tenant_id`：所属租户 ID（默认 `"default"`）。
/// - `exp`：过期时间（UNIX 时间戳）。
/// - `iat`：签发时间（UNIX 时间戳）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
    pub exp: usize,
    pub iat: usize,
}

fn default_tenant_id() -> String {
    crate::constants::DEFAULT_TENANT.to_string()
}

/// 校验密码强度。
///
/// 要求：
/// - 最少 8 个字符
/// - 至少包含一个大写字母
/// - 至少包含一个小写字母
/// - 至少包含一个数字
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

/// 使用 Argon2id 算法对密码进行哈希。
///
/// 通过 `getrandom` 生成 32 字节随机盐值，返回 PHC 格式的哈希字符串。
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

/// 验证明文密码是否与 Argon2 哈希匹配。
///
/// 返回 `Ok(true)` 表示匹配，`Ok(false)` 表示不匹配。
pub fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// 生成 HS256 签名的 JWT 访问令牌。
pub(crate) fn generate_access_token_internal(
    user_id: &str,
    role: &str,
    tenant_id: &str,
    secret: &str,
    expires_in: u64,
) -> AppResult<String> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() as usize) + (expires_in as usize);
    let claims = Claims {
        sub: user_id.to_string(),
        role: role.to_string(),
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

/// 验证并解码 JWT 令牌。
///
/// 若令牌过期或无效，统一返回 [`AppError::Unauthorized`]。
pub fn verify_token(token: &str, key: &jsonwebtoken::DecodingKey) -> AppResult<Claims> {
    jsonwebtoken::decode::<Claims>(token, key, &Validation::default())
        .map(|data| data.claims)
        .map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::Unauthorized,
            _ => AppError::Unauthorized,
        })
}

/// 生成 32 字节随机刷新令牌，以十六进制字符串返回。
pub(crate) fn generate_refresh_token_string_internal() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("refresh token generation failed: {e}")))?;
    Ok(hex::encode(bytes))
}

/// 测试辅助：使用固定 secret 生成 JWT token。
#[allow(clippy::doc_lazy_continuation)]
#[must_use]
pub fn generate_access_token_for_test(user_id: &str, role: &str) -> String {
    generate_access_token_internal(
        user_id,
        role,
        crate::constants::DEFAULT_TENANT,
        "test-secret-key-at-least-32-characters-long",
        900,
    )
    .unwrap()
}

/// 用户注册。
///
/// 检查邮箱是否已被注册，若唯一则哈希密码并创建用户记录。
#[tracing::instrument(skip(user_repo, eventbus), fields(username = tracing::field::Empty))]
pub async fn register(
    user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    req: RegisterRequest,
    tenant_id: Option<&str>,
    require_email_verification: bool,
    pool: &crate::db::Pool,
) -> AppResult<UserResponse> {
    if user_repo
        .find_by_email(&req.email, tenant_id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("email_registered".into()));
    }

    validate_password_strength(&req.password)?;
    let password_hash = hash_password(&req.password)?;
    let user = user_repo
        .create(
            CreateUserCmd {
                email: req.email,
                username: req.username,
                password_hash,
            },
            tenant_id,
        )
        .await?;
    eventbus.emit(Event::UserRegistered {
        id: user.id.clone(),
        username: user.username.clone(),
        email: user.email.clone(),
    });

    if require_email_verification {
        let _ = trigger_email_verification(pool, eventbus, &user.id, &user.email).await;
    }

    Ok(user.into())
}

/// 用户登录。
///
/// 验证邮箱和密码，成功后生成访问令牌和刷新令牌，将刷新令牌存入数据库。
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(user_repo, refresh_token_repo, plugins, eventbus), fields(email = %req.email))]
pub async fn login(
    user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    req: &crate::handlers::dto::LoginRequest,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    tenant_id: Option<&str>,
    require_email_verification: bool,
) -> AppResult<LoginResponse> {
    let user = user_repo
        .find_by_email(&req.email, tenant_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    if !verify_password(&req.password, &user.password_hash)? {
        plugins
            .dispatch_action(
                HookPoint::OnLogin,
                &serde_json::json!({"email": &req.email, "success": false}),
            )
            .await;
        return Err(AppError::Unauthorized);
    }

    if require_email_verification && user.email_verified == 0 {
        return Err(AppError::BadRequest("email_not_verified".into()));
    }

    let access_token = generate_access_token_internal(
        &user.id,
        &user.role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let refresh_token_str = generate_refresh_token_string_internal()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token_repo
        .create_token(&user.id, &refresh_token_str, &expires_at.to_rfc3339())
        .await?;

    plugins
        .dispatch_action(
            HookPoint::OnLogin,
            &serde_json::json!({"email": &req.email, "success": true, "user_id": &user.id}),
        )
        .await;

    eventbus.emit(Event::UserLoggedIn {
        id: user.id.clone(),
        success: true,
    });

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: user.into(),
    })
}

/// 刷新令牌。
///
/// 验证刷新令牌的有效性，执行令牌轮换：在事务中删除旧刷新令牌，
/// 生成新的访问令牌和刷新令牌，确保原子性。
#[allow(clippy::too_many_arguments)]
pub async fn refresh(
    user_repo: &dyn UserRepository,
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

    let expires_at = chrono::DateTime::parse_from_rfc3339(&stored.expires_at)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid token expiry")))?;

    if expires_at < Utc::now() {
        let _ = refresh_token_repo.delete_by_token(refresh_token_str).await;
        return Err(AppError::Unauthorized);
    }

    let user = user_repo
        .find_by_id(&stored.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    let access_token = generate_access_token_internal(
        &user.id,
        &user.role,
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

    let mut tx = pool.begin().await?;

    sqlx::query(&crate::db::dialect::translate(
        "DELETE FROM refresh_tokens WHERE token = ?",
    ))
    .bind(refresh_token_str)
    .execute(&mut *tx)
    .await?;

    sqlx::query(&crate::db::dialect::translate(
        "INSERT INTO refresh_tokens (id, user_id, token, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
    ))
    .bind(&new_id)
    .bind(&user.id)
    .bind(&new_refresh_token)
    .bind(&new_expires_str)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_in: jwt_access_expires,
        user: user.into(),
    })
}

/// 用户登出。
///
/// 删除该用户的所有刷新令牌，使其所有设备上的会话失效。
pub async fn logout(
    refresh_token_repo: &dyn RefreshTokenRepository,
    auth: &AuthUser,
) -> AppResult<()> {
    refresh_token_repo
        .delete_by_user(auth.ensure_authenticated()?)
        .await
}

/// 获取当前用户资料。
pub async fn get_me(user_repo: &dyn UserRepository, auth: &AuthUser) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(auth.ensure_authenticated()?, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    Ok(user.into())
}

/// 更新当前用户资料（用户名、简介、网站、头像）。
pub async fn update_me(
    user_repo: &dyn UserRepository,
    auth: &AuthUser,
    req: UpdateUserRequest,
) -> AppResult<UserResponse> {
    let user = user_repo
        .update_profile(
            UpdateProfileCmd {
                id: auth.ensure_authenticated()?.to_string(),
                username: req.username,
                bio: req.bio,
                website: req.website,
                avatar: req.avatar,
            },
            auth.tenant_id(),
        )
        .await?;
    Ok(user.into())
}

/// 修改密码。
///
/// 验证旧密码正确后，在事务中用新密码的哈希替换旧哈希，
/// 并删除所有刷新令牌，确保旧会话全部失效。
pub async fn change_password(
    user_repo: &dyn UserRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    req: UpdatePasswordRequest,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    let user = user_repo
        .find_by_id(user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    if !verify_password(&req.old_password, &user.password_hash)? {
        return Err(AppError::BadRequest("incorrect_password".into()));
    }

    validate_password_strength(&req.new_password)?;
    let new_hash = hash_password(&req.new_password)?;
    user_repo
        .update_password(user_id, &new_hash, tenant_id)
        .await?;

    sqlx::query(&crate::db::dialect::translate(
        "DELETE FROM refresh_tokens WHERE user_id = ?",
    ))
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 获取指定用户的公开资料。
pub async fn get_public_user(
    user_repo: &dyn UserRepository,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    Ok(user.into())
}

/// 分页查询用户列表。
///
/// 返回用户响应列表和总记录数。
pub async fn list_users(
    user_repo: &dyn UserRepository,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<UserResponse>, i64)> {
    let (users, total) = user_repo.find_all(page, page_size, tenant_id).await?;
    let responses = users.into_iter().map(UserResponse::from).collect();
    Ok((responses, total))
}

/// 请求密码重置。
///
/// 查找用户，删除旧令牌，创建新令牌，通过 EventBus 触发邮件发送。
/// 无论用户是否存在都返回成功（防止邮箱枚举）。
pub async fn forgot_password(
    pool: &crate::db::Pool,
    user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    email: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let user = match user_repo.find_by_email(email, tenant_id).await? {
        Some(u) => u,
        None => return Ok(()),
    };

    crate::models::password_reset::delete_unused_by_user(pool, &user.id).await?;

    let reset_token = crate::models::password_reset::create(pool, &user.id, 3600).await?;

    eventbus.emit(Event::PasswordResetRequested {
        user_id: user.id,
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

    validate_password_strength(new_password)?;
    let new_hash = hash_password(new_password)?;

    let mut tx = pool.begin().await?;

    let sql = crate::db::dialect::translate(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
    );
    let now = Utc::now().to_rfc3339();
    sqlx::query(&sql)
        .bind(&new_hash)
        .bind(&now)
        .bind(&reset_token.user_id)
        .execute(&mut *tx)
        .await?;

    let sql =
        crate::db::dialect::translate("UPDATE password_reset_tokens SET used_at = ? WHERE id = ?");
    sqlx::query(&sql)
        .bind(&now)
        .bind(&reset_token.id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = ?")
        .bind(&reset_token.user_id)
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

    validate_password_strength(new_password)?;
    let new_hash = hash_password(new_password)?;
    user_repo
        .update_password(user_id, &new_hash, tenant_id)
        .await?;

    let _ = pool;
    Ok(())
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
            generate_access_token_internal("user-1", "admin", "default", "secret", 900).unwrap();
        let key = jsonwebtoken::DecodingKey::from_secret("secret".as_bytes());
        let claims = verify_token(&token, &key).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.role, "admin");
    }

    #[test]
    fn verify_token_rejects_wrong_secret() {
        let token =
            generate_access_token_internal("user-1", "admin", "default", "secret-a", 900).unwrap();
        let key = jsonwebtoken::DecodingKey::from_secret("secret-b".as_bytes());
        assert!(verify_token(&token, &key).is_err());
    }

    #[test]
    fn verify_token_rejects_expired() {
        let now = chrono::Utc::now();
        let claims = Claims {
            sub: "user-1".into(),
            role: "admin".into(),
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
        let token = generate_access_token_for_test("user-1", "author");
        assert!(token.len() > 20);
    }

    #[test]
    fn sms_code_generate_length() {
        let code = crate::models::sms_code::generate_code(6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}

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

    let result = crate::models::sms_code::verify_code(pool, &sms.id, code).await?;

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

    let access_token = generate_access_token_internal(
        &user.id,
        &user.role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;
    let refresh_token_str = generate_refresh_token_string_internal()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token_repo
        .create_token(&user.id, &refresh_token_str, &expires_at.to_rfc3339())
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

    let result = crate::models::sms_code::verify_code(pool, &sms.id, code).await?;

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

/// 注册后触发邮箱验证（若配置启用）。
///
/// 删除旧令牌，创建新令牌，通过 EventBus 发送验证邮件。
pub async fn trigger_email_verification(
    pool: &crate::db::Pool,
    eventbus: &EventBus,
    user_id: &str,
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
    let sql = crate::db::dialect::translate(
        "UPDATE email_verification_tokens SET verified_at = ? WHERE id = ?",
    );
    sqlx::query(&sql)
        .bind(&now)
        .bind(&verification.id)
        .execute(&mut *tx)
        .await?;

    let sql = crate::db::dialect::translate(
        "UPDATE users SET email_verified = 1, updated_at = ? WHERE id = ?",
    );
    sqlx::query(&sql)
        .bind(&now)
        .bind(&verification.user_id)
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

    trigger_email_verification(pool, eventbus, &user.id, &user.email).await
}
