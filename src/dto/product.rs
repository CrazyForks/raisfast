use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;
use validator::Validate;

#[derive(Debug, Deserialize)]
pub struct AdminProductListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
}

use super::validate_optional_id;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateProductRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    #[validate(custom(function = "validate_optional_id"))]
    pub category_id: Option<String>,
    pub product_type: Option<String>,
    pub fulfillment_type: Option<String>,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub price: i64,
    pub currency: Option<String>,
    pub attributes: Option<String>,
    pub sort_order: Option<i64>,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: Option<String>,
    pub min_purchase: Option<i64>,
    pub max_purchase: Option<i64>,
    pub virtual_sales: Option<i64>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub stock: Option<i64>,
    pub cost_price: Option<i64>,
    pub sale_price: Option<i64>,
    pub has_variants: Option<bool>,
    pub tag_ids: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateProductRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    #[validate(custom(function = "validate_optional_id"))]
    pub category_id: Option<String>,
    pub product_type: Option<String>,
    pub fulfillment_type: Option<String>,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub price: Option<i64>,
    pub currency: Option<String>,
    pub status: Option<String>,
    pub attributes: Option<String>,
    pub sort_order: Option<i64>,
    pub version: i64,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: Option<String>,
    pub min_purchase: Option<i64>,
    pub max_purchase: Option<i64>,
    pub virtual_sales: Option<i64>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub stock: Option<i64>,
    pub cost_price: Option<i64>,
    pub sale_price: Option<i64>,
    pub has_variants: Option<bool>,
    pub tag_ids: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductResponse {
    pub id: String,
    pub category_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub product_type: String,
    pub fulfillment_type: String,
    pub delivery_hook: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub weight: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub price: i64,
    pub currency: String,
    pub status: String,
    pub attributes: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sort_order: i64,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub min_purchase: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub max_purchase: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total_sales: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub virtual_sales: i64,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub published_at: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub stock: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub cost_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub sale_price: Option<i64>,
    pub has_variants: bool,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<crate::models::post::TagBrief>,
}

impl From<crate::models::product::Product> for ProductResponse {
    fn from(p: crate::models::product::Product) -> Self {
        Self {
            id: p.id.to_string(),
            category_id: p.category_id.map(|c| c.to_string()),
            title: p.title,
            description: p.description,
            cover_url: p.cover_url,
            product_type: p.product_type.to_string(),
            fulfillment_type: p.fulfillment_type.to_string(),
            delivery_hook: p.delivery_hook,
            weight: p.weight,
            price: p.price,
            currency: p.currency,
            status: p.status.to_string(),
            attributes: p.attributes,
            sort_order: p.sort_order,
            slug: p.slug,
            content: p.content,
            image_ids: p.image_ids,
            original_price: p.original_price,
            specs: p.specs,
            unit: p.unit,
            min_purchase: p.min_purchase,
            max_purchase: p.max_purchase,
            total_sales: p.total_sales,
            virtual_sales: p.virtual_sales,
            meta_title: p.meta_title,
            meta_description: p.meta_description,
            og_title: p.og_title,
            og_description: p.og_description,
            og_image: p.og_image,
            published_at: p.published_at.map(|t| t.to_string()),
            stock: p.stock,
            cost_price: p.cost_price,
            sale_price: p.sale_price,
            has_variants: p.has_variants,
            version: p.version,
            created_at: p.created_at.to_string(),
            updated_at: p.updated_at.to_string(),
            tags: vec![],
        }
    }
}
