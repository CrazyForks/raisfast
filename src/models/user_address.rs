use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct UserAddress {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub label: String,
    pub recipient_name: String,
    pub phone: String,
    pub country: String,
    pub province: String,
    pub city: String,
    pub district: String,
    pub address_line1: String,
    pub address_line2: Option<String>,
    pub postal_code: Option<String>,
    pub is_default: bool,
    pub address_type: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<UserAddress>> {
    let result: Option<UserAddress> = raisfast_derive::crud_find!(
        pool,
        "user_addresses",
        UserAddress,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<UserAddress>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "user_addresses",
        UserAddress,
        where: ("user_id", user_id),
        order_by: "is_default DESC, created_at DESC",
        tenant: tenant_id
    )?)
}

pub async fn find_default_by_user(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<UserAddress>> {
    let result: Option<UserAddress> = raisfast_derive::crud_find!(
        pool,
        "user_addresses",
        UserAddress,
        where: AND(("user_id", user_id), ("is_default", true)),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateUserAddressCmd,
    tenant_id: Option<&str>,
) -> AppResult<UserAddress> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "user_addresses",
        [
            "id" => id,
            "user_id" => cmd.user_id,
            "label" => &cmd.label,
            "recipient_name" => &cmd.recipient_name,
            "phone" => &cmd.phone,
            "country" => &cmd.country,
            "province" => &cmd.province,
            "city" => &cmd.city,
            "district" => &cmd.district,
            "address_line1" => &cmd.address_line1,
            "address_line2" => &cmd.address_line2,
            "postal_code" => &cmd.postal_code,
            "is_default" => cmd.is_default,
            "address_type" => &cmd.address_type,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("user_address not found after insert")))
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateUserAddressCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(
        pool,
        "user_addresses",
        bind: [
            "label" => &cmd.label,
            "recipient_name" => &cmd.recipient_name,
            "phone" => &cmd.phone,
            "country" => &cmd.country,
            "province" => &cmd.province,
            "city" => &cmd.city,
            "district" => &cmd.district,
            "address_line1" => &cmd.address_line1,
            "address_line2" => &cmd.address_line2,
            "postal_code" => &cmd.postal_code,
            "is_default" => cmd.is_default,
            "address_type" => &cmd.address_type,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: AND(("id", cmd.id), ("user_id", cmd.user_id)),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::DbQueryResult = raisfast_derive::crud_delete!(
        pool,
        "user_addresses",
        where: AND(("id", id), ("user_id", user_id)),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn count_by_user(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let count = raisfast_derive::crud_count!(pool, "user_addresses", where: ("user_id", user_id), tenant: tenant_id)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("testuser_{id}");
        sqlx::query(
            "INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')",
        )
        .bind(id)
        .bind(&username)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_address(pool: &crate::db::Pool, user_id: i64, label: &str) -> UserAddress {
        insert(
            pool,
            &crate::commands::CreateUserAddressCmd {
                user_id: SnowflakeId(user_id),
                label: label.to_string(),
                recipient_name: "John".to_string(),
                phone: "13800138000".to_string(),
                country: "CN".to_string(),
                province: "Guangdong".to_string(),
                city: "Shenzhen".to_string(),
                district: "Nanshan".to_string(),
                address_line1: "123 Main St".to_string(),
                address_line2: None,
                postal_code: Some("518000".to_string()),
                is_default: false,
                address_type: "shipping".to_string(),
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let addr = seed_address(&pool, uid, "Home").await;

        let found = super::find_by_id(&pool, addr.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, addr.id);
        assert_eq!(found.recipient_name, "John");
        assert_eq!(found.phone, "13800138000");
        assert_eq!(found.country, "CN");
        assert_eq!(found.address_line1, "123 Main St");
    }

    #[tokio::test]
    async fn find_by_user_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        seed_address(&pool, uid, "Home").await;
        seed_address(&pool, uid, "Office").await;

        let addrs = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert_eq!(addrs.len(), 2);
    }

    #[tokio::test]
    async fn find_by_user_id_empty() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let addrs = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert!(addrs.is_empty());
    }

    #[tokio::test]
    async fn find_default_by_user() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;

        insert(
            &pool,
            &crate::commands::CreateUserAddressCmd {
                user_id: SnowflakeId(uid),
                label: "Default".to_string(),
                recipient_name: "Jane".to_string(),
                phone: "13900139000".to_string(),
                country: "CN".to_string(),
                province: "Beijing".to_string(),
                city: "Beijing".to_string(),
                district: "Chaoyang".to_string(),
                address_line1: "456 Road".to_string(),
                address_line2: None,
                postal_code: None,
                is_default: true,
                address_type: "shipping".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        let default = super::find_default_by_user(&pool, SnowflakeId(uid), None)
            .await
            .unwrap()
            .unwrap();
        assert!(default.is_default);
        assert_eq!(default.recipient_name, "Jane");
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let addr = seed_address(&pool, uid, "Old").await;

        let ok = super::update(
            &pool,
            &crate::commands::UpdateUserAddressCmd {
                id: addr.id,
                user_id: SnowflakeId(uid),
                label: "Updated".to_string(),
                recipient_name: "Jane".to_string(),
                phone: "13900139000".to_string(),
                country: "US".to_string(),
                province: "CA".to_string(),
                city: "SF".to_string(),
                district: "Downtown".to_string(),
                address_line1: "789 New St".to_string(),
                address_line2: Some("Apt 2".to_string()),
                postal_code: Some("94102".to_string()),
                is_default: true,
                address_type: "billing".to_string(),
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);

        let found = super::find_by_id(&pool, addr.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.label, "Updated");
        assert_eq!(found.recipient_name, "Jane");
        assert_eq!(found.country, "US");
        assert!(found.is_default);
        assert_eq!(found.address_type, "billing");
    }

    #[tokio::test]
    async fn update_wrong_user_fails() {
        let pool = setup_pool().await;
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let addr = seed_address(&pool, uid1, "Home").await;

        let ok = super::update(
            &pool,
            &crate::commands::UpdateUserAddressCmd {
                id: addr.id,
                user_id: SnowflakeId(uid2),
                label: "Hacked".to_string(),
                recipient_name: "X".to_string(),
                phone: "000".to_string(),
                country: "XX".to_string(),
                province: "X".to_string(),
                city: "X".to_string(),
                district: "X".to_string(),
                address_line1: "X".to_string(),
                address_line2: None,
                postal_code: None,
                is_default: false,
                address_type: "shipping".to_string(),
            },
            None,
        )
        .await
        .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn delete_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let addr = seed_address(&pool, uid, "Bye").await;

        let ok = super::delete_by_id(&pool, addr.id, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert!(ok);
        assert!(
            super::find_by_id(&pool, addr.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_wrong_user_fails() {
        let pool = setup_pool().await;
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let addr = seed_address(&pool, uid1, "Home").await;

        let ok = super::delete_by_id(&pool, addr.id, SnowflakeId(uid2), None)
            .await
            .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn count_by_user() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        assert_eq!(
            super::count_by_user(&pool, SnowflakeId(uid), None)
                .await
                .unwrap(),
            0
        );

        seed_address(&pool, uid, "Home").await;
        seed_address(&pool, uid, "Office").await;
        assert_eq!(
            super::count_by_user(&pool, SnowflakeId(uid), None)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;

        insert(
            &pool,
            &crate::commands::CreateUserAddressCmd {
                user_id: SnowflakeId(uid),
                label: "TenantA".to_string(),
                recipient_name: "TA".to_string(),
                phone: "111".to_string(),
                country: "CN".to_string(),
                province: "X".to_string(),
                city: "X".to_string(),
                district: "X".to_string(),
                address_line1: "X".to_string(),
                address_line2: None,
                postal_code: None,
                is_default: false,
                address_type: "shipping".to_string(),
            },
            Some("tenant_a"),
        )
        .await
        .unwrap();

        let a_addrs = super::find_by_user_id(&pool, SnowflakeId(uid), Some("tenant_a"))
            .await
            .unwrap();
        assert_eq!(a_addrs.len(), 1);

        let b_addrs = super::find_by_user_id(&pool, SnowflakeId(uid), Some("tenant_b"))
            .await
            .unwrap();
        assert!(b_addrs.is_empty());

        let all_addrs = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert_eq!(all_addrs.len(), 1);
    }
}
