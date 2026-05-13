use crate::audit::AuditService;
use crate::config::app::AppConfig;
use crate::db::dialect::ph;
use crate::db::pool::DbConnection;
use crate::dto::payment::*;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::models::payment_refund::PaymentRefund;
use crate::models::payment_transaction::PaymentTransaction;
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::payment::ProviderResponse;
use crate::repositories::{
    PaymentChannelRepository, PaymentOrderRepository, PaymentRefundRepository,
    PaymentTransactionRepository, WalletRepository,
};
use base64::Engine;

async fn tx_update_payment_order_status(
    tx: &mut DbConnection,
    id: i64,
    status: &str,
    timestamp_col: Option<&str>,
) -> AppResult<()> {
    let sql = if let Some(col) = timestamp_col {
        format!(
            "UPDATE payment_orders SET status = {}, {} = datetime('now'), updated_at = datetime('now'), version = version + 1 WHERE id = {}",
            ph(1),
            col,
            ph(2)
        )
    } else {
        format!(
            "UPDATE payment_orders SET status = {}, updated_at = datetime('now'), version = version + 1 WHERE id = {}",
            ph(1),
            ph(2)
        )
    };
    sqlx::query(&sql)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn tx_insert_payment_transaction(
    tx: &mut DbConnection,
    document_id: &str,
    payment_order_id: i64,
    order_id: Option<&str>,
    user_id: i64,
    tx_type: &str,
    amount: i64,
    currency: &str,
    provider_tx_id: &str,
    status: &str,
    raw_payload: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO payment_transactions (document_id, payment_order_id, order_id, user_id, tx_type, amount, currency, provider_tx_id, status, raw_payload, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(payment_order_id)
        .bind(order_id)
        .bind(user_id)
        .bind(tx_type)
        .bind(amount)
        .bind(currency)
        .bind(provider_tx_id)
        .bind(status)
        .bind(raw_payload)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn tx_insert_payment_refund(
    tx: &mut DbConnection,
    document_id: &str,
    payment_order_id: i64,
    order_id: Option<&str>,
    user_id: i64,
    amount: i64,
    currency: &str,
    reason: Option<&str>,
    provider_refund_id: Option<&str>,
    status: &str,
    metadata: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO payment_refunds (document_id, payment_order_id, order_id, user_id, amount, currency, reason, provider_refund_id, status, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(payment_order_id)
        .bind(order_id)
        .bind(user_id)
        .bind(amount)
        .bind(currency)
        .bind(reason)
        .bind(provider_refund_id)
        .bind(status)
        .bind(metadata)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn tx_sum_refunded_by_order(tx: &mut DbConnection, payment_order_id: i64) -> AppResult<i64> {
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0) FROM payment_refunds WHERE payment_order_id = {} AND status IN ('succeeded', 'pending', 'processing')",
        ph(1)
    );
    let (total,): (i64,) = sqlx::query_as(&sql)
        .bind(payment_order_id)
        .fetch_one(&mut *tx)
        .await?;
    Ok(total)
}

async fn tx_find_order_id_by_doc_id(
    tx: &mut DbConnection,
    document_id: &str,
) -> AppResult<Option<i64>> {
    let sql = format!("SELECT id FROM orders WHERE document_id = {}", ph(1));
    let result: Option<(i64,)> = sqlx::query_as(&sql)
        .bind(document_id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(result.map(|(id,)| id))
}

async fn tx_update_order_status(
    tx: &mut DbConnection,
    id: i64,
    status: &str,
    timestamp_col: Option<&str>,
) -> AppResult<()> {
    let sql = if let Some(col) = timestamp_col {
        format!(
            "UPDATE orders SET status = {}, {} = datetime('now'), updated_at = datetime('now') WHERE id = {}",
            ph(1),
            col,
            ph(2)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = datetime('now') WHERE id = {}",
            ph(1),
            ph(2)
        )
    };
    sqlx::query(&sql)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn tx_find_refund_by_doc_id(
    tx: &mut DbConnection,
    document_id: &str,
) -> AppResult<Option<PaymentRefund>> {
    let sql = format!(
        "SELECT * FROM payment_refunds WHERE document_id = {}",
        ph(1)
    );
    sqlx::query_as::<_, PaymentRefund>(&sql)
        .bind(document_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Into::into)
}

fn get_encrypt_key(config: &AppConfig) -> AppResult<[u8; 32]> {
    let key_str = config
        .app_key
        .as_deref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("APP_KEY not configured")))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(key_str)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("APP_KEY base64 decode: {e}")))?;
    if decoded.len() != 32 {
        return Err(AppError::Internal(anyhow::anyhow!(
            "APP_KEY must be 32 bytes, got {}",
            decoded.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded);
    Ok(arr)
}

fn encrypt_credential(value: &str, config: &AppConfig) -> AppResult<String> {
    let key = get_encrypt_key(config)?;
    crate::payment::crypto::aes256gcm_encrypt(value, &key)
}

pub async fn create_channel(
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
    config: &AppConfig,
    audit: &AuditService,
    req: CreatePaymentChannelRequest,
) -> AppResult<PaymentChannel> {
    auth.ensure_admin()?;
    let document_id = uuid::Uuid::now_v7().to_string();
    let encrypted_credentials = encrypt_credential(&req.credentials, config)?;
    let encrypted_webhook_secret = req
        .webhook_secret
        .as_deref()
        .map(|s| encrypt_credential(s, config))
        .transpose()?;
    let channel = channel_repo
        .insert(
            &document_id,
            &req.provider,
            &req.name,
            req.is_live.unwrap_or(false),
            &encrypted_credentials,
            encrypted_webhook_secret.as_deref(),
            req.settings.as_deref(),
            true,
            req.sort_order.unwrap_or(0),
            auth.tenant_id(),
        )
        .await?;
    let _ = audit
        .log(
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.channel.create",
            "payment_channel",
            Some(&channel.document_id),
            None,
            None,
            None,
        )
        .await;
    Ok(channel)
}

pub async fn update_channel(
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
    config: &AppConfig,
    audit: &AuditService,
    id: &str,
    req: UpdatePaymentChannelRequest,
) -> AppResult<PaymentChannel> {
    auth.ensure_admin()?;
    let channel = channel_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    let encrypted_credentials = if req.credentials.is_some() {
        encrypt_credential(
            req.credentials.as_deref().unwrap_or(&channel.credentials),
            config,
        )?
    } else {
        channel.credentials.clone()
    };

    let encrypted_webhook_secret = match req.webhook_secret.as_deref() {
        Some(s) => Some(encrypt_credential(s, config)?),
        None => channel.webhook_secret.clone(),
    };

    let updated = channel_repo
        .update(
            channel.id,
            &channel.provider,
            req.name.as_deref().unwrap_or(&channel.name),
            req.is_live.unwrap_or(channel.is_live != 0),
            &encrypted_credentials,
            encrypted_webhook_secret.as_deref(),
            req.settings.as_deref().or(channel.settings.as_deref()),
            req.is_active.unwrap_or(channel.is_active != 0),
            req.sort_order.unwrap_or(channel.sort_order),
            req.version,
            auth.tenant_id(),
        )
        .await?;

    if !updated {
        return Err(AppError::Conflict("version_conflict".into()));
    }

    let result = channel_repo
        .find_by_id(channel.id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    let _ = audit
        .log(
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.channel.update",
            "payment_channel",
            Some(&result.document_id),
            None,
            None,
            None,
        )
        .await;

    Ok(result)
}

pub async fn delete_channel(
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
    audit: &AuditService,
    id: &str,
) -> AppResult<()> {
    auth.ensure_admin()?;
    let channel = channel_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;
    let deleted = channel_repo
        .delete_by_id(channel.id, auth.tenant_id())
        .await?;
    if !deleted {
        return Err(AppError::not_found("payment_channel"));
    }
    let _ = audit
        .log(
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.channel.delete",
            "payment_channel",
            Some(&channel.document_id),
            None,
            None,
            None,
        )
        .await;
    Ok(())
}

pub async fn get_channel(
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
    id: &str,
) -> AppResult<PaymentChannel> {
    auth.ensure_admin()?;
    channel_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))
}

pub async fn list_channels(
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
) -> AppResult<Vec<PaymentChannel>> {
    auth.ensure_admin()?;
    channel_repo.find_all_active(auth.tenant_id()).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_payment_order(
    _pool: &crate::db::Pool,
    channel_repo: &dyn PaymentChannelRepository,
    order_repo: &dyn PaymentOrderRepository,
    _product_repo: &dyn crate::repositories::ProductRepository,
    order_order_repo: &dyn crate::repositories::OrderRepository,
    auth: &AuthUser,
    user_id: i64,
    req: CreatePaymentOrderRequest,
    config: &AppConfig,
    client_ip: Option<&str>,
) -> AppResult<(PaymentOrder, Option<ProviderResponse>)> {
    let _ = auth.ensure_authenticated()?;

    let order = order_order_repo
        .find_by_document_id(&req.order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    let channel = channel_repo
        .find_by_document_id(&req.channel_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    if channel.is_active == 0 {
        return Err(AppError::BadRequest("channel_inactive".into()));
    }

    let idempotency_key = format!("{}_{}", order.document_id, channel.document_id);

    if let Some(existing) = order_repo
        .find_by_idempotency_key(&idempotency_key, auth.tenant_id())
        .await?
    {
        return Ok((existing, None));
    }

    let document_id = uuid::Uuid::now_v7().to_string();
    let title = format!("Order {}", order.order_no);

    let payment_order = order_repo
        .insert(
            &document_id,
            user_id,
            Some(&order.document_id),
            &title,
            order.total_amount,
            &order.currency,
            channel.id,
            &channel.provider,
            None,
            None,
            req.return_url.as_deref(),
            &idempotency_key,
            client_ip,
            req.metadata.as_deref(),
            auth.tenant_id(),
        )
        .await?;

    let provider_response = async {
        let key = get_encrypt_key(config).ok()?;
        let provider = crate::payment::providers::get_provider(&channel.provider, &key).ok()?;
        let resp = provider
            .create(&channel, &payment_order, req.return_url.as_deref())
            .await
            .ok()?;
        order_repo
            .update_provider_order_id(
                payment_order.id,
                &resp.provider_order_id,
                None,
                auth.tenant_id(),
            )
            .await
            .ok()?;
        Some(resp)
    }
    .await;

    Ok((payment_order, provider_response))
}

pub async fn cancel_payment_order(
    order_repo: &dyn PaymentOrderRepository,
    channel_repo: &dyn PaymentChannelRepository,
    auth: &AuthUser,
    audit: &AuditService,
    config: &AppConfig,
    id: &str,
    user_id: i64,
) -> AppResult<()> {
    let _ = auth.ensure_authenticated()?;
    let order = order_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;

    if order.user_id != user_id {
        return Err(AppError::Forbidden);
    }
    if order.status != PaymentStatus::Pending {
        return Err(AppError::BadRequest("only_pending_can_cancel".into()));
    }

    if let Some(ref provider_order_id) = order.provider_order_id
        && let Ok(key) = get_encrypt_key(config)
    {
        let channel = channel_repo
            .find_by_id(order.channel_id, auth.tenant_id())
            .await?;
        if let Some(ch) = channel
            && let Ok(provider) = crate::payment::providers::get_provider(&order.provider, &key)
        {
            let _ = provider.cancel(&ch, provider_order_id).await;
        }
    }

    order_repo
        .update_status(
            order.id,
            PaymentStatus::Cancelled.as_str(),
            Some("cancelled_at"),
            auth.tenant_id(),
        )
        .await?;

    let _ = audit
        .log(
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.order.cancel",
            "payment_order",
            Some(&order.document_id),
            None,
            None,
            None,
        )
        .await;

    Ok(())
}

pub async fn get_payment_order(
    order_repo: &dyn PaymentOrderRepository,
    auth: &AuthUser,
    id: &str,
) -> AppResult<PaymentOrder> {
    let _ = auth.ensure_authenticated()?;
    order_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))
}

pub async fn list_user_payment_orders(
    order_repo: &dyn PaymentOrderRepository,
    auth: &AuthUser,
    user_id: i64,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    let _ = auth.ensure_authenticated()?;
    order_repo
        .find_by_user_paginated(user_id, auth.tenant_id(), page, page_size)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_callback(
    pool: &crate::db::Pool,
    channel_repo: &dyn PaymentChannelRepository,
    order_repo: &dyn PaymentOrderRepository,
    tx_repo: &dyn PaymentTransactionRepository,
    wallet_repo: &dyn WalletRepository,
    audit: &AuditService,
    config: &AppConfig,
    channel_doc_id: &str,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let channel = channel_repo
        .find_by_document_id(channel_doc_id, None)
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    if channel.is_active == 0 {
        return Err(AppError::BadRequest("channel_inactive".into()));
    }

    let key = get_encrypt_key(config)?;
    let provider = crate::payment::providers::get_provider(&channel.provider, &key)?;
    let callback = match provider.verify_callback(&channel, headers, body).await {
        Ok(cb) => cb,
        Err(e) => {
            let audit = AuditService::new(pool.clone());
            let _ = audit
                .log(
                    "",
                    None,
                    None,
                    "payment.callback.failed",
                    "payment_channel",
                    Some(channel_doc_id),
                    Some(&format!("verification_error: {e}")),
                    None,
                    None,
                )
                .await;
            return Err(e);
        }
    };

    let payment_order = order_repo
        .find_by_provider_order_id(&callback.provider_order_id, None)
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;

    if payment_order.status == PaymentStatus::Paid {
        return Ok(());
    }

    if payment_order.status != PaymentStatus::Pending {
        return Err(AppError::BadRequest("order_not_pending".into()));
    }

    if callback.amount != payment_order.amount {
        return Err(AppError::BadRequest("amount_mismatch".into()));
    }

    if callback.status != PaymentStatus::Paid {
        return Ok(());
    }

    if let Some(ref provider_tx_id) = callback.provider_tx_id
        && tx_repo
            .find_by_provider_tx_id(provider_tx_id, None)
            .await?
            .is_some()
    {
        return Ok(());
    }

    crate::in_transaction!(pool, tx, {
        tx_update_payment_order_status(
            &mut tx,
            payment_order.id,
            PaymentStatus::Paid.as_str(),
            Some("paid_at"),
        )
        .await?;

        if let Some(ref provider_tx_id) = callback.provider_tx_id {
            let tx_doc_id = uuid::Uuid::now_v7().to_string();
            let raw_payload = serde_json::to_string(&callback).ok();
            tx_insert_payment_transaction(
                &mut tx,
                &tx_doc_id,
                payment_order.id,
                payment_order.order_id.as_deref(),
                payment_order.user_id,
                "charge",
                payment_order.amount,
                &payment_order.currency,
                provider_tx_id,
                "succeeded",
                raw_payload.as_deref(),
            )
            .await?;
        }

        if let Some(ref order_doc_id) = payment_order.order_id
            && let Some(order_id) = tx_find_order_id_by_doc_id(&mut tx, order_doc_id).await?
        {
            tx_update_order_status(
                &mut tx,
                order_id,
                crate::models::order::OrderStatus::Paid.as_str(),
                Some("paid_at"),
            )
            .await?;
        }

        Ok(())
    })?;

    if let Err(e) = crate::services::wallet::credit_wallet(
        wallet_repo,
        pool,
        payment_order.user_id,
        &payment_order.currency,
        payment_order.amount,
        WalletTxType::Recharge,
        &format!("PAY-{}", payment_order.document_id),
        Some(WalletReferenceType::Payment),
        Some(&payment_order.document_id),
        None,
    )
    .await
    {
        tracing::error!("wallet credit failed after payment callback: {e}");
    }

    let _ = audit
        .log(
            "",
            None,
            None,
            "payment.callback.success",
            "payment_order",
            Some(&payment_order.document_id),
            None,
            None,
            None,
        )
        .await;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn refund_payment_order(
    pool: &crate::db::Pool,
    order_repo: &dyn PaymentOrderRepository,
    channel_repo: &dyn PaymentChannelRepository,
    _tx_repo: &dyn PaymentTransactionRepository,
    refund_repo: &dyn PaymentRefundRepository,
    wallet_repo: &dyn WalletRepository,
    auth: &AuthUser,
    audit: &AuditService,
    config: &AppConfig,
    id: &str,
    req: CreateRefundRequest,
) -> AppResult<PaymentRefund> {
    auth.ensure_admin()?;
    let payment_order = order_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;

    if payment_order.status != PaymentStatus::Paid
        && payment_order.status != PaymentStatus::PartiallyRefunded
    {
        return Err(AppError::BadRequest("only_paid_can_refund".into()));
    }

    let already_refunded = refund_repo
        .sum_refunded_by_order(payment_order.id, auth.tenant_id())
        .await?;

    if already_refunded + req.amount > payment_order.amount {
        return Err(AppError::BadRequest("refund_exceeds_payment".into()));
    }

    let provider_refund_id = if let Some(ref provider_order_id) = payment_order.provider_order_id {
        let key = get_encrypt_key(config)?;
        let channel = channel_repo
            .find_by_id(payment_order.channel_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("payment_channel"))?;
        let provider = crate::payment::providers::get_provider(&payment_order.provider, &key)?;
        let result = provider
            .refund(
                &channel,
                provider_order_id,
                req.amount,
                req.reason.as_deref(),
            )
            .await?;
        result.provider_refund_id
    } else {
        format!("re_{}", uuid::Uuid::now_v7())
    };
    let refund_doc_id = uuid::Uuid::now_v7().to_string();
    let wallet_tx_no = format!("PAYMENT_REFUND_{}", refund_doc_id);

    let refund = crate::in_transaction!(pool, tx, {
        tx_insert_payment_refund(
            &mut tx,
            &refund_doc_id,
            payment_order.id,
            payment_order.order_id.as_deref(),
            payment_order.user_id,
            req.amount,
            &payment_order.currency,
            req.reason.as_deref(),
            Some(&provider_refund_id),
            "succeeded",
            None,
        )
        .await?;

        let tx_doc_id = uuid::Uuid::now_v7().to_string();
        let provider_tx_id = format!("txr_{}", uuid::Uuid::now_v7());
        tx_insert_payment_transaction(
            &mut tx,
            &tx_doc_id,
            payment_order.id,
            payment_order.order_id.as_deref(),
            payment_order.user_id,
            "refund",
            req.amount,
            &payment_order.currency,
            &provider_tx_id,
            "succeeded",
            None,
        )
        .await?;

        let already_refunded_in_tx = tx_sum_refunded_by_order(&mut tx, payment_order.id).await?;
        let is_full_refund = already_refunded_in_tx >= payment_order.amount;
        let new_status = if is_full_refund {
            PaymentStatus::Refunded.as_str()
        } else {
            PaymentStatus::PartiallyRefunded.as_str()
        };
        tx_update_payment_order_status(&mut tx, payment_order.id, new_status, None).await?;

        let refund = tx_find_refund_by_doc_id(&mut tx, &refund_doc_id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("inserted refund not found")))?;

        Ok(refund)
    })?;

    if let Err(e) = crate::services::wallet::debit_wallet(
        wallet_repo,
        pool,
        payment_order.user_id,
        &payment_order.currency,
        req.amount,
        WalletTxType::Refund,
        &wallet_tx_no,
        Some(WalletReferenceType::PaymentRefund),
        Some(&payment_order.document_id),
        None,
    )
    .await
    {
        tracing::error!("wallet debit failed after refund: {e}");
    }

    let _ = audit
        .log(
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.refund.initiated",
            "payment_order",
            Some(&payment_order.document_id),
            Some(&format!("amount={}", req.amount)),
            None,
            None,
        )
        .await;

    if req.amount > 100_000 {
        let _ = audit
            .log(
                auth.tenant_id().unwrap_or(""),
                auth.user_id().and_then(|s| s.parse::<i64>().ok()),
                Some(auth.role()),
                "payment.refund.large",
                "payment_order",
                Some(&payment_order.document_id),
                Some(&format!("amount={} threshold=100000", req.amount)),
                None,
                None,
            )
            .await;
    }

    Ok(refund)
}

pub async fn list_admin_payment_orders(
    order_repo: &dyn PaymentOrderRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    auth.ensure_admin()?;
    order_repo
        .find_all_admin_paginated(auth.tenant_id(), page, page_size, status)
        .await
}

pub async fn list_admin_transactions(
    tx_repo: &dyn PaymentTransactionRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentTransaction>, i64)> {
    auth.ensure_admin()?;
    tx_repo
        .find_all_admin_paginated(auth.tenant_id(), page, page_size)
        .await
}

pub async fn list_admin_refunds(
    refund_repo: &dyn PaymentRefundRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentRefund>, i64)> {
    auth.ensure_admin()?;
    refund_repo
        .find_all_admin_paginated(auth.tenant_id(), page, page_size)
        .await
}
