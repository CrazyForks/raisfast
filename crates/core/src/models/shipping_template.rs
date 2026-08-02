use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    ShippingTemplateType {
        Weight = "weight",
        Quantity = "quantity",
        Flat = "flat",
    }
);

define_enum!(
    ShippingTemplateStatus {
        Active = "active",
        Inactive = "inactive",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ShippingTemplate {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub template_type: ShippingTemplateType,
    pub first_unit: i64,
    pub first_price: i64,
    pub additional_unit: i64,
    pub additional_price: i64,
    pub free_shipping_amount: i64,
    pub regions: Option<String>,
    pub status: ShippingTemplateStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ShippingTemplate>> {
    let result: Option<ShippingTemplate> = raisfast_derive::crud_find!(
        pool,
        "shipping_templates",
        ShippingTemplate,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_all_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<ShippingTemplate>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool,
        ShippingTemplate,
        table: "shipping_templates",
        where: ["status" => status],
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateShippingTemplateCmd,
    tenant_id: Option<&str>,
) -> AppResult<ShippingTemplate> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "shipping_templates",
        [
            "id" => id,
            "name" => &cmd.name,
            "type" => &cmd.template_type,
            "first_unit" => cmd.first_unit,
            "first_price" => cmd.first_price,
            "additional_unit" => cmd.additional_unit,
            "additional_price" => cmd.additional_price,
            "free_shipping_amount" => cmd.free_shipping_amount,
            "regions" => &cmd.regions,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id).await?.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("shipping_template not found after insert"))
    })
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateShippingTemplateCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let existing = find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("shipping_template"))?;

    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(
        pool,
        "shipping_templates",
        bind: [
            "name" => cmd.name.as_deref().unwrap_or(&existing.name),
            "type" => cmd.template_type.as_deref().unwrap_or(existing.template_type.as_str()),
            "first_unit" => cmd.first_unit.unwrap_or(existing.first_unit),
            "first_price" => cmd.first_price.unwrap_or(existing.first_price),
            "additional_unit" => cmd.additional_unit.unwrap_or(existing.additional_unit),
            "additional_price" => cmd.additional_price.unwrap_or(existing.additional_price),
            "free_shipping_amount" => cmd.free_shipping_amount.unwrap_or(existing.free_shipping_amount),
            "regions" => cmd.regions.as_deref().or(existing.regions.as_deref()).unwrap_or("[]"),
            "status" => cmd.status.as_deref().unwrap_or(existing.status.as_str()),
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", cmd.id),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "shipping_templates",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "shipping_template")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn seed_cmd(name: &str) -> crate::commands::CreateShippingTemplateCmd {
        crate::commands::CreateShippingTemplateCmd {
            name: name.to_string(),
            template_type: "weight".to_string(),
            first_unit: 1000,
            first_price: 500,
            additional_unit: 500,
            additional_price: 200,
            free_shipping_amount: 0,
            regions: "[]".to_string(),
        }
    }

    #[tokio::test]
    async fn test_insert_and_find_by_id() {
        let pool = setup_pool().await;
        let t = super::insert(&pool, &seed_cmd("Standard"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, t.id, None).await.unwrap().unwrap();
        assert_eq!(found.name, "Standard");
        assert_eq!(found.template_type, ShippingTemplateType::Weight);
        assert_eq!(found.first_unit, 1000);
        assert_eq!(found.first_price, 500);
        assert_eq!(found.status, ShippingTemplateStatus::Active);
    }

    #[tokio::test]
    async fn test_find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_id(&pool, SnowflakeId(99999), None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_find_all_paginated() {
        let pool = setup_pool().await;
        for i in 0..5 {
            super::insert(&pool, &seed_cmd(&format!("T{i}")), None)
                .await
                .unwrap();
        }
        let (items, total) = super::find_all_paginated(&pool, None, 1, 3, None)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn test_find_all_paginated_status_filter() {
        let pool = setup_pool().await;
        super::insert(&pool, &seed_cmd("Active"), None)
            .await
            .unwrap();
        let t2 = super::insert(&pool, &seed_cmd("Inactive"), None)
            .await
            .unwrap();
        sqlx::query("UPDATE shipping_templates SET status = 'inactive' WHERE id = ?")
            .bind(t2.id)
            .execute(&pool)
            .await
            .unwrap();

        let (items, total) = super::find_all_paginated(&pool, None, 1, 10, Some("active"))
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].name, "Active");
    }

    #[tokio::test]
    async fn test_update_changes_name() {
        let pool = setup_pool().await;
        let t = super::insert(&pool, &seed_cmd("Old"), None).await.unwrap();
        let ok = super::update(
            &pool,
            &crate::commands::UpdateShippingTemplateCmd {
                id: t.id,
                name: Some("New".into()),
                template_type: None,
                first_unit: None,
                first_price: Some(800),
                additional_unit: None,
                additional_price: None,
                free_shipping_amount: None,
                regions: None,
                status: None,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);
        let found = super::find_by_id(&pool, t.id, None).await.unwrap().unwrap();
        assert_eq!(found.name, "New");
        assert_eq!(found.first_price, 800);
    }

    #[tokio::test]
    async fn test_update_status_to_inactive() {
        let pool = setup_pool().await;
        let t = super::insert(&pool, &seed_cmd("Stat"), None).await.unwrap();
        super::update(
            &pool,
            &crate::commands::UpdateShippingTemplateCmd {
                id: t.id,
                name: None,
                template_type: None,
                first_unit: None,
                first_price: None,
                additional_unit: None,
                additional_price: None,
                free_shipping_amount: None,
                regions: None,
                status: Some("inactive".into()),
            },
            None,
        )
        .await
        .unwrap();
        let found = super::find_by_id(&pool, t.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, ShippingTemplateStatus::Inactive);
    }

    #[tokio::test]
    async fn test_delete_removes_template() {
        let pool = setup_pool().await;
        let t = super::insert(&pool, &seed_cmd("Del"), None).await.unwrap();
        super::delete_by_id(&pool, t.id, None).await.unwrap();
        assert!(
            super::find_by_id(&pool, t.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let pool = setup_pool().await;
        let err = super::delete_by_id(&pool, SnowflakeId(99999), None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::errors::app_error::AppError::NotFound(_)
        ));
    }
}
