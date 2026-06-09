//! API Token business logic
//!
//! Provides services for creating, listing, deleting, and verifying API tokens.
//! Token format is `rblog_` + 64 hex characters; the plaintext is returned only once at creation time.

use crate::cache::CacheStore;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::api_token;
use crate::types::snowflake_id::{SnowflakeId, parse_id};
use crate::utils::tz::Timestamp;
#[cfg(feature = "export-types")]
use ts_rs::TS;

/// API Token prefix
const TOKEN_PREFIX: &str = "rblog_";

/// Cache key prefix
const CACHE_PREFIX: &str = "api_token:";

/// Cache TTL (seconds)
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// scope → role mapping
fn scope_to_role(scopes: &[String]) -> String {
    use crate::models::user::UserRole;
    if scopes.iter().any(|s| s == "admin") {
        UserRole::Admin.to_string()
    } else if scopes.iter().any(|s| s == "write") {
        UserRole::Author.to_string()
    } else {
        UserRole::Reader.to_string()
    }
}

/// Cached token authentication result
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedTokenAuth {
    user_id: SnowflakeId,
    role: String,
    tenant_id: Option<String>,
    expires_at: Option<Timestamp>,
}

/// Generate a plaintext token and its SHA-256 hash
fn generate_token() -> (String, String) {
    let raw = crate::utils::id::random_hex(32);
    let plain = format!("{TOKEN_PREFIX}{raw}");
    let hash = sha256_hex(plain.as_bytes());
    (plain, hash)
}

/// SHA-256 hex digest
fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let hash = <sha2::Sha256 as sha2::Digest>::digest(data);
    let mut hex = String::with_capacity(64);
    for byte in hash {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Compute the SHA-256 hash of a plaintext token
pub fn hash_token(plain: &str) -> String {
    sha256_hex(plain.as_bytes())
}

/// Check whether a string is an API token (starts with `rblog_`)
pub fn is_api_token(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX)
}

