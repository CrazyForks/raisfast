use crate::types::price::Price;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// A user's favorite (wishlist) entry for a product.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ProductFavorite {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub product_id: SnowflakeId,
    pub created_at: Timestamp,
}

/// A favorite joined with its product snapshot (for paginated listing).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ProductFavoriteJoinedRow {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub product_id: SnowflakeId,
    pub created_at: Timestamp,
    pub product_title: Option<String>,
    pub product_slug: Option<String>,
    pub product_cover_image: Option<String>,
    pub product_price: Option<Price>,
    pub product_original_price: Option<Price>,
    pub product_status: Option<String>,
    pub product_stock: Option<i64>,
    pub product_sales: Option<i64>,
}

/// Paginated favorites joined with product snapshots, newest first.
pub async fn find_paged_with_product(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<ProductFavoriteJoinedRow>, i64)> {
    let result = raisfast_derive::crud_join_paged!(
        pool, ProductFavoriteJoinedRow,
        select: [
            "f.id", "f.user_id", "f.product_id", "f.created_at",
            "p.title AS product_title", "p.slug AS product_slug",
            "p.cover_url AS product_cover_image", "p.price AS product_price",
            "p.original_price AS product_original_price", "p.status AS product_status",
            "p.stock AS product_stock",
            "(p.total_sales + p.virtual_sales) AS product_sales"
        ],
        from: "product_favorites f",
        joins: [
            LEFT "products p" ON "f.product_id = p.id"
        ],
        where: ("f.user_id", user_id),
        tenant_alias: "f",
        tenant: tenant_id,
        order_by: "f.created_at DESC",
        page: page,
        page_size: page_size
    );
    Ok(result)
}

/// List a user's favorites, newest first.
pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ProductFavorite>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "product_favorites",
        ProductFavorite,
        where: ("user_id", user_id),
        order_by: "created_at DESC",
        tenant: tenant_id
    )?)
}

/// Find one favorite by user + product (for toggle/dedupe).
pub async fn find_by_user_and_product(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ProductFavorite>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "product_favorites",
        ProductFavorite,
        where: AND(("user_id", user_id), ("product_id", product_id)),
        tenant: tenant_id
    )?)
}

/// Create a favorite row.
pub async fn tx_create(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        tx,
        "product_favorites",
        [
            "id" => id,
            "user_id" => user_id,
            "product_id" => product_id,
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    Ok(())
}

/// Delete a favorite by user + product.
pub async fn tx_delete_by_user_and_product(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        tx,
        "product_favorites",
        where: AND(("user_id", user_id), ("product_id", product_id)),
        tenant: tenant_id
    )?;
    Ok(())
}
