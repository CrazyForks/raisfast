use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Currency {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub decimals: i64,
    pub is_active: bool,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_code(
    pool: &crate::db::Pool,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Currency>> {
    raisfast_derive::crud_find!(pool, "currencies", Currency, where: ("code", code), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_active_by_code(
    pool: &crate::db::Pool,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Currency>> {
    raisfast_derive::crud_find!(pool, "currencies", Currency, where: AND(("code", code), ("is_active", 1i64)), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn ensure_active(
    pool: &crate::db::Pool,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<Currency> {
    find_active_by_code(pool, code, tenant_id)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::BadRequest(format!("currency_not_active: {code}"))
        })
}

pub async fn find_by_code_tx(
    tx: &mut crate::db::pool::DbConnection,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Currency>> {
    raisfast_derive::crud_find!(tx, "currencies", Currency, where: AND(("code", code), ("is_active", 1i64)), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<Currency>> {
    raisfast_derive::check_schema!(
        "currencies",
        "id",
        "tenant_id",
        "code",
        "name",
        "decimals",
        "is_active",
        "version",
        "created_at",
        "updated_at"
    );
    raisfast_derive::crud_list!(pool, "currencies", Currency, order_by: "code", tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn create(
    pool: &crate::db::Pool,
    tenant_id: &str,
    code: &str,
    name: &str,
    decimals: i64,
) -> AppResult<Currency> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "currencies",
        [
            "id" => id,
            "tenant_id" => tenant_id,
            "code" => code,
            "name" => name,
            "decimals" => decimals,
            "created_at" => now,
            "updated_at" => now
        ]
    )?;

    raisfast_derive::crud_find_one!(pool, "currencies", Currency, where: ("id", id), tenant: Some(tenant_id))
        .map_err(Into::into)
}

pub async fn update(
    pool: &crate::db::Pool,
    code: &str,
    name: Option<&str>,
    is_active: Option<bool>,
    tenant_id: Option<&str>,
) -> AppResult<Option<Currency>> {
    raisfast_derive::check_schema!(
        "currencies",
        "id",
        "tenant_id",
        "code",
        "name",
        "is_active",
        "version",
        "updated_at"
    );
    let existing = find_by_code(pool, code, tenant_id).await?;
    let existing = match existing {
        Some(e) => e,
        None => return Ok(None),
    };

    let name = name.unwrap_or(&existing.name);
    let is_active = is_active.unwrap_or(existing.is_active);
    let now = crate::utils::tz::now_str();

    let result = raisfast_derive::crud_update!(pool, "currencies",
        bind: ["name" => name, "is_active" => is_active, "updated_at" => now],
        raw: ["version" => "version + 1"],
        where: AND(("id", existing.id), ("version", existing.version))
    )?;
    let affected = result.rows_affected();

    if affected == 0 {
        return Err(crate::errors::app_error::AppError::Conflict(
            "concurrent_currency_update".into(),
        ));
    }

    find_by_code(pool, code, tenant_id).await
}

pub async fn delete_by_code(
    pool: &crate::db::Pool,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let existing = find_by_code(pool, code, tenant_id).await?;
    let existing = match existing {
        Some(e) => e,
        None => return Ok(false),
    };

    let count: i64 = raisfast_derive::crud_count!(pool, "wallets", where: ("currency", code), tenant: tenant_id)?;

    if count > 0 {
        return Err(crate::errors::app_error::AppError::BadRequest(format!(
            "currency_in_use: {count} wallet(s) using '{code}'"
        )));
    }

    let result = raisfast_derive::crud_delete!(pool, "currencies", where: ("id", existing.id))?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    #[tokio::test]
    async fn create_and_find_currency() {
        let pool = setup_pool().await;
        let c = create(&pool, "test_tenant", "CNY", "Chinese Yuan", 2)
            .await
            .unwrap();
        assert_eq!(c.code, "CNY");
        assert_eq!(c.name, "Chinese Yuan");
        assert_eq!(c.decimals, 2);
        assert!(c.is_active);

        let found = find_by_code(&pool, "CNY", Some("test_tenant"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, c.id);
    }

    #[tokio::test]
    async fn find_nonexistent_returns_none() {
        let pool = setup_pool().await;
        assert!(
            find_by_code(&pool, "XXX", Some("test_tenant"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn update_currency() {
        let pool = setup_pool().await;
        create(&pool, "test_tenant", "USD", "US Dollar", 2)
            .await
            .unwrap();

        let updated = update(
            &pool,
            "USD",
            Some("US Dollar Updated"),
            None,
            Some("test_tenant"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.name, "US Dollar Updated");
        assert_eq!(updated.decimals, 2);
    }

    #[tokio::test]
    async fn deactivate_currency() {
        let pool = setup_pool().await;
        create(&pool, "test_tenant", "EUR", "Euro", 2)
            .await
            .unwrap();

        update(&pool, "EUR", None, Some(false), Some("test_tenant"))
            .await
            .unwrap();
        let c = find_by_code(&pool, "EUR", Some("test_tenant"))
            .await
            .unwrap()
            .unwrap();
        assert!(!c.is_active);

        assert!(
            find_active_by_code(&pool, "EUR", Some("test_tenant"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_currency() {
        let pool = setup_pool().await;
        create(&pool, "test_tenant", "JPY", "Japanese Yen", 0)
            .await
            .unwrap();
        assert!(
            delete_by_code(&pool, "JPY", Some("test_tenant"))
                .await
                .unwrap()
        );
        assert!(
            find_by_code(&pool, "JPY", Some("test_tenant"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_currency_in_use_rejected() {
        let pool = setup_pool().await;
        let user = crate::models::user::create(
            &pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        crate::models::wallet::create(&pool, user.id, "CNY")
            .await
            .unwrap();

        let err = delete_by_code(&pool, "CNY", Some("default"))
            .await
            .unwrap_err();
        match err {
            crate::errors::app_error::AppError::BadRequest(msg) => {
                assert!(msg.starts_with("currency_in_use"));
            }
            _ => panic!("expected BadRequest, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_false() {
        let pool = setup_pool().await;
        assert!(
            !delete_by_code(&pool, "XXX", Some("test_tenant"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn find_all_currencies() {
        let pool = setup_pool().await;
        create(&pool, "test_tenant", "CNY", "Chinese Yuan", 2)
            .await
            .unwrap();
        create(&pool, "test_tenant", "USD", "US Dollar", 2)
            .await
            .unwrap();
        let all = find_all(&pool, Some("test_tenant")).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].code, "CNY");
        assert_eq!(all[1].code, "USD");
    }
}
