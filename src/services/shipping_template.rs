use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::shipping_template::{
    CalculateShippingResponse, CreateShippingTemplateRequest, ShippingTemplateResponse,
    TemplateShippingDetail, UpdateShippingTemplateRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::shipping_template::{
    ShippingTemplate, ShippingTemplateStatus, ShippingTemplateType,
};
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait ShippingTemplateService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateShippingTemplateRequest,
    ) -> AppResult<ShippingTemplate>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateShippingTemplateRequest,
    ) -> AppResult<ShippingTemplate>;
    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ShippingTemplate>;
    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<ShippingTemplateResponse>, i64)>;
    fn calculate_for_template(
        &self,
        template: &ShippingTemplate,
        value: i64,
        region: Option<&str>,
    ) -> i64;
    async fn calculate_shipping(
        &self,
        product_weights: &[(SnowflakeId, i64, i64)],
        region: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<CalculateShippingResponse>;
}

pub struct ShippingTemplateServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl ShippingTemplateServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ShippingTemplateService for ShippingTemplateServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateShippingTemplateRequest,
    ) -> AppResult<ShippingTemplate> {
        auth.ensure_admin()?;
        crate::models::shipping_template::insert(
            &self.pool,
            &crate::commands::CreateShippingTemplateCmd {
                name: req.name,
                template_type: req.template_type.unwrap_or_else(|| "weight".to_string()),
                first_unit: req.first_unit.unwrap_or(1),
                first_price: req.first_price.unwrap_or(0),
                additional_unit: req.additional_unit.unwrap_or(1),
                additional_price: req.additional_price.unwrap_or(0),
                free_shipping_amount: req.free_shipping_amount.unwrap_or(0),
                regions: req.regions.unwrap_or_else(|| "[]".to_string()),
            },
            auth.tenant_id(),
        )
        .await
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateShippingTemplateRequest,
    ) -> AppResult<ShippingTemplate> {
        auth.ensure_admin()?;
        crate::models::shipping_template::update(
            &self.pool,
            &crate::commands::UpdateShippingTemplateCmd {
                id,
                name: req.name,
                template_type: req.template_type,
                first_unit: req.first_unit,
                first_price: req.first_price,
                additional_unit: req.additional_unit,
                additional_price: req.additional_price,
                free_shipping_amount: req.free_shipping_amount,
                regions: req.regions,
                status: req.status,
            },
            auth.tenant_id(),
        )
        .await?;
        crate::models::shipping_template::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("shipping_template"))
    }

    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        auth.ensure_admin()?;
        crate::models::shipping_template::delete_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ShippingTemplate> {
        crate::models::shipping_template::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("shipping_template"))
    }

    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<ShippingTemplateResponse>, i64)> {
        let (items, total) = crate::models::shipping_template::find_all_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            status,
        )
        .await?;
        Ok((
            items
                .into_iter()
                .map(ShippingTemplateResponse::from)
                .collect(),
            total,
        ))
    }

    fn calculate_for_template(
        &self,
        template: &ShippingTemplate,
        value: i64,
        _region: Option<&str>,
    ) -> i64 {
        if template.template_type == ShippingTemplateType::Flat {
            return template.first_price;
        }

        if value <= template.first_unit {
            return template.first_price;
        }

        let remaining = value - template.first_unit;
        let additional_blocks =
            (remaining + template.additional_unit - 1) / template.additional_unit;
        template.first_price + additional_blocks * template.additional_price
    }

    async fn calculate_shipping(
        &self,
        product_weights: &[(SnowflakeId, i64, i64)],
        region: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<CalculateShippingResponse> {
        let mut template_map: HashMap<i64, (ShippingTemplate, i64)> = HashMap::new();

        for (product_id, _weight, _quantity) in product_weights {
            let product = crate::models::product::find_by_id(&self.pool, *product_id, tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found("product"))?;

            let tmpl_id = match product.shipping_template_id {
                Some(id) => *id,
                None => continue,
            };

            let tmpl = match crate::models::shipping_template::find_by_id(
                &self.pool,
                SnowflakeId(tmpl_id),
                tenant_id,
            )
            .await?
            {
                Some(t) if t.status == ShippingTemplateStatus::Active => t,
                _ => continue,
            };

            let entry = template_map.entry(tmpl_id).or_insert_with(|| (tmpl, 0));
            if product.fulfillment_type == crate::models::product::FulfillmentType::Physical {
                let weight_val = product.weight.unwrap_or(0);
                entry.1 += weight_val * _quantity;
            }
        }

        let mut total_shipping: i64 = 0;
        let mut details = Vec::new();

        for (tmpl, total_value) in template_map.values() {
            if tmpl.free_shipping_amount > 0 && *total_value >= tmpl.free_shipping_amount {
                continue;
            }
            let amount = self.calculate_for_template(tmpl, *total_value, region);
            total_shipping += amount;
            details.push(TemplateShippingDetail {
                template_id: tmpl.id.to_string(),
                template_name: tmpl.name.clone(),
                shipping_amount: amount,
            });
        }

        Ok(CalculateShippingResponse {
            shipping_amount: total_shipping,
            details,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(
        template_type: ShippingTemplateType,
        first_unit: i64,
        first_price: i64,
        additional_unit: i64,
        additional_price: i64,
        free_shipping_amount: i64,
    ) -> ShippingTemplate {
        ShippingTemplate {
            id: SnowflakeId(1),
            tenant_id: None,
            name: "Test Template".to_string(),
            template_type,
            first_unit,
            first_price,
            additional_unit,
            additional_price,
            free_shipping_amount,
            regions: None,
            status: ShippingTemplateStatus::Active,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn make_service(pool: crate::db::Pool) -> ShippingTemplateServiceImpl {
        ShippingTemplateServiceImpl::new(Arc::new(pool))
    }

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    #[tokio::test]
    async fn calculate_weight_within_first_unit() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let tmpl = make_template(ShippingTemplateType::Weight, 1000, 500, 500, 200, 0);
        assert_eq!(svc.calculate_for_template(&tmpl, 800, None), 500);
        assert_eq!(svc.calculate_for_template(&tmpl, 1000, None), 500);
    }

    #[tokio::test]
    async fn calculate_weight_one_additional_block() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let tmpl = make_template(ShippingTemplateType::Weight, 1000, 500, 500, 200, 0);
        assert_eq!(svc.calculate_for_template(&tmpl, 1001, None), 700);
        assert_eq!(svc.calculate_for_template(&tmpl, 1500, None), 700);
    }

    #[tokio::test]
    async fn calculate_weight_multiple_additional_blocks() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let tmpl = make_template(ShippingTemplateType::Weight, 1000, 500, 500, 200, 0);
        assert_eq!(svc.calculate_for_template(&tmpl, 1501, None), 900);
        assert_eq!(svc.calculate_for_template(&tmpl, 2500, None), 1100);
    }

    #[tokio::test]
    async fn calculate_flat_always_first_price() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let tmpl = make_template(ShippingTemplateType::Flat, 0, 800, 0, 0, 0);
        assert_eq!(svc.calculate_for_template(&tmpl, 0, None), 800);
        assert_eq!(svc.calculate_for_template(&tmpl, 10000, None), 800);
    }

    #[tokio::test]
    async fn calculate_quantity_type() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let tmpl = make_template(ShippingTemplateType::Quantity, 1, 500, 1, 200, 0);
        assert_eq!(svc.calculate_for_template(&tmpl, 1, None), 500);
        assert_eq!(svc.calculate_for_template(&tmpl, 2, None), 700);
        assert_eq!(svc.calculate_for_template(&tmpl, 5, None), 1300);
    }

    #[tokio::test]
    async fn calculate_shipping_with_template() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());

        let tmpl = crate::models::shipping_template::insert(
            &pool,
            &crate::commands::CreateShippingTemplateCmd {
                name: "Weight Shipping".to_string(),
                template_type: "weight".to_string(),
                first_unit: 1000,
                first_price: 500,
                additional_unit: 500,
                additional_price: 200,
                free_shipping_amount: 0,
                regions: "[]".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        let product = crate::models::product::insert(
            &pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Heavy Item".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "physical".to_string(),
                delivery_hook: None,
                weight: Some(1200),
                price: 5000,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                stock: 100,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();

        sqlx::query("UPDATE products SET shipping_template_id = ? WHERE id = ?")
            .bind(*tmpl.id)
            .bind(product.id)
            .execute(&pool)
            .await
            .unwrap();

        let result = svc
            .calculate_shipping(&[(product.id, 1200, 2)], None, None)
            .await
            .unwrap();

        assert!(result.shipping_amount > 0);
        assert_eq!(result.details.len(), 1);
        assert_eq!(result.details[0].template_id, tmpl.id.to_string());
    }

    #[tokio::test]
    async fn calculate_shipping_free_threshold() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());

        let tmpl = crate::models::shipping_template::insert(
            &pool,
            &crate::commands::CreateShippingTemplateCmd {
                name: "Free Over 10k".to_string(),
                template_type: "weight".to_string(),
                first_unit: 1000,
                first_price: 500,
                additional_unit: 500,
                additional_price: 200,
                free_shipping_amount: 2000,
                regions: "[]".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        let product = crate::models::product::insert(
            &pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Heavy Item".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "physical".to_string(),
                delivery_hook: None,
                weight: Some(2500),
                price: 5000,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                stock: 100,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();

        sqlx::query("UPDATE products SET shipping_template_id = ? WHERE id = ?")
            .bind(*tmpl.id)
            .bind(product.id)
            .execute(&pool)
            .await
            .unwrap();

        let result = svc
            .calculate_shipping(&[(product.id, 2500, 1)], None, None)
            .await
            .unwrap();

        assert_eq!(result.shipping_amount, 0);
        assert!(result.details.is_empty());
    }

    #[tokio::test]
    async fn calculate_shipping_no_template_skips() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());

        let product = crate::models::product::insert(
            &pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Digital".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 100,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                stock: 100,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();

        let result = svc
            .calculate_shipping(&[(product.id, 0, 1)], None, None)
            .await
            .unwrap();

        assert_eq!(result.shipping_amount, 0);
        assert!(result.details.is_empty());
    }

    #[tokio::test]
    async fn calculate_shipping_inactive_template_skipped() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());

        let tmpl = crate::models::shipping_template::insert(
            &pool,
            &crate::commands::CreateShippingTemplateCmd {
                name: "Inactive".to_string(),
                template_type: "flat".to_string(),
                first_unit: 1,
                first_price: 500,
                additional_unit: 1,
                additional_price: 0,
                free_shipping_amount: 0,
                regions: "[]".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        sqlx::query("UPDATE shipping_templates SET status = 'inactive' WHERE id = ?")
            .bind(tmpl.id)
            .execute(&pool)
            .await
            .unwrap();

        let product = crate::models::product::insert(
            &pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Physical Item".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "physical".to_string(),
                delivery_hook: None,
                weight: Some(500),
                price: 1000,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                stock: 100,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();

        sqlx::query("UPDATE products SET shipping_template_id = ? WHERE id = ?")
            .bind(*tmpl.id)
            .bind(product.id)
            .execute(&pool)
            .await
            .unwrap();

        let result = svc
            .calculate_shipping(&[(product.id, 500, 1)], None, None)
            .await
            .unwrap();

        assert_eq!(result.shipping_amount, 0);
        assert!(result.details.is_empty());
    }
}
