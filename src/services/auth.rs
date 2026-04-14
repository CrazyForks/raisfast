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

use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::handlers::dto::{
    LoginResponse, RegisterRequest, UpdatePasswordRequest, UpdateUserRequest, UserResponse,
};
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::{RefreshTokenRepository, UserRepository};

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

/// 测试辅助：使用固定 secret 生成 JWT token。
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

/// 用户注册。
///
/// 检查邮箱是否已被注册，若唯一则哈希密码并创建用户记录。
pub async fn register(
    user_repo: &dyn UserRepository,
    eventbus: &EventBus,
    req: RegisterRequest,
) -> AppResult<UserResponse> {
    if user_repo.find_by_email(&req.email).await?.is_some() {
        return Err(AppError::Conflict("email_registered".into()));
    }

    let password_hash = hash_password(&req.password)?;
    let user = user_repo
        .create(CreateUserCmd {
            email: req.email,
            username: req.username,
            password_hash,
        })
        .await?;
    eventbus.emit(Event::UserRegistered {
        id: user.id.clone(),
        username: user.username.clone(),
        email: user.email.clone(),
    });
    Ok(user.into())
}

/// 用户登录。
///
/// 验证邮箱和密码，成功后生成访问令牌和刷新令牌，将刷新令牌存入数据库。
#[allow(clippy::too_many_arguments)]
pub async fn login(
    user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    req: &crate::handlers::dto::LoginRequest,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let user = user_repo
        .find_by_email(&req.email)
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

    let access_token = generate_access_token(&user.id, &user.role, jwt_secret, jwt_access_expires)?;
    let refresh_token_str = generate_refresh_token_string()?;

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
pub async fn refresh(
    user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    pool: &crate::db::Pool,
    refresh_token_str: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
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
        .find_by_id(&stored.user_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized)?;

    let access_token = generate_access_token(&user.id, &user.role, jwt_secret, jwt_access_expires)?;
    let new_refresh_token = generate_refresh_token_string()?;
    let new_expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);
    let new_expires_str = new_expires_at.to_rfc3339();
    let new_id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await?;

    sqlx::query!(
        "DELETE FROM refresh_tokens WHERE token = ?",
        refresh_token_str,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO refresh_tokens (id, user_id, token, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        new_id,
        user.id,
        new_refresh_token,
        new_expires_str,
        now,
    )
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
    user_id: &str,
) -> AppResult<()> {
    refresh_token_repo.delete_by_user(user_id).await
}

/// 获取当前用户资料。
pub async fn get_me(user_repo: &dyn UserRepository, user_id: &str) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;
    Ok(user.into())
}

/// 更新当前用户资料（用户名、简介、网站、头像）。
pub async fn update_me(
    user_repo: &dyn UserRepository,
    user_id: &str,
    req: UpdateUserRequest,
) -> AppResult<UserResponse> {
    let user = user_repo
        .update_profile(UpdateProfileCmd {
            id: user_id.to_string(),
            username: req.username,
            bio: req.bio,
            website: req.website,
            avatar: req.avatar,
        })
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
    user_id: &str,
    req: UpdatePasswordRequest,
) -> AppResult<()> {
    let user = user_repo
        .find_by_id(user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;

    if !verify_password(&req.old_password, &user.password_hash)? {
        return Err(AppError::BadRequest("incorrect_password".into()));
    }

    let new_hash = hash_password(&req.new_password)?;
    let now = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await?;

    sqlx::query!(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
        new_hash,
        now,
        user_id,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!("DELETE FROM refresh_tokens WHERE user_id = ?", user_id,)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// 获取指定用户的公开资料。
pub async fn get_public_user(user_repo: &dyn UserRepository, id: &str) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;
    Ok(user.into())
}

/// 分页查询用户列表。
///
/// 返回用户响应列表和总记录数。
pub async fn list_users(
    user_repo: &dyn UserRepository,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<UserResponse>, i64)> {
    let (users, total) = user_repo.find_all(page, page_size).await?;
    let responses = users.into_iter().map(UserResponse::from).collect();
    Ok((responses, total))
}
