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
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::models::refresh_token;
use crate::models::user::{
    self, LoginResponse, RegisterRequest, UpdatePasswordRequest, UpdateUserRequest, UserResponse,
};

/// JWT 令牌载荷（Claims）。
///
/// - `sub`：用户 ID。
/// - `role`：用户角色（如 `"admin"`、`"author"`）。
/// - `exp`：过期时间（UNIX 时间戳）。
/// - `iat`：签发时间（UNIX 时间戳）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
}

/// 使用 Argon2id 算法对密码进行哈希。
///
/// 通过 `getrandom` 生成 32 字节随机盐值，返回 PHC 格式的哈希字符串。
pub fn hash_password(password: &str) -> AppResult<String> {
    let mut salt_bytes = [0u8; 32];
    getrandom::getrandom(&mut salt_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("salt generation failed: {}", e)))?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("salt encoding failed: {}", e)))?;
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("password hashing failed: {}", e)))?;
    Ok(hash.to_string())
}

/// 验证明文密码是否与 Argon2 哈希匹配。
///
/// 返回 `Ok(true)` 表示匹配，`Ok(false)` 表示不匹配。
pub fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid password hash: {}", e)))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// 生成 HS256 签名的 JWT 访问令牌。
///
/// # 参数
///
/// - `user_id`：用户 ID，将作为 `sub` 字段。
/// - `role`：用户角色，将作为 `role` 字段。
/// - `secret`：JWT 签名密钥。
/// - `expires_in`：有效期（秒）。
/// 测试辅助：使用固定 secret 生成 JWT token。
///
/// 仅用于集成测试，不应在生产代码中调用。
#[allow(clippy::doc_lazy_continuation)]
pub fn generate_access_token_for_test(user_id: &str, role: &str) -> String {
    generate_access_token(
        user_id,
        role,
        "test-secret-key-at-least-32-characters-long",
        900,
    )
    .unwrap()
}

fn generate_access_token(
    user_id: &str,
    role: &str,
    secret: &str,
    expires_in: u64,
) -> AppResult<String> {
    let now = Utc::now();
    let iat = now.timestamp() as usize;
    let exp = (now.timestamp() as usize) + (expires_in as usize);
    let claims = Claims {
        sub: user_id.to_string(),
        role: role.to_string(),
        exp,
        iat,
    };
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("token encoding failed: {}", e)))
}

/// 验证并解码 JWT 令牌。
///
/// 若令牌过期或无效，统一返回 [`AppError::Unauthorized`]。
pub fn verify_token(token: &str, secret: &str) -> AppResult<Claims> {
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::Unauthorized,
        _ => AppError::Unauthorized,
    })
}

/// 生成 32 字节随机刷新令牌，以十六进制字符串返回。
fn generate_refresh_token_string() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("refresh token generation failed: {}", e))
    })?;
    Ok(hex::encode(bytes))
}

/// 用户注册。
///
/// 检查邮箱是否已被注册，若唯一则哈希密码并创建用户记录。
pub async fn register(pool: &crate::db::Pool, req: RegisterRequest) -> AppResult<UserResponse> {
    if user::find_by_email(pool, &req.email).await?.is_some() {
        return Err(AppError::Conflict("email_registered".into()));
    }

    let password_hash = hash_password(&req.password)?;
    let user = user::create(pool, &req.email, &req.username, &password_hash).await?;
    Ok(user.into())
}

/// 用户登录。
///
/// 验证邮箱和密码，成功后生成访问令牌和刷新令牌，将刷新令牌存入数据库。
pub async fn login(
    pool: &crate::db::Pool,
    req: &crate::models::user::LoginRequest,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let user = user::find_by_email(pool, &req.email)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    if !verify_password(&req.password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }

    let access_token = generate_access_token(&user.id, &user.role, jwt_secret, jwt_access_expires)?;
    let refresh_token_str = generate_refresh_token_string()?;

    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token::create_token(pool, &user.id, &refresh_token_str, &expires_at.to_rfc3339())
        .await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: user.into(),
    })
}

/// 刷新令牌。
///
/// 验证刷新令牌的有效性，执行令牌轮换：删除旧刷新令牌，生成新的访问令牌和刷新令牌。
pub async fn refresh(
    pool: &crate::db::Pool,
    refresh_token_str: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let stored = refresh_token::find_by_token(pool, refresh_token_str)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    let expires_at = chrono::DateTime::parse_from_rfc3339(&stored.expires_at)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid token expiry")))?;

    if expires_at < Utc::now() {
        let _ = refresh_token::delete_by_token(pool, refresh_token_str).await;
        return Err(AppError::Unauthorized);
    }

    let user = user::find_by_id(pool, &stored.user_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    refresh_token::delete_by_token(pool, refresh_token_str).await?;

    let access_token = generate_access_token(&user.id, &user.role, jwt_secret, jwt_access_expires)?;
    let new_refresh_token = generate_refresh_token_string()?;
    let new_expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    refresh_token::create_token(
        pool,
        &user.id,
        &new_refresh_token,
        &new_expires_at.to_rfc3339(),
    )
    .await?;

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
pub async fn logout(pool: &crate::db::Pool, user_id: &str) -> AppResult<()> {
    refresh_token::delete_by_user(pool, user_id).await
}

/// 获取当前用户资料。
pub async fn get_me(pool: &crate::db::Pool, user_id: &str) -> AppResult<UserResponse> {
    let user = user::find_by_id(pool, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;
    Ok(user.into())
}

/// 更新当前用户资料（用户名、简介、网站、头像）。
pub async fn update_me(
    pool: &crate::db::Pool,
    user_id: &str,
    req: UpdateUserRequest,
) -> AppResult<UserResponse> {
    let user = user::update_profile(
        pool,
        user_id,
        req.username.as_deref(),
        req.bio.as_deref(),
        req.website.as_deref(),
        req.avatar.as_deref(),
    )
    .await?;
    Ok(user.into())
}

/// 修改密码。
///
/// 验证旧密码正确后，用新密码的哈希替换旧哈希。
pub async fn change_password(
    pool: &crate::db::Pool,
    user_id: &str,
    req: UpdatePasswordRequest,
) -> AppResult<()> {
    let user = user::find_by_id(pool, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;

    if !verify_password(&req.old_password, &user.password_hash)? {
        return Err(AppError::BadRequest("incorrect_password".into()));
    }

    let new_hash = hash_password(&req.new_password)?;
    user::update_password(pool, user_id, &new_hash).await?;
    refresh_token::delete_by_user(pool, user_id).await?;
    Ok(())
}

/// 获取指定用户的公开资料。
pub async fn get_public_user(pool: &crate::db::Pool, id: &str) -> AppResult<UserResponse> {
    let user = user::find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;
    Ok(user.into())
}

/// 分页查询用户列表。
///
/// 返回用户响应列表和总记录数。
pub async fn list_users(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<UserResponse>, i64)> {
    let (users, total) = user::find_all(pool, page, page_size).await?;
    let responses = users.into_iter().map(UserResponse::from).collect();
    Ok((responses, total))
}
