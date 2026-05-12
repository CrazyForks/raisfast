//! API Token 模型与数据库查询
//!
//! 定义 API Token（长期访问令牌）的数据结构以及对 `api_tokens` 表的
//! 创建、查找、删除操作。令牌以 SHA-256 哈希存储，创建时只返回一次明文。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// API Token 完整数据库行模型
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub scopes: String,
    pub last_used_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// API Token 列表项（脱敏，不含 token_hash）
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct ApiTokenListItem {
    pub id: i64,
    pub document_id: String,
    pub name: String,
    pub token_prefix: String,
    pub scopes: String,
    pub last_used_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// 创建新的 API Token 记录
pub async fn create(
    pool: &crate::db::Pool,
    user_id: i64,
    name: &str,
    token_hash: &str,
    token_prefix: &str,
    scopes: &str,
    expires_at: Option<&str>,
) -> AppResult<ApiToken> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO api_tokens (document_id, user_id, name, token_hash, token_prefix, scopes, expires_at, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(user_id)
        .bind(name)
        .bind(token_hash)
        .bind(token_prefix)
        .bind(scopes)
        .bind(expires_at)
        .bind(now)
        .execute(pool)
        .await?;
    let sql = format!("SELECT * FROM api_tokens WHERE document_id = {}", ph(1));
    let row = sqlx::query_as::<_, ApiToken>(&sql)
        .bind(&document_id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// 根据 token_hash 查找 API Token
pub async fn find_by_hash(pool: &crate::db::Pool, token_hash: &str) -> AppResult<Option<ApiToken>> {
    let sql = format!("SELECT * FROM api_tokens WHERE token_hash = {}", ph(1));
    let row = sqlx::query_as::<_, ApiToken>(&sql)
        .bind(token_hash)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 列出指定用户的所有 API Token（脱敏）
pub async fn list_by_user(
    pool: &crate::db::Pool,
    user_id: i64,
) -> AppResult<Vec<ApiTokenListItem>> {
    let sql = format!(
        "SELECT id, document_id, name, token_prefix, scopes, last_used_at, expires_at, created_at FROM api_tokens WHERE user_id = {} ORDER BY created_at DESC",
        ph(1)
    );
    let rows = sqlx::query_as::<_, ApiTokenListItem>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 根据 document_id 查找 API Token
pub async fn find_by_id(pool: &crate::db::Pool, document_id: &str) -> AppResult<Option<ApiToken>> {
    let sql = format!("SELECT * FROM api_tokens WHERE document_id = {}", ph(1));
    let row = sqlx::query_as::<_, ApiToken>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 根据 document_id 删除 API Token
pub async fn delete_by_id(pool: &crate::db::Pool, document_id: &str) -> AppResult<()> {
    let sql = format!("DELETE FROM api_tokens WHERE document_id = {}", ph(1));
    sqlx::query(&sql).bind(document_id).execute(pool).await?;
    Ok(())
}

/// 更新 last_used_at（按整数主键）
pub async fn touch_last_used(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE api_tokens SET last_used_at = {} WHERE id = {}",
        ph(1),
        ph(2)
    );
    sqlx::query(&sql).bind(now).bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            None,
        )
        .await
        .unwrap();
        user.id
    }

    #[tokio::test]
    async fn create_and_find_by_hash() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(
            &pool,
            user_id,
            "Test",
            "hash123",
            "rblog_ab",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        assert_eq!(row.name, "Test");
        assert_eq!(row.token_hash, "hash123");
        assert_eq!(row.token_prefix, "rblog_ab");
        assert_eq!(row.scopes, "[\"read\"]");
        assert!(row.expires_at.is_none());

        let found = find_by_hash(&pool, "hash123").await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
    }

    #[tokio::test]
    async fn find_by_hash_not_found() {
        let pool = setup_pool().await;
        let result = find_by_hash(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        let result = find_by_id(&pool, "nonexistent-doc-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_by_user_returns_tokens() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create(&pool, user_id, "First", "h1", "rblog_a", "[\"read\"]", None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        create(
            &pool,
            user_id,
            "Second",
            "h2",
            "rblog_b",
            "[\"write\"]",
            None,
        )
        .await
        .unwrap();

        let list = list_by_user(&pool, user_id).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Second");
        assert_eq!(list[1].name, "First");
    }

    #[tokio::test]
    async fn list_by_user_empty() {
        let pool = setup_pool().await;
        let list = list_by_user(&pool, 99999).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn delete_by_id_removes_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(&pool, user_id, "Del", "h3", "rblog_c", "[\"read\"]", None)
            .await
            .unwrap();
        delete_by_id(&pool, &row.document_id).await.unwrap();
        let found = find_by_id(&pool, &row.document_id).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn touch_last_used_updates_field() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(&pool, user_id, "Touch", "h4", "rblog_d", "[\"read\"]", None)
            .await
            .unwrap();
        assert!(row.last_used_at.is_none());

        touch_last_used(&pool, row.id).await.unwrap();
        let updated = find_by_id(&pool, &row.document_id).await.unwrap().unwrap();
        assert!(updated.last_used_at.is_some());
    }

    #[tokio::test]
    async fn create_with_expires_at() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(
            &pool,
            user_id,
            "Expiring",
            "h5",
            "rblog_e",
            "[\"admin\"]",
            Some("2099-12-31T00:00:00+00:00"),
        )
        .await
        .unwrap();
        assert_eq!(
            row.expires_at.unwrap(),
            "2099-12-31T00:00:00+00:00".parse::<Timestamp>().unwrap()
        );
    }

    #[tokio::test]
    async fn list_by_user_does_not_include_hash() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create(
            &pool,
            user_id,
            "Safe",
            "secret_hash",
            "rblog_f",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        let list = list_by_user(&pool, user_id).await.unwrap();
        let json = serde_json::to_value(&list[0]).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("token_hash"));
        assert!(!obj.contains_key("user_id"));
    }
}
