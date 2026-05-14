use crate::audit::AuditService;
use crate::config::app::AppConfig;
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

fn is_unique_violation(err: &AppError) -> bool {
    match err {
        AppError::Internal(e) => {
            let s = e.to_string();
            s.contains("UNIQUE constraint failed") || s.contains("duplicate key")
        }
        _ => false,
    }
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

macro_rules! audit_log {
    ($audit:expr, $($arg:expr),*) => {
        if let Err(e) = $audit.log($($arg),*).await {
            tracing::warn!("audit log failed: {e}");
        }
    };
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
    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id().and_then(|s| s.parse::<i64>().ok()),
        Some(auth.role()),
        "payment.channel.create",
        "payment_channel",
        Some(&channel.document_id),
        None,
        None,
        None
    );
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

    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id().and_then(|s| s.parse::<i64>().ok()),
        Some(auth.role()),
        "payment.channel.update",
        "payment_channel",
        Some(&result.document_id),
        None,
        None,
        None
    );

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
    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id().and_then(|s| s.parse::<i64>().ok()),
        Some(auth.role()),
        "payment.channel.delete",
        "payment_channel",
        Some(&channel.document_id),
        None,
        None,
        None
    );
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

    if order.user_id != user_id {
        return Err(AppError::Forbidden);
    }

    if order.total_amount <= 0 {
        return Err(AppError::BadRequest("order_amount_invalid".into()));
    }

    let channel = channel_repo
        .find_by_document_id(&req.channel_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    if channel.is_active == 0 {
        return Err(AppError::BadRequest("channel_inactive".into()));
    }

    let idempotency_key = format!("{}_{}", order.document_id, channel.document_id);

    if let Some(existing) = order_repo
        .find_by_idempotency_key(&idempotency_key, None)
        .await?
    {
        return Ok((existing, None));
    }

    let document_id = uuid::Uuid::now_v7().to_string();
    let title = format!("Order {}", order.order_no);

    let payment_order = match order_repo
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
        .await
    {
        Ok(po) => po,
        Err(e) => {
            if is_unique_violation(&e)
                && let Some(existing) = order_repo
                    .find_by_idempotency_key(&idempotency_key, None)
                    .await?
            {
                return Ok((existing, None));
            }
            return Err(e);
        }
    };

    let key = get_encrypt_key(config)?;
    let provider = crate::payment::providers::get_provider(&channel.provider, &key)?;
    let provider_response = match provider
        .create(&channel, &payment_order, req.return_url.as_deref())
        .await
    {
        Ok(resp) => {
            if let Err(e) = order_repo
                .update_provider_order_id(
                    payment_order.id,
                    &resp.provider_order_id,
                    None,
                    auth.tenant_id(),
                )
                .await
            {
                tracing::warn!("failed to save provider_order_id: {e}");
            }
            Some(resp)
        }
        Err(e) => {
            tracing::warn!("provider create failed for order {}: {e}", payment_order.document_id);
            return Err(e);
        }
    };

    Ok((payment_order, provider_response))
}

#[allow(clippy::too_many_arguments)]
pub async fn cancel_payment_order(
    pool: &crate::db::Pool,
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
            && let Ok(provider) =
                crate::payment::providers::get_provider(&order.provider, &key)
            && let Err(e) = provider.cancel(&ch, provider_order_id).await
        {
            tracing::warn!(
                "provider cancel failed for order {}: {e}",
                order.document_id
            );
        }
    }

    crate::in_transaction!(pool, tx, {
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            order.id,
            PaymentStatus::Cancelled,
            Some("cancelled_at"),
            PaymentStatus::Pending,
        )
        .await?;
        if rows == 0 {
            return Err(AppError::BadRequest("concurrent_status_change".into()));
        }
        Ok(())
    })?;

    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id().and_then(|s| s.parse::<i64>().ok()),
        Some(auth.role()),
        "payment.order.cancel",
        "payment_order",
        Some(&order.document_id),
        None,
        None,
        None
    );

    Ok(())
}

