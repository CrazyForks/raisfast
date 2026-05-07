//! OAuth 账号绑定模型与数据库查询
//!
//! 定义 `oauth_accounts` 和 `oauth_states` 表的数据结构和 CRUD 操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// OAuth 账号绑定记录
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct OAuthAccount {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub provider_user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<String>,
    pub profile: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// OAuth 短期 state 记录
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct OAuthState {
    pub id: String,
    pub provider: String,
    pub code_verifier: String,
    pub user_id: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

/// 创建 OAuth state 记录
pub async fn create_state(
    pool: &crate::db::Pool,
    id: &str,
    provider: &str,
    code_verifier: &str,
    user_id: Option<&str>,
    expires_at: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "INSERT INTO oauth_states (id, provider, code_verifier, user_id, created_at, expires_at) VALUES ({}, {}, {}, {}, {}, {})",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3),
        crate::db::dialect::ph(4),
        crate::db::dialect::ph(5),
        crate::db::dialect::ph(6)
    );
    let mut q = sqlx::query(&sql)
        .bind(id)
        .bind(provider)
        .bind(code_verifier);
    q = if let Some(uid) = user_id {
        q.bind(uid)
    } else {
        q.bind(Option::<String>::None)
    };
    q.bind(now).bind(expires_at).execute(pool).await?;
    Ok(())
}

/// 根据 state ID 查找并删除（一次性）
pub async fn consume_state(pool: &crate::db::Pool, id: &str) -> AppResult<Option<OAuthState>> {
    let sql = format!(
        "SELECT * FROM oauth_states WHERE id = {} AND expires_at > {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::now_fn(),
    );
    let state = sqlx::query_as::<_, OAuthState>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;

    if state.is_some() {
        let del_sql = format!(
            "DELETE FROM oauth_states WHERE id = {}",
            crate::db::dialect::ph(1)
        );
        sqlx::query(&del_sql).bind(id).execute(pool).await?;
    }

    Ok(state)
}

/// 清理过期的 OAuth state 记录
pub async fn cleanup_expired_states(pool: &crate::db::Pool) -> AppResult<u64> {
    let sql = format!(
        "DELETE FROM oauth_states WHERE expires_at <= {}",
        crate::db::dialect::now_fn(),
    );
    let result = sqlx::query(&sql).execute(pool).await?;
    Ok(result.rows_affected())
}

/// 根据 Provider + Provider 用户 ID 查找绑定
pub async fn find_by_provider_user(
    pool: &crate::db::Pool,
    provider: &str,
    provider_user_id: &str,
) -> AppResult<Option<OAuthAccount>> {
    let sql = format!(
        "SELECT * FROM oauth_accounts WHERE provider = {} AND provider_user_id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2)
    );
    let account = sqlx::query_as::<_, OAuthAccount>(&sql)
        .bind(provider)
        .bind(provider_user_id)
        .fetch_optional(pool)
        .await?;
    Ok(account)
}

/// 查找用户的所有 OAuth 绑定
pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: &str,
) -> AppResult<Vec<OAuthAccount>> {
    let sql = format!(
        "SELECT * FROM oauth_accounts WHERE user_id = {} ORDER BY created_at",
        crate::db::dialect::ph(1)
    );
    let accounts = sqlx::query_as::<_, OAuthAccount>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(accounts)
}

/// 创建 OAuth 账号绑定的参数
pub struct CreateOAuthAccountParams<'a> {
    pub user_id: &'a str,
    pub provider: &'a str,
    pub provider_user_id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub access_token: Option<&'a str>,
    pub refresh_token: Option<&'a str>,
    pub token_expires_at: Option<&'a str>,
    pub profile: Option<&'a str>,
}

/// 创建 OAuth 账号绑定
pub async fn create_account(
    pool: &crate::db::Pool,
    params: CreateOAuthAccountParams<'_>,
) -> AppResult<OAuthAccount> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();

    let sql = format!(
        "INSERT INTO oauth_accounts (id, user_id, provider, provider_user_id, email, display_name, avatar_url, access_token, refresh_token, token_expires_at, profile, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3),
        crate::db::dialect::ph(4),
        crate::db::dialect::ph(5),
        crate::db::dialect::ph(6),
        crate::db::dialect::ph(7),
        crate::db::dialect::ph(8),
        crate::db::dialect::ph(9),
        crate::db::dialect::ph(10),
        crate::db::dialect::ph(11),
        crate::db::dialect::ph(12),
        crate::db::dialect::ph(13)
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(params.user_id)
        .bind(params.provider)
        .bind(params.provider_user_id)
        .bind(params.email)
        .bind(params.display_name)
        .bind(params.avatar_url)
        .bind(params.access_token)
        .bind(params.refresh_token)
        .bind(params.token_expires_at)
        .bind(params.profile)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

    let sql2 = format!(
        "SELECT * FROM oauth_accounts WHERE id = {}",
        crate::db::dialect::ph(1)
    );
    let account = sqlx::query_as::<_, OAuthAccount>(&sql2)
        .bind(&id)
        .fetch_one(pool)
        .await?;

    Ok(account)
}

/// 更新 OAuth 账号绑定的参数
pub struct UpdateOAuthAccountParams<'a> {
    pub id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub access_token: Option<&'a str>,
    pub refresh_token: Option<&'a str>,
    pub token_expires_at: Option<&'a str>,
    pub profile: Option<&'a str>,
}

/// 更新 OAuth 账号绑定信息
pub async fn update_account(
    pool: &crate::db::Pool,
    params: UpdateOAuthAccountParams<'_>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "UPDATE oauth_accounts SET email = {}, display_name = {}, avatar_url = {}, access_token = {}, refresh_token = {}, token_expires_at = {}, profile = {}, updated_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3),
        crate::db::dialect::ph(4),
        crate::db::dialect::ph(5),
        crate::db::dialect::ph(6),
        crate::db::dialect::ph(7),
        crate::db::dialect::ph(8),
        crate::db::dialect::ph(9)
    );
    sqlx::query(&sql)
        .bind(params.email)
        .bind(params.display_name)
        .bind(params.avatar_url)
        .bind(params.access_token)
        .bind(params.refresh_token)
        .bind(params.token_expires_at)
        .bind(params.profile)
        .bind(now)
        .bind(params.id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除 OAuth 账号绑定（解绑）
pub async fn delete_account(
    pool: &crate::db::Pool,
    user_id: &str,
    provider: &str,
) -> AppResult<bool> {
    let sql = format!(
        "DELETE FROM oauth_accounts WHERE user_id = {} AND provider = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2)
    );
    let result = sqlx::query(&sql)
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 统计用户绑定的 OAuth Provider 数量
pub async fn count_by_user(pool: &crate::db::Pool, user_id: &str) -> AppResult<i64> {
    let sql = format!(
        "SELECT COUNT(*) FROM oauth_accounts WHERE user_id = {}",
        crate::db::dialect::ph(1)
    );
    let (count,) = sqlx::query_as::<_, (i64,)>(&sql)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(count)
}
