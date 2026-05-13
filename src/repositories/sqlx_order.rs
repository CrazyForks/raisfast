use crate::errors::app_error::AppResult;
use crate::models::order::{self, Order};
use crate::models::order_item::{self, OrderItem, InsertOrderItem};
use crate::models::product::{self, Product};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxProductRepository);

#[allow(clippy::too_many_arguments)]
#[async_trait::async_trait]
pub trait ProductRepository: Send + Sync {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Product>>;
    async fn find_by_document_id(&self, document_id: &str, tenant_id: Option<&str>) -> AppResult<Option<Product>>;
    async fn find_active_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Product>, i64)>;
    async fn find_all_admin(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<Product>, i64)>;
    async fn insert(
        &self,
        document_id: &str,
        category_id: Option<i64>,
        title: &str,
        description: Option<&str>,
        cover_url: Option<&str>,
        product_type: &str,
        fulfillment_type: &str,
        delivery_hook: Option<&str>,
        weight: Option<i64>,
        price: i64,
        currency: &str,
        attributes: Option<&str>,
        sort_order: i64,
        slug: Option<&str>,
        content: Option<&str>,
        image_ids: Option<&str>,
        original_price: Option<i64>,
        specs: Option<&str>,
        unit: &str,
        min_purchase: i64,
        max_purchase: Option<i64>,
        virtual_sales: i64,
        meta_title: Option<&str>,
        meta_description: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<Product>;
    async fn update(
        &self,
        id: i64,
        category_id: Option<i64>,
        title: &str,
        description: Option<&str>,
        cover_url: Option<&str>,
        product_type: &str,
        fulfillment_type: &str,
        delivery_hook: Option<&str>,
        weight: Option<i64>,
        price: i64,
        currency: &str,
        status: &str,
        attributes: Option<&str>,
        sort_order: i64,
        slug: Option<&str>,
        content: Option<&str>,
        image_ids: Option<&str>,
        original_price: Option<i64>,
        specs: Option<&str>,
        unit: &str,
        min_purchase: i64,
        max_purchase: Option<i64>,
        total_sales: i64,
        virtual_sales: i64,
        meta_title: Option<&str>,
        meta_description: Option<&str>,
        published_at: Option<&str>,
        version: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<bool>;
    async fn delete_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<bool>;
}

#[async_trait::async_trait]
impl ProductRepository for SqlxProductRepository {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Product>> {
        product::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(&self, document_id: &str, tenant_id: Option<&str>) -> AppResult<Option<Product>> {
        product::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn find_active_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Product>, i64)> {
        product::find_active_paginated(&self.pool, tenant_id, page, page_size).await
    }

    async fn find_all_admin(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<Product>, i64)> {
        product::find_all_admin(&self.pool, tenant_id, page, page_size, status).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert(
        &self,
        document_id: &str,
        category_id: Option<i64>,
        title: &str,
        description: Option<&str>,
        cover_url: Option<&str>,
        product_type: &str,
        fulfillment_type: &str,
        delivery_hook: Option<&str>,
        weight: Option<i64>,
        price: i64,
        currency: &str,
        attributes: Option<&str>,
        sort_order: i64,
        slug: Option<&str>,
        content: Option<&str>,
        image_ids: Option<&str>,
        original_price: Option<i64>,
        specs: Option<&str>,
        unit: &str,
        min_purchase: i64,
        max_purchase: Option<i64>,
        virtual_sales: i64,
        meta_title: Option<&str>,
        meta_description: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<Product> {
        product::insert(
            &self.pool,
            document_id,
            category_id,
            title,
            description,
            cover_url,
            product_type,
            fulfillment_type,
            delivery_hook,
            weight,
            price,
            currency,
            attributes,
            sort_order,
            slug,
            content,
            image_ids,
            original_price,
            specs,
            unit,
            min_purchase,
            max_purchase,
            virtual_sales,
            meta_title,
            meta_description,
            tenant_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: i64,
        category_id: Option<i64>,
        title: &str,
        description: Option<&str>,
        cover_url: Option<&str>,
        product_type: &str,
        fulfillment_type: &str,
        delivery_hook: Option<&str>,
        weight: Option<i64>,
        price: i64,
        currency: &str,
        status: &str,
        attributes: Option<&str>,
        sort_order: i64,
        slug: Option<&str>,
        content: Option<&str>,
        image_ids: Option<&str>,
        original_price: Option<i64>,
        specs: Option<&str>,
        unit: &str,
        min_purchase: i64,
        max_purchase: Option<i64>,
        total_sales: i64,
        virtual_sales: i64,
        meta_title: Option<&str>,
        meta_description: Option<&str>,
        published_at: Option<&str>,
        version: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<bool> {
        product::update(
            &self.pool,
            id,
            category_id,
            title,
            description,
            cover_url,
            product_type,
            fulfillment_type,
            delivery_hook,
            weight,
            price,
            currency,
            status,
            attributes,
            sort_order,
            slug,
            content,
            image_ids,
            original_price,
            specs,
            unit,
            min_purchase,
            max_purchase,
            total_sales,
            virtual_sales,
            meta_title,
            meta_description,
            published_at,
            version,
            tenant_id,
        )
        .await
    }

    async fn delete_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<bool> {
        product::delete_by_id(&self.pool, id, tenant_id).await
    }
}

define_sqlx_repo!(SqlxOrderRepository);

#[allow(clippy::too_many_arguments)]
#[async_trait::async_trait]
pub trait OrderRepository: Send + Sync {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Order>>;
    async fn find_by_document_id(&self, document_id: &str, tenant_id: Option<&str>) -> AppResult<Option<Order>>;
    async fn find_by_order_no(&self, order_no: &str, tenant_id: Option<&str>) -> AppResult<Option<Order>>;
    async fn find_by_user_paginated(
        &self,
        user_id: i64,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Order>, i64)>;
    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<Order>, i64)>;
    async fn insert_order(
        &self,
        document_id: &str,
        user_id: i64,
        order_no: &str,
        subtotal: i64,
        discount_amount: i64,
        shipping_amount: i64,
        total_amount: i64,
        currency: &str,
        buyer_name: Option<&str>,
        buyer_phone: Option<&str>,
        buyer_email: Option<&str>,
        shipping_address: Option<&str>,
        remark: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<Order>;
    async fn update_status(
        &self,
        id: i64,
        status: &str,
        timestamp_col: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;
    async fn update_shipped(
        &self,
        id: i64,
        tracking_no: Option<&str>,
        carrier: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;
    async fn update_admin_remark(&self, id: i64, admin_remark: &str, tenant_id: Option<&str>) -> AppResult<()>;
    async fn update_delivery_data(&self, id: i64, delivery_data: &str, tenant_id: Option<&str>) -> AppResult<()>;
    async fn find_items_by_order_id(&self, order_id: i64, tenant_id: Option<&str>) -> AppResult<Vec<OrderItem>>;
    async fn insert_items_batch(&self, items: Vec<InsertOrderItem>, tenant_id: Option<&str>) -> AppResult<()>;
}

#[async_trait::async_trait]
impl OrderRepository for SqlxOrderRepository {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Order>> {
        order::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(&self, document_id: &str, tenant_id: Option<&str>) -> AppResult<Option<Order>> {
        order::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn find_by_order_no(&self, order_no: &str, tenant_id: Option<&str>) -> AppResult<Option<Order>> {
        order::find_by_order_no(&self.pool, order_no, tenant_id).await
    }

    async fn find_by_user_paginated(
        &self,
        user_id: i64,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Order>, i64)> {
        order::find_by_user_paginated(&self.pool, user_id, tenant_id, page, page_size).await
    }

    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<Order>, i64)> {
        order::find_all_admin_paginated(&self.pool, tenant_id, page, page_size, status).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_order(
        &self,
        document_id: &str,
        user_id: i64,
        order_no: &str,
        subtotal: i64,
        discount_amount: i64,
        shipping_amount: i64,
        total_amount: i64,
        currency: &str,
        buyer_name: Option<&str>,
        buyer_phone: Option<&str>,
        buyer_email: Option<&str>,
        shipping_address: Option<&str>,
        remark: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<Order> {
        order::insert(
            &self.pool,
            document_id,
            user_id,
            order_no,
            subtotal,
            discount_amount,
            shipping_amount,
            total_amount,
            currency,
            buyer_name,
            buyer_phone,
            buyer_email,
            shipping_address,
            remark,
            tenant_id,
        )
        .await
    }

    async fn update_status(
        &self,
        id: i64,
        status: &str,
        timestamp_col: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        order::update_status(&self.pool, id, status, timestamp_col, tenant_id).await
    }

    async fn update_shipped(
        &self,
        id: i64,
        tracking_no: Option<&str>,
        carrier: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        order::update_shipped(&self.pool, id, tracking_no, carrier, tenant_id).await
    }

    async fn update_admin_remark(&self, id: i64, admin_remark: &str, tenant_id: Option<&str>) -> AppResult<()> {
        order::update_admin_remark(&self.pool, id, admin_remark, tenant_id).await
    }

    async fn update_delivery_data(&self, id: i64, delivery_data: &str, tenant_id: Option<&str>) -> AppResult<()> {
        order::update_delivery_data(&self.pool, id, delivery_data, tenant_id).await
    }

    async fn find_items_by_order_id(&self, order_id: i64, tenant_id: Option<&str>) -> AppResult<Vec<OrderItem>> {
        order_item::find_by_order_id(&self.pool, order_id, tenant_id).await
    }

    async fn insert_items_batch(&self, items: Vec<InsertOrderItem>, tenant_id: Option<&str>) -> AppResult<()> {
        order_item::insert_batch(&self.pool, items, tenant_id).await
    }
}