/// Result returned when creating an API token
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, serde::Serialize)]
pub struct CreateTokenResult {
    pub id: String,
    pub name: String,
    pub token: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// Create an API token
pub async fn create_token(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    name: &str,
    scopes: Vec<String>,
    expires_at: Option<&str>,
) -> AppResult<CreateTokenResult> {
    let _user_id = auth.ensure_snowflake_user_id()?;
    if name.trim().is_empty() {
        return Err(AppError::BadRequest("token name is required".into()));
    }
    if scopes.is_empty() {
        return Err(AppError::BadRequest(
            "at least one scope is required".into(),
        ));
    }
    for s in &scopes {
        if !["read", "write", "admin"].contains(&s.as_str()) {
            return Err(AppError::BadRequest(format!("invalid scope: {s}")));
        }
    }

    let (plain, hash) = generate_token();
    let prefix = &plain[..8];
    let scopes_json = serde_json::to_string(&scopes).unwrap_or_default();

    let user_id = auth.ensure_snowflake_user_id()?;

    let row = api_token::create(
        pool,
        user_id,
        name.trim(),
        &hash,
        prefix,
        &scopes_json,
        expires_at,
    )
    .await?;

    Ok(CreateTokenResult {
        id: row.id.to_string(),
        name: row.name,
        token: plain,
        token_prefix: row.token_prefix,
        scopes,
        expires_at: row.expires_at,
        created_at: row.created_at,
    })
}

/// List API tokens for the given user (masked)
pub async fn list_tokens(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<api_token::ApiTokenListItem>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let user = crate::models::user::find_by_id(pool, user_id, None)
        .await?
        .ok_or(AppError::Unauthorized)?;
    api_token::list_by_user(pool, user.id).await
}

/// Delete an API token (only the owner or an admin can do this)
pub async fn delete_token(
    pool: &crate::db::Pool,
    cache: &dyn CacheStore,
    token_id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let is_admin = auth.is_admin();
    let token = api_token::find_by_id(pool, parse_id(token_id)?)
        .await?
        .ok_or_else(|| AppError::NotFound("api_token".into()))?;
    let user = crate::models::user::find_by_id(pool, user_id, None)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !is_admin && token.user_id != user.id {
        return Err(AppError::Forbidden);
    }
    if let Err(e) = cache
        .delete(&format!("{CACHE_PREFIX}{}", token.token_hash))
        .await
    {
        tracing::warn!("api_token cache delete on revoke: {e}");
    }
    api_token::delete_by_id(pool, parse_id(token_id)?).await
}

/// Verify an API token and return (user_id, role, tenant_id)
///
/// Uses a cache-aside pattern: when the cache hits, all 3 DB operations are skipped.
/// Cache key is `api_token:{sha256_hash}`, TTL 300s.
pub async fn verify_api_token(
    pool: &crate::db::Pool,
    cache: &dyn CacheStore,
    plain: &str,
) -> AppResult<(i64, String, Option<String>)> {
    let hash = hash_token(plain);
    let cache_key = format!("{CACHE_PREFIX}{hash}");

    if let Some(cached) = cache.get(&cache_key).await
        && let Ok(auth) = serde_json::from_str::<CachedTokenAuth>(&cached)
    {
        if let Some(ref exp) = auth.expires_at
            && exp < &chrono::Utc::now()
        {
            let _ = cache.delete(&cache_key).await;
            return Err(AppError::Unauthorized);
        }
        return Ok((*auth.user_id, auth.role, auth.tenant_id));
    }

    let token = api_token::find_by_hash(pool, &hash)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if let Some(ref exp) = token.expires_at
        && exp < &chrono::Utc::now()
    {
        if let Err(e) = api_token::delete_by_id(pool, token.id).await {
            tracing::warn!("api_token delete expired: {e}");
        }
        if let Err(e) = cache.delete(&cache_key).await {
            tracing::warn!("api_token cache delete expired: {e}");
        }
        return Err(AppError::Unauthorized);
    }

    let user = crate::models::user::find_by_id(pool, token.user_id, None)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let scopes: Vec<String> = serde_json::from_str(&token.scopes).unwrap_or_default();
    let role = scope_to_role(&scopes);

    if let Err(e) = api_token::touch_last_used(pool, token.id).await {
        tracing::debug!("api_token touch_last_used: {e}");
    }

    let cached_auth = CachedTokenAuth {
        user_id: user.id,
        role: role.clone(),
        tenant_id: user.tenant_id.clone(),
        expires_at: token.expires_at,
    };
    if let Ok(json) = serde_json::to_string(&cached_auth)
        && let Err(e) = cache.set(&cache_key, &json, Some(CACHE_TTL)).await
    {
        tracing::debug!("api_token cache set: {e}");
    }

    Ok((*user.id, role, user.tenant_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn test_cache() -> std::sync::Arc<dyn crate::cache::CacheStore> {
        std::sync::Arc::new(crate::cache::MemoryCache::new())
    }

    async fn insert_user(
        pool: &crate::db::Pool,
        role: crate::models::user::UserRole,
    ) -> crate::models::user::User {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        crate::models::user::update_role(pool, user.id, role, None)
            .await
            .unwrap()
    }

    #[test]
    fn is_api_token_detects_prefix() {
        assert!(is_api_token("rblog_abc123"));
        assert!(is_api_token("rblog_"));
        assert!(!is_api_token("Bearer rblog_abc"));
        assert!(!is_api_token("eyJhbGciOiJIUzI1NiJ9.xxx"));
        assert!(!is_api_token(""));
    }

    #[test]
    fn hash_token_is_deterministic() {
        let h1 = hash_token("rblog_test123");
        let h2 = hash_token("rblog_test123");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_token_differs_for_different_inputs() {
        assert_ne!(hash_token("rblog_aaa"), hash_token("rblog_bbb"));
    }

    #[test]
    fn generate_token_format() {
        let (plain, hash) = generate_token();
        assert!(plain.starts_with("rblog_"));
        assert!(plain.len() > 10);
        assert!(plain[6..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn generate_token_unique() {
        let (p1, h1) = generate_token();
        let (p2, h2) = generate_token();
        assert_ne!(p1, p2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn scope_to_role_admin_wins() {
        assert_eq!(
            scope_to_role(&["read".into(), "write".into(), "admin".into()]),
            "admin"
        );
    }

    #[test]
    fn scope_to_role_write() {
        assert_eq!(scope_to_role(&["read".into(), "write".into()]), "author");
    }

    #[test]
    fn scope_to_role_read_only() {
        assert_eq!(scope_to_role(&["read".into()]), "reader");
    }

    #[test]
    fn scope_to_role_empty() {
        assert_eq!(scope_to_role(&[]), "reader");
    }

    #[tokio::test]
    async fn create_token_rejects_empty_name() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            1,
            crate::models::user::UserRole::Author,
            "default",
        );
        let msg = create_token(&pool, &auth, "   ", vec!["read".into()], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(msg.contains("name is required"));
    }

    #[tokio::test]
    async fn create_token_rejects_empty_scopes() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            1,
            crate::models::user::UserRole::Author,
            "default",
        );
        let msg = create_token(&pool, &auth, "Test", vec![], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(msg.contains("scope"));
    }

    #[tokio::test]
    async fn create_token_rejects_invalid_scope() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            1,
            crate::models::user::UserRole::Author,
            "default",
        );
        let msg = create_token(&pool, &auth, "Test", vec!["superuser".into()], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(msg.contains("invalid scope"));
    }

    #[tokio::test]
    async fn create_token_rejects_mixed_valid_invalid_scope() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            1,
            crate::models::user::UserRole::Author,
            "default",
        );
        let msg = create_token(
            &pool,
            &auth,
            "Test",
            vec!["read".into(), "delete".into()],
            None,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(msg.contains("invalid scope"));
    }

    #[tokio::test]
    async fn create_token_success_with_valid_scopes() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Author).await;
        let auth = crate::middleware::auth::AuthUser::from_parts(
            Some(*user.id),
            crate::models::user::UserRole::Author,
            Some("default".to_string()),
        );
        let result = create_token(
            &pool,
            &auth,
            "CI/CD",
            vec!["read".into(), "write".into()],
            None,
        )
        .await
        .unwrap();
        assert!(result.token.starts_with("rblog_"));
        assert_eq!(result.name, "CI/CD");
        assert_eq!(result.scopes, vec!["read", "write"]);
        assert!(result.expires_at.is_none());
        assert!(!result.id.is_empty());
    }

    #[tokio::test]
    async fn verify_api_token_expired_is_deleted() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let plain = "rblog_expired_token_for_test";
        let token_hash = hash_token(plain);
        let past = "2000-01-01T00:00:00+00:00";
        crate::models::api_token::create(
            &pool,
            user.id,
            "Expired",
            &token_hash,
            &plain[..8],
            "[\"read\"]",
            Some(past),
        )
        .await
        .unwrap();
        let cache = test_cache();
        assert!(verify_api_token(&pool, &*cache, plain).await.is_err());
        assert!(
            crate::models::api_token::find_by_hash(&pool, &token_hash)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn verify_api_token_not_expired_succeeds() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Admin).await;
        let plain = "rblog_valid_future_token_test";
        let token_hash = hash_token(plain);
        let future = "2099-12-31T23:59:59+00:00";
        crate::models::api_token::create(
            &pool,
            user.id,
            "Valid",
            &token_hash,
            &plain[..8],
            "[\"admin\"]",
            Some(future),
        )
        .await
        .unwrap();
        let cache = test_cache();
        let (uid, role, tenant_id) = verify_api_token(&pool, &*cache, plain).await.unwrap();
        assert_eq!(uid, *user.id);
        assert_eq!(role, "admin");
        assert_eq!(tenant_id, Some("default".to_string()));
        assert_eq!(tenant_id, Some("default".to_string()));
    }

    #[tokio::test]
    async fn verify_api_token_updates_last_used() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let plain = "rblog_last_used_test_token";
        let token_hash = hash_token(plain);
        crate::models::api_token::create(
            &pool,
            user.id,
            "TouchTest",
            &token_hash,
            &plain[..8],
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        assert!(
            crate::models::api_token::find_by_hash(&pool, &token_hash)
                .await
                .unwrap()
                .unwrap()
                .last_used_at
                .is_none()
        );
        let cache = test_cache();
        verify_api_token(&pool, &*cache, plain).await.unwrap();
        assert!(
            crate::models::api_token::find_by_hash(&pool, &token_hash)
                .await
                .unwrap()
                .unwrap()
                .last_used_at
                .is_some()
        );
    }

    #[tokio::test]
    async fn delete_token_by_owner_succeeds() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let row = crate::models::api_token::create(
            &pool,
            user.id,
            "Own",
            "h-del",
            "rblog_d",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            *user.id,
            crate::models::user::UserRole::Reader,
            "default",
        );
        let cache = test_cache();
        delete_token(&pool, &*cache, &row.id.to_string(), &auth)
            .await
            .unwrap();
        assert!(
            crate::models::api_token::find_by_id(&pool, row.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_token_by_admin_succeeds() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let admin = insert_user(&pool, crate::models::user::UserRole::Admin).await;
        let row = crate::models::api_token::create(
            &pool,
            user.id,
            "Target",
            "h-adm",
            "rblog_a",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            *admin.id,
            crate::models::user::UserRole::Admin,
            "default",
        );
        let cache = test_cache();
        delete_token(&pool, &*cache, &row.id.to_string(), &auth)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_token_non_owner_non_admin_forbidden() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let row = crate::models::api_token::create(
            &pool,
            user.id,
            "FB",
            "h-fb",
            "rblog_f",
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            99999,
            crate::models::user::UserRole::Reader,
            "default",
        );
        let cache = test_cache();
        assert!(
            delete_token(&pool, &*cache, &row.id.to_string(), &auth)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_token_nonexistent_not_found() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "default",
        );
        let cache = test_cache();
        assert!(
            delete_token(&pool, &*cache, "no-such-id", &auth)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn verify_api_token_cache_hit_skips_db() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let plain = "rblog_cache_hit_test_token";
        let token_hash = hash_token(plain);
        crate::models::api_token::create(
            &pool,
            user.id,
            "Cached",
            &token_hash,
            &plain[..8],
            "[\"read\"]",
            None,
        )
        .await
        .unwrap();

        let cache = test_cache();
        let (uid1, _, _) = verify_api_token(&pool, &*cache, plain).await.unwrap();
        assert_eq!(uid1, *user.id);

        let cache_key = format!("{CACHE_PREFIX}{token_hash}");
        assert!(cache.get(&cache_key).await.is_some());

        crate::models::api_token::delete_by_id(
            &pool,
            crate::models::api_token::find_by_hash(&pool, &token_hash)
                .await
                .unwrap()
                .unwrap()
                .id,
        )
        .await
        .unwrap();

        let (uid2, _, _) = verify_api_token(&pool, &*cache, plain).await.unwrap();
        assert_eq!(uid2, *user.id);
    }

    #[tokio::test]
    async fn verify_api_token_expired_cache_clears() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, crate::models::user::UserRole::Reader).await;
        let plain = "rblog_cache_expired_test";
        let token_hash = hash_token(plain);
        let past = "2000-01-01T00:00:00+00:00";
        crate::models::api_token::create(
            &pool,
            user.id,
            "CacheExp",
            &token_hash,
            &plain[..8],
            "[\"read\"]",
            Some(past),
        )
        .await
        .unwrap();

        let cache = test_cache();
        let cache_key = format!("{CACHE_PREFIX}{token_hash}");
        let cached = serde_json::to_string(&CachedTokenAuth {
            user_id: user.id,
            role: "reader".into(),
            tenant_id: Some("default".to_string()),
            expires_at: Some(past.parse().unwrap()),
        })
        .unwrap();
        cache
            .set(&cache_key, &cached, Some(CACHE_TTL))
            .await
            .unwrap();

        assert!(verify_api_token(&pool, &*cache, plain).await.is_err());
        assert!(cache.get(&cache_key).await.is_none());
    }
}