pub async fn get_payment_order(
    order_repo: &dyn PaymentOrderRepository,
    auth: &AuthUser,
    user_id: i64,
    id: &str,
) -> AppResult<PaymentOrder> {
    let _ = auth.ensure_authenticated()?;
    let order = order_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;
    if auth.role() != "admin" && order.user_id != user_id {
        return Err(AppError::Forbidden);
    }
    Ok(order)
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
            audit_log!(
                audit,
                "",
                None,
                None,
                "payment.callback.failed",
                "payment_channel",
                Some(channel_doc_id),
                Some(&format!("verification_error: {e}")),
                None,
                None
            );
            return Err(e);
        }
    };

    let payment_order = order_repo
        .find_by_provider_order_id(&callback.provider_order_id, None)
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;

    if payment_order.channel_id != channel.id {
        return Err(AppError::BadRequest("channel_order_mismatch".into()));
    }

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
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            payment_order.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await?;

        if rows == 0 {
            tracing::info!(
                "callback for order {} skipped: CAS failed (already processed)",
                payment_order.document_id
            );
            return Ok(());
        }

        if let Some(ref provider_tx_id) = callback.provider_tx_id {
            let tx_doc_id = uuid::Uuid::now_v7().to_string();
            let raw_payload = serde_json::to_string(&callback).ok();
            crate::models::payment_transaction::tx_insert(
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
                payment_order.tenant_id.as_deref(),
            )
            .await?;
        }

        if let Some(ref order_doc_id) = payment_order.order_id
            && let Some(order_id) =
                crate::models::order::tx_find_id_by_document_id(&mut tx, order_doc_id).await?
        {
            crate::models::order::tx_update_status(
                &mut tx,
                order_id,
            crate::models::order::OrderStatus::Paid,
                Some("paid_at"),
            )
            .await?;
        }

        let outbox_doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::wallet_outbox::tx_insert(
            &mut tx,
            &outbox_doc_id,
            payment_order.user_id,
            &payment_order.currency,
            payment_order.amount,
            "credit",
            WalletTxType::Recharge,
            &format!("PAY-{}", payment_order.document_id),
            Some(WalletReferenceType::Payment),
            Some(&payment_order.document_id),
            None,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        Ok(())
    })?;

    let _ = wallet_repo;

    audit_log!(
        audit,
        "",
        None,
        None,
        "payment.callback.success",
        "payment_order",
        Some(&payment_order.document_id),
        None,
        None,
        None
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn refund_payment_order(
    pool: &crate::db::Pool,
    order_repo: &dyn PaymentOrderRepository,
    channel_repo: &dyn PaymentChannelRepository,
    _tx_repo: &dyn PaymentTransactionRepository,
    _refund_repo: &dyn PaymentRefundRepository,
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

    let refund_doc_id = uuid::Uuid::now_v7().to_string();
    let wallet_tx_no = format!("PAYMENT_REFUND_{}", refund_doc_id);

    let refund = crate::in_transaction!(pool, tx, {
        let already_refunded_in_tx =
            crate::models::payment_refund::tx_sum_refunded_by_order(&mut tx, payment_order.id, auth.tenant_id())
                .await?;
        if already_refunded_in_tx + req.amount > payment_order.amount {
            return Err(AppError::BadRequest("refund_exceeds_payment".into()));
        }

        let provider_refund_id =
            if let Some(ref provider_order_id) = payment_order.provider_order_id {
                let key = get_encrypt_key(config)?;
                let channel = channel_repo
                    .find_by_id(payment_order.channel_id, auth.tenant_id())
                    .await?
                    .ok_or_else(|| AppError::not_found("payment_channel"))?;
                let provider =
                    crate::payment::providers::get_provider(&payment_order.provider, &key)?;
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

        crate::models::payment_refund::tx_insert(
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
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        let tx_doc_id = uuid::Uuid::now_v7().to_string();
        let provider_tx_id = format!("txr_{}", uuid::Uuid::now_v7());
        crate::models::payment_transaction::tx_insert(
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
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        let already_refunded_in_tx =
            crate::models::payment_refund::tx_sum_refunded_by_order(&mut tx, payment_order.id, auth.tenant_id())
                .await?;
        let is_full_refund = already_refunded_in_tx >= payment_order.amount;
        let new_status = if is_full_refund {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            payment_order.id,
            new_status,
            None,
            payment_order.status,
        )
        .await?;
        if rows == 0 {
            return Err(AppError::BadRequest("concurrent_status_change".into()));
        }

        let refund = crate::models::payment_refund::tx_find_by_document_id(
            &mut tx,
            &refund_doc_id,
        )
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("inserted refund not found")))?;

        let outbox_doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::wallet_outbox::tx_insert(
            &mut tx,
            &outbox_doc_id,
            payment_order.user_id,
            &payment_order.currency,
            req.amount,
            "debit",
            WalletTxType::Refund,
            &wallet_tx_no,
            Some(WalletReferenceType::PaymentRefund),
            Some(&payment_order.document_id),
            None,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        Ok(refund)
    })?;

    let _ = wallet_repo;

    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id().and_then(|s| s.parse::<i64>().ok()),
        Some(auth.role()),
        "payment.refund.initiated",
        "payment_order",
        Some(&payment_order.document_id),
        Some(&format!("amount={}", req.amount)),
        None,
        None
    );

    if req.amount > 100_000 {
        audit_log!(
            audit,
            auth.tenant_id().unwrap_or(""),
            auth.user_id().and_then(|s| s.parse::<i64>().ok()),
            Some(auth.role()),
            "payment.refund.large",
            "payment_order",
            Some(&payment_order.document_id),
            Some(&format!("amount={} threshold=100000", req.amount)),
            None,
            None
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;
    use crate::models::currencies;
    use crate::models::payment_order::PaymentStatus;
    use crate::repositories::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        currencies::create(&pool, "CNY", "Chinese Yuan", 2)
            .await
            .unwrap();
        currencies::create(&pool, "USD", "US Dollar", 2)
            .await
            .unwrap();
        pool
    }

    fn test_config() -> AppConfig {
        let mut c = AppConfig::test_defaults();
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).unwrap();
        c.app_key = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
        c
    }

    fn admin_auth() -> AuthUser {
        AuthUser::from_parts(
            Some("admin".to_string()),
            Some(1),
            crate::models::user::UserRole::Admin,
            None,
        )
    }

    fn user_auth(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(format!("u{user_int_id}")),
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            None,
        )
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let username = format!("testuser_{doc_id}");
        sqlx::query(
            "INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')",
        )
        .bind(&doc_id)
        .bind(&username)
        .execute(pool)
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE document_id = ?")
            .bind(&doc_id)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_admin(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let username = format!("admin_{doc_id}");
        sqlx::query(
            "INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, ?, 'admin', 'active', 'email')",
        )
        .bind(&doc_id)
        .bind(&username)
        .execute(pool)
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE document_id = ?")
            .bind(&doc_id)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_channel(pool: &crate::db::Pool, provider: &str) -> PaymentChannel {
        let doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::payment_channel::insert(
            pool,
            &doc_id,
            provider,
            &format!("{provider}-test"),
            false,
            r#"{"api_key":"test"}"#,
            None,
            None,
            true,
            0,
            None,
        )
        .await
        .unwrap()
    }

    async fn seed_order(pool: &crate::db::Pool, user_id: i64, amount: i64, currency: &str) -> crate::models::order::Order {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let order_no = format!("ORD-{doc_id}");
        crate::models::order::insert(
            pool,
            &doc_id,
            user_id,
            &order_no,
            amount,
            0,
            0,
            amount,
            currency,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap()
    }

    async fn seed_payment_order(
        pool: &crate::db::Pool,
        user_id: i64,
        channel_id: i64,
        amount: i64,
        currency: &str,
    ) -> PaymentOrder {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let idem_key = format!("idem_{doc_id}");
        crate::models::payment_order::insert(
            pool,
            &doc_id,
            user_id,
            None,
            "Test Payment",
            amount,
            currency,
            channel_id,
            "stripe",
            None,
            None,
            None,
            &idem_key,
            None,
            None,
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_payment_order_rejects_non_owner() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;
        let other_id = seed_user(&pool).await;
        let _admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let order = seed_order(&pool, owner_id, 1000, "CNY").await;

        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_order_repo = SqlxOrderRepository::new(pool.clone());

        let other_auth = user_auth(other_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.document_id.clone(),
            channel_id: channel.document_id.clone(),
            method: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &channel_repo,
            &order_repo,
            &product_repo,
            &order_order_repo,
            &other_auth,
            other_id,
            req,
            &config,
            None,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden => {}
            e => panic!("expected Forbidden, got: {e:?}"),
        }
    }

    #[tokio::test]
    #[cfg(feature = "payment-stripe")]
    async fn create_payment_order_owner_succeeds() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let order = seed_order(&pool, owner_id, 1000, "CNY").await;

        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_order_repo = SqlxOrderRepository::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.document_id.clone(),
            channel_id: channel.document_id.clone(),
            method: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &channel_repo,
            &order_repo,
            &product_repo,
            &order_order_repo,
            &owner_auth,
            owner_id,
            req,
            &config,
            None,
        )
        .await;

        assert!(result.is_ok());
        let (payment_order, _) = result.unwrap();
        assert_eq!(payment_order.user_id, owner_id);
        assert_eq!(payment_order.amount, 1000);
    }

    #[tokio::test]
    async fn create_payment_order_rejects_zero_amount() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "creem").await;
        let order = seed_order(&pool, owner_id, 0, "CNY").await;

        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_order_repo = SqlxOrderRepository::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.document_id.clone(),
            channel_id: channel.document_id.clone(),
            method: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &channel_repo,
            &order_repo,
            &product_repo,
            &order_order_repo,
            &owner_auth,
            owner_id,
            req,
            &config,
            None,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert_eq!(msg, "order_amount_invalid"),
            e => panic!("expected BadRequest, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn get_payment_order_rejects_non_owner() {
        let pool = setup_pool().await;
        let owner_id = seed_user(&pool).await;
        let other_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());

        let other_auth = user_auth(other_id);
        let result =
            super::get_payment_order(&order_repo, &other_auth, other_id, &po.document_id).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden => {}
            e => panic!("expected Forbidden, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn get_payment_order_owner_can_view() {
        let pool = setup_pool().await;
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        let result =
            super::get_payment_order(&order_repo, &owner_auth, owner_id, &po.document_id).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().amount, 500);
    }

    #[tokio::test]
    async fn get_payment_order_admin_can_view_any() {
        let pool = setup_pool().await;
        let owner_id = seed_user(&pool).await;
        let admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());

        let admin_auth_user = AuthUser::from_parts(
            Some(format!("a{admin_id}")),
            Some(admin_id),
            crate::models::user::UserRole::Admin,
            None,
        );
        let result =
            super::get_payment_order(&order_repo, &admin_auth_user, admin_id, &po.document_id)
                .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancel_order_rejects_non_owner() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;
        let other_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let other_auth = user_auth(other_id);
        let result = super::cancel_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &other_auth,
            &audit,
            &config,
            &po.document_id,
            other_id,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Forbidden => {}
            e => panic!("expected Forbidden, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_order_owner_succeeds() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        super::cancel_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &owner_auth,
            &audit,
            &config,
            &po.document_id,
            owner_id,
        )
        .await
        .unwrap();

        let updated = crate::models::payment_order::find_by_id(&pool, po.id, None)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(updated.status, PaymentStatus::Cancelled));
    }

    #[tokio::test]
    async fn cancel_only_pending() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, channel.id, 500, "CNY").await;

        let mut tx = pool.begin().await.unwrap();
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            po.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        assert_eq!(rows, 1);
        tx.commit().await.unwrap();

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        let result = super::cancel_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &owner_auth,
            &audit,
            &config,
            &po.document_id,
            owner_id,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert_eq!(msg, "only_pending_can_cancel"),
            e => panic!("expected BadRequest, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn refund_rejects_non_paid() {
        let pool = setup_pool().await;
        let config = test_config();
        let admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, admin_id, channel.id, 1000, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let tx_repo = SqlxPaymentTransactionRepository::new(pool.clone());
        let refund_repo = SqlxPaymentRefundRepository::new(pool.clone());
        let wallet_repo = SqlxWalletRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let admin_auth_user = AuthUser::from_parts(
            Some(format!("a{admin_id}")),
            Some(admin_id),
            crate::models::user::UserRole::Admin,
            None,
        );
        let result = super::refund_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &tx_repo,
            &refund_repo,
            &wallet_repo,
            &admin_auth_user,
            &audit,
            &config,
            &po.document_id,
            CreateRefundRequest {
                amount: 500,
                reason: Some("test".into()),
            },
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert_eq!(msg, "only_paid_can_refund"),
            e => panic!("expected BadRequest, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn refund_exceeds_payment() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;
        let admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "creem").await;
        let po = seed_payment_order(&pool, user_id, channel.id, 1000, "CNY").await;

        let mut tx = pool.begin().await.unwrap();
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            po.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        assert_eq!(rows, 1);
        tx.commit().await.unwrap();

        let wallet_repo = SqlxWalletRepository::new(pool.clone());
        crate::services::wallet::credit_wallet(
            &wallet_repo,
            &pool,
            user_id,
            "CNY",
            1000,
            WalletTxType::Recharge,
            &format!("PAY-{}", po.document_id),
            Some(WalletReferenceType::Payment),
            Some(&po.document_id),
            None,
        )
        .await
        .unwrap();

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let tx_repo = SqlxPaymentTransactionRepository::new(pool.clone());
        let refund_repo = SqlxPaymentRefundRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let admin_auth_user = AuthUser::from_parts(
            Some(format!("a{admin_id}")),
            Some(admin_id),
            crate::models::user::UserRole::Admin,
            None,
        );
        let result = super::refund_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &tx_repo,
            &refund_repo,
            &wallet_repo,
            &admin_auth_user,
            &audit,
            &config,
            &po.document_id,
            CreateRefundRequest {
                amount: 2000,
                reason: None,
            },
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert_eq!(msg, "refund_exceeds_payment"),
            e => panic!("expected BadRequest, got: {e:?}"),
        }
    }

    #[tokio::test]
    async fn refund_partial_then_full() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;
        let admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "creem").await;
        let po = seed_payment_order(&pool, user_id, channel.id, 1000, "CNY").await;

        let mut tx = pool.begin().await.unwrap();
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            po.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        assert_eq!(rows, 1);
        tx.commit().await.unwrap();

        let wallet_repo = SqlxWalletRepository::new(pool.clone());
        crate::services::wallet::credit_wallet(
            &wallet_repo,
            &pool,
            user_id,
            "CNY",
            1000,
            WalletTxType::Recharge,
            &format!("PAY-{}", po.document_id),
            Some(WalletReferenceType::Payment),
            Some(&po.document_id),
            None,
        )
        .await
        .unwrap();

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let tx_repo = SqlxPaymentTransactionRepository::new(pool.clone());
        let refund_repo = SqlxPaymentRefundRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let admin_auth_user = AuthUser::from_parts(
            Some(format!("a{admin_id}")),
            Some(admin_id),
            crate::models::user::UserRole::Admin,
            None,
        );

        let refund1 = super::refund_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &tx_repo,
            &refund_repo,
            &wallet_repo,
            &admin_auth_user,
            &audit,
            &config,
            &po.document_id,
            CreateRefundRequest {
                amount: 400,
                reason: Some("partial".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(refund1.amount, 400);

        let updated = crate::models::payment_order::find_by_id(&pool, po.id, None)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(updated.status, PaymentStatus::PartiallyRefunded));

        let refund2 = super::refund_payment_order(
            &pool,
            &order_repo,
            &channel_repo,
            &tx_repo,
            &refund_repo,
            &wallet_repo,
            &admin_auth_user,
            &audit,
            &config,
            &po.document_id,
            CreateRefundRequest {
                amount: 600,
                reason: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(refund2.amount, 600);

        let updated = crate::models::payment_order::find_by_id(&pool, po.id, None)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(updated.status, PaymentStatus::Refunded));
    }

    #[tokio::test]
    #[cfg(feature = "payment-stripe")]
    async fn callback_channel_order_mismatch() {
        let pool = setup_pool().await;
        let config = test_config();

        let channel_a = seed_channel(&pool, "stripe").await;
        let channel_b = seed_channel(&pool, "stripe").await;
        let user_id = seed_user(&pool).await;

        let mut po = seed_payment_order(&pool, user_id, channel_a.id, 500, "CNY").await;
        po.provider_order_id = Some("prov_123".to_string());
        crate::models::payment_order::update_provider_order_id(
            &pool,
            po.id,
            "prov_123",
            None,
            None,
        )
        .await
        .unwrap();

        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let tx_repo = SqlxPaymentTransactionRepository::new(pool.clone());
        let wallet_repo = SqlxWalletRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let result = super::handle_callback(
            &pool,
            &channel_repo,
            &order_repo,
            &tx_repo,
            &wallet_repo,
            &audit,
            &config,
            &channel_b.document_id,
            &axum::http::HeaderMap::new(),
            b"test",
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert_eq!(msg, "channel_order_mismatch"),
            e => panic!("expected channel_order_mismatch, got: {e:?}"),
        }
    }

    #[tokio::test]
    #[cfg(feature = "payment-stripe")]
    async fn callback_idempotent_on_paid_order() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, user_id, channel.id, 500, "CNY").await;

        let mut tx = pool.begin().await.unwrap();
        let rows = crate::models::payment_order::tx_update_status_cas(
            &mut tx,
            po.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        assert_eq!(rows, 1);
        tx.commit().await.unwrap();

        let channel_repo = SqlxPaymentChannelRepository::new(pool.clone());
        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let tx_repo = SqlxPaymentTransactionRepository::new(pool.clone());
        let wallet_repo = SqlxWalletRepository::new(pool.clone());
        let audit = AuditService::new(pool.clone());

        let result = super::handle_callback(
            &pool,
            &channel_repo,
            &order_repo,
            &tx_repo,
            &wallet_repo,
            &audit,
            &config,
            &channel.document_id,
            &axum::http::HeaderMap::new(),
            b"test",
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cas_prevents_double_process() {
        let pool = setup_pool().await;
        let user_id = seed_user(&pool).await;
        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, user_id, channel.id, 1000, "CNY").await;

        let result: Result<(), crate::errors::app_error::AppError> = async {
            crate::in_transaction!(pool, tx, {
                let rows = crate::models::payment_order::tx_update_status_cas(
                    &mut tx,
                    po.id,
                    PaymentStatus::Paid,
                    Some("paid_at"),
                    PaymentStatus::Pending,
                )
                .await
                .unwrap();
                assert_eq!(rows, 1);

                let rows2 = crate::models::payment_order::tx_update_status_cas(
                    &mut tx,
                    po.id,
                    PaymentStatus::Paid,
                    Some("paid_at"),
                    PaymentStatus::Pending,
                )
                .await
                .unwrap();
                assert_eq!(rows2, 0);

                Ok(())
            })
        }
        .await;
        result.unwrap();
    }

    #[tokio::test]
    async fn idempotency_key_dedup() {
        let pool = setup_pool().await;
        let user_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "creem").await;
        let order = seed_order(&pool, user_id, 1000, "CNY").await;

        let order_repo = SqlxPaymentOrderRepository::new(pool.clone());
        let idem_key = format!("{}_{}", order.document_id, channel.document_id);

        let doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::payment_order::insert(
            &pool,
            &doc_id,
            user_id,
            Some(&order.document_id),
            "Test",
            1000,
            "CNY",
            channel.id,
            "creem",
            None,
            None,
            None,
            &idem_key,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let found = order_repo
            .find_by_idempotency_key(&idem_key, None)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().document_id, doc_id);
    }
}
