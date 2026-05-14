use crate::commands::{
    CreatePaymentChannelCmd, CreatePaymentOrderCmd, CreatePaymentRefundCmd,
    CreatePaymentTransactionCmd,
};
use crate::errors::app_error::AppResult;
use crate::models::payment_channel::{self, PaymentChannel};
use crate::models::payment_order::{self, PaymentOrder};
use crate::models::payment_refund::{self, PaymentRefund};
use crate::models::payment_transaction::{self, PaymentTransaction};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxPaymentChannelRepository);

#[async_trait::async_trait]
pub trait PaymentChannelRepository: Send + Sync {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentChannel>>;
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentChannel>>;
    async fn find_all_active(&self, tenant_id: Option<&str>) -> AppResult<Vec<PaymentChannel>>;
    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        is_active: Option<bool>,
    ) -> AppResult<(Vec<PaymentChannel>, i64)>;
    async fn insert(
        &self,
        cmd: &CreatePaymentChannelCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentChannel>;
    async fn update(
        &self,
        cmd: &crate::commands::UpdatePaymentChannelCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<bool>;
    async fn delete_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<bool>;
}

#[async_trait::async_trait]
impl PaymentChannelRepository for SqlxPaymentChannelRepository {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentChannel>> {
        payment_channel::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentChannel>> {
        payment_channel::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn find_all_active(&self, tenant_id: Option<&str>) -> AppResult<Vec<PaymentChannel>> {
        payment_channel::find_all_active(&self.pool, tenant_id).await
    }

    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        is_active: Option<bool>,
    ) -> AppResult<(Vec<PaymentChannel>, i64)> {
        payment_channel::find_all_admin_paginated(&self.pool, tenant_id, page, page_size, is_active)
            .await
    }

    async fn insert(
        &self,
        cmd: &CreatePaymentChannelCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentChannel> {
        payment_channel::insert(&self.pool, cmd, tenant_id).await
    }

    async fn update(
        &self,
        cmd: &crate::commands::UpdatePaymentChannelCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<bool> {
        payment_channel::update(&self.pool, cmd, tenant_id).await
    }

    async fn delete_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<bool> {
        payment_channel::delete_by_id(&self.pool, id, tenant_id).await
    }
}

define_sqlx_repo!(SqlxPaymentOrderRepository);

#[async_trait::async_trait]
pub trait PaymentOrderRepository: Send + Sync {
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>)
    -> AppResult<Option<PaymentOrder>>;
    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>>;
    async fn find_by_idempotency_key(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>>;
    async fn find_by_provider_order_id(
        &self,
        provider_order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>>;
    async fn find_by_user_paginated(
        &self,
        user_id: i64,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentOrder>, i64)>;
    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<PaymentOrder>, i64)>;
    async fn insert(
        &self,
        cmd: &CreatePaymentOrderCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentOrder>;
    async fn update_provider_order_id(
        &self,
        id: i64,
        provider_order_id: &str,
        provider_data: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;
}

#[async_trait::async_trait]
impl PaymentOrderRepository for SqlxPaymentOrderRepository {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>> {
        payment_order::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_document_id(
        &self,
        document_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>> {
        payment_order::find_by_document_id(&self.pool, document_id, tenant_id).await
    }

    async fn find_by_idempotency_key(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>> {
        payment_order::find_by_idempotency_key(&self.pool, key, tenant_id).await
    }

    async fn find_by_provider_order_id(
        &self,
        provider_order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentOrder>> {
        payment_order::find_by_provider_order_id(&self.pool, provider_order_id, tenant_id).await
    }

    async fn find_by_user_paginated(
        &self,
        user_id: i64,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentOrder>, i64)> {
        payment_order::find_by_user_paginated(&self.pool, user_id, tenant_id, page, page_size).await
    }

    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<PaymentOrder>, i64)> {
        payment_order::find_all_admin_paginated(&self.pool, tenant_id, page, page_size, status)
            .await
    }

    async fn insert(
        &self,
        cmd: &CreatePaymentOrderCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentOrder> {
        payment_order::insert(&self.pool, cmd, tenant_id).await
    }

    async fn update_provider_order_id(
        &self,
        id: i64,
        provider_order_id: &str,
        provider_data: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        payment_order::update_provider_order_id(
            &self.pool,
            id,
            provider_order_id,
            provider_data,
            tenant_id,
        )
        .await
    }
}

define_sqlx_repo!(SqlxPaymentTransactionRepository);

#[async_trait::async_trait]
pub trait PaymentTransactionRepository: Send + Sync {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentTransaction>>;
    async fn find_by_payment_order_id(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentTransaction>>;
    async fn find_by_order_id(
        &self,
        order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentTransaction>>;
    async fn find_by_provider_tx_id(
        &self,
        provider_tx_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentTransaction>>;
    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentTransaction>, i64)>;
    async fn insert(
        &self,
        cmd: &CreatePaymentTransactionCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentTransaction>;
}

#[async_trait::async_trait]
impl PaymentTransactionRepository for SqlxPaymentTransactionRepository {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentTransaction>> {
        payment_transaction::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_payment_order_id(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentTransaction>> {
        payment_transaction::find_by_payment_order_id(&self.pool, payment_order_id, tenant_id).await
    }

    async fn find_by_order_id(
        &self,
        order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentTransaction>> {
        payment_transaction::find_by_order_id(&self.pool, order_id, tenant_id).await
    }

    async fn find_by_provider_tx_id(
        &self,
        provider_tx_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentTransaction>> {
        payment_transaction::find_by_provider_tx_id(&self.pool, provider_tx_id, tenant_id).await
    }

    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentTransaction>, i64)> {
        payment_transaction::find_all_admin_paginated(&self.pool, tenant_id, page, page_size).await
    }

    async fn insert(
        &self,
        cmd: &CreatePaymentTransactionCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentTransaction> {
        payment_transaction::insert(&self.pool, cmd, tenant_id).await
    }
}

define_sqlx_repo!(SqlxPaymentRefundRepository);

#[async_trait::async_trait]
pub trait PaymentRefundRepository: Send + Sync {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentRefund>>;
    async fn find_by_payment_order_id(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentRefund>>;
    async fn find_by_order_id(
        &self,
        order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentRefund>>;
    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentRefund>, i64)>;
    async fn insert(
        &self,
        cmd: &CreatePaymentRefundCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentRefund>;
    async fn update_status(&self, id: i64, status: &str, tenant_id: Option<&str>) -> AppResult<()>;
    async fn sum_refunded_by_order(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<i64>;
}

#[async_trait::async_trait]
impl PaymentRefundRepository for SqlxPaymentRefundRepository {
    async fn find_by_id(
        &self,
        id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<PaymentRefund>> {
        payment_refund::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn find_by_payment_order_id(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentRefund>> {
        payment_refund::find_by_payment_order_id(&self.pool, payment_order_id, tenant_id).await
    }

    async fn find_by_order_id(
        &self,
        order_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PaymentRefund>> {
        payment_refund::find_by_order_id(&self.pool, order_id, tenant_id).await
    }

    async fn find_all_admin_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentRefund>, i64)> {
        let offset = (page - 1) * page_size;
        let tenant_ph = crate::db::tenant::tenant_filter_ph(tenant_id, 1);
        let count_sql = format!(
            "SELECT COUNT(*) as count FROM payment_refunds WHERE 1=1{}",
            tenant_ph
        );
        let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql);
        if let Some(tid) = tenant_id {
            cq = cq.bind(tid);
        }
        let (total,): (i64,) = cq.fetch_one(&self.pool).await?;
        let base = usize::from(tenant_id.is_some()) + 1;
        let sql = format!(
            "SELECT * FROM payment_refunds WHERE 1=1{} ORDER BY created_at DESC LIMIT {} OFFSET {}",
            tenant_ph,
            crate::db::dialect::ph(base),
            crate::db::dialect::ph(base + 1)
        );
        let mut dq = sqlx::query_as::<_, PaymentRefund>(&sql);
        if let Some(tid) = tenant_id {
            dq = dq.bind(tid);
        }
        let rows = dq
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        Ok((rows, total))
    }

    async fn insert(
        &self,
        cmd: &CreatePaymentRefundCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<PaymentRefund> {
        payment_refund::insert(&self.pool, cmd, tenant_id).await
    }

    async fn update_status(&self, id: i64, status: &str, tenant_id: Option<&str>) -> AppResult<()> {
        payment_refund::update_status(&self.pool, id, status, tenant_id).await
    }

    async fn sum_refunded_by_order(
        &self,
        payment_order_id: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<i64> {
        payment_refund::sum_refunded_by_order(&self.pool, payment_order_id, tenant_id).await
    }
}
