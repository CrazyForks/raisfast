use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::commands::{
    CreatePaymentChannelCmd, CreatePaymentOrderCmd, CreatePaymentRefundCmd,
    CreatePaymentTransactionCmd, CreateWalletOutboxCmd,
};
use crate::config::app::AppConfig;
use crate::dto::payment::*;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::payment_channel::PaymentChannel;
use crate::models::payment_order::{PaymentOrder, PaymentStatus};
use crate::models::payment_refund::PaymentRefund;
use crate::models::payment_transaction::PaymentTransaction;
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::payment::ProviderResponse;
use crate::payment::routing::{RoutingContext, select_best_channel, select_channels};
use crate::services::audit::AuditService;
use crate::types::snowflake_id::SnowflakeId;
use base64::Engine;

#[async_trait]
pub trait PaymentService: Send + Sync {
    async fn create_channel(
        &self,
        auth: &AuthUser,
        req: CreatePaymentChannelRequest,
    ) -> AppResult<PaymentChannel>;
    async fn update_channel(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdatePaymentChannelRequest,
    ) -> AppResult<PaymentChannel>;
    async fn delete_channel(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
    async fn get_channel(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<PaymentChannel>;
    async fn list_channels(&self, auth: &AuthUser) -> AppResult<Vec<PaymentChannel>>;
    async fn list_available_channels(
        &self,
        auth: &AuthUser,
        order_id: &str,
        country: Option<&str>,
        language: Option<&str>,
    ) -> AppResult<AvailableChannelsResponse>;
    async fn create_payment_order(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        req: CreatePaymentOrderRequest,
        client_ip: Option<&str>,
        client_language: Option<&str>,
        client_user_agent: Option<&str>,
    ) -> AppResult<(PaymentOrder, Option<ProviderResponse>)>;
    async fn cancel_payment_order(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()>;
    async fn get_payment_order(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        id: SnowflakeId,
    ) -> AppResult<PaymentOrder>;
    async fn list_user_payment_orders(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentOrder>, i64)>;
    async fn handle_callback(
        &self,
        channel_id: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<PaymentOrder>;
    async fn refund_payment_order(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: CreateRefundRequest,
    ) -> AppResult<PaymentRefund>;
    async fn list_admin_payment_orders(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<PaymentOrder>, i64)>;
    async fn list_admin_transactions(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentTransaction>, i64)>;
    async fn list_admin_refunds(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentRefund>, i64)>;
    async fn list_admin_channels(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentChannel>, i64)>;
    async fn list_order_transactions(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        order_id: SnowflakeId,
    ) -> AppResult<Vec<PaymentTransaction>>;
    async fn list_order_refunds(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        order_id: SnowflakeId,
    ) -> AppResult<Vec<PaymentRefund>>;
}

pub struct PaymentServiceImpl {
    config: Arc<AppConfig>,
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

impl PaymentServiceImpl {
    pub fn new(
        config: Arc<AppConfig>,
        aspect_engine: Arc<AspectEngine>,
        pool: Arc<crate::db::Pool>,
    ) -> Self {
        Self {
            config,
            aspect_engine,
            pool,
        }
    }

    fn after_payment_order_created(&self, order: &PaymentOrder) {
        self.aspect_engine
            .emit(Event::PaymentOrderCreated(order.clone()));
    }

    fn after_payment_paid(&self, order: &PaymentOrder) {
        self.aspect_engine.emit(Event::PaymentPaid(order.clone()));
    }

    fn after_payment_refunded(&self, order: &PaymentOrder) {
        self.aspect_engine
            .emit(Event::PaymentRefunded(order.clone()));
    }
}

#[async_trait]
impl PaymentService for PaymentServiceImpl {
    async fn create_channel(
        &self,
        auth: &AuthUser,
        req: CreatePaymentChannelRequest,
    ) -> AppResult<PaymentChannel> {
        let audit = AuditService::new((*self.pool).clone());
        create_channel(&self.pool, auth, &self.config, &audit, req).await
    }

    async fn update_channel(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdatePaymentChannelRequest,
    ) -> AppResult<PaymentChannel> {
        let audit = AuditService::new((*self.pool).clone());
        update_channel(&self.pool, auth, &self.config, &audit, id, req).await
    }

    async fn delete_channel(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        let audit = AuditService::new((*self.pool).clone());
        delete_channel(&self.pool, auth, &audit, id).await
    }

    async fn get_channel(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<PaymentChannel> {
        get_channel(&self.pool, auth, id).await
    }

    async fn list_channels(&self, auth: &AuthUser) -> AppResult<Vec<PaymentChannel>> {
        list_channels(&self.pool, auth).await
    }

    async fn list_available_channels(
        &self,
        auth: &AuthUser,
        order_id: &str,
        country: Option<&str>,
        language: Option<&str>,
    ) -> AppResult<AvailableChannelsResponse> {
        list_available_channels(&self.pool, auth, order_id, country, language).await
    }

    async fn create_payment_order(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        req: CreatePaymentOrderRequest,
        client_ip: Option<&str>,
        client_language: Option<&str>,
        client_user_agent: Option<&str>,
    ) -> AppResult<(PaymentOrder, Option<ProviderResponse>)> {
        let (order, resp) = create_payment_order(
            &self.pool,
            auth,
            user_id,
            req,
            &self.config,
            client_ip,
            client_language,
            client_user_agent,
        )
        .await?;
        self.after_payment_order_created(&order);
        Ok((order, resp))
    }

    async fn cancel_payment_order(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()> {
        let audit = AuditService::new((*self.pool).clone());
        cancel_payment_order(&self.pool, auth, &audit, &self.config, id, user_id).await
    }

    async fn get_payment_order(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        id: SnowflakeId,
    ) -> AppResult<PaymentOrder> {
        get_payment_order(&self.pool, auth, user_id, id).await
    }

    async fn list_user_payment_orders(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentOrder>, i64)> {
        list_user_payment_orders(&self.pool, auth, user_id, page, page_size).await
    }

    async fn handle_callback(
        &self,
        channel_id: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<PaymentOrder> {
        let audit = AuditService::new((*self.pool).clone());
        let order =
            handle_callback(&self.pool, &audit, &self.config, channel_id, headers, body).await?;
        self.after_payment_paid(&order);
        Ok(order)
    }

    async fn refund_payment_order(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: CreateRefundRequest,
    ) -> AppResult<PaymentRefund> {
        let audit = AuditService::new((*self.pool).clone());
        let refund = refund_payment_order(&self.pool, auth, &audit, &self.config, id, req).await?;
        if let Ok(Some(order)) =
            crate::models::payment_order::find_by_id(&self.pool, id, auth.tenant_id()).await
        {
            self.after_payment_refunded(&order);
        }
        Ok(refund)
    }

    async fn list_admin_payment_orders(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<PaymentOrder>, i64)> {
        list_admin_payment_orders(&self.pool, auth, page, page_size, status).await
    }

    async fn list_admin_transactions(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentTransaction>, i64)> {
        list_admin_transactions(&self.pool, auth, page, page_size).await
    }

    async fn list_admin_refunds(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentRefund>, i64)> {
        list_admin_refunds(&self.pool, auth, page, page_size).await
    }

    async fn list_admin_channels(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<PaymentChannel>, i64)> {
        crate::models::payment_channel::find_all_admin_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            None,
        )
        .await
    }

    async fn list_order_transactions(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        order_id: SnowflakeId,
    ) -> AppResult<Vec<PaymentTransaction>> {
        let order = self.get_payment_order(auth, user_id, order_id).await?;
        crate::models::payment_transaction::find_by_payment_order_id(
            &self.pool,
            order.id,
            auth.tenant_id(),
        )
        .await
    }

    async fn list_order_refunds(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        order_id: SnowflakeId,
    ) -> AppResult<Vec<PaymentRefund>> {
        let order = self.get_payment_order(auth, user_id, order_id).await?;
        crate::models::payment_refund::find_by_payment_order_id(
            &self.pool,
            order.id,
            auth.tenant_id(),
        )
        .await
    }
}

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
    pool: &crate::db::Pool,
    auth: &AuthUser,
    config: &AppConfig,
    audit: &AuditService,
    req: CreatePaymentChannelRequest,
) -> AppResult<PaymentChannel> {
    let encrypted_credentials = encrypt_credential(&req.credentials, config)?;
    let encrypted_webhook_secret = req
        .webhook_secret
        .as_deref()
        .map(|s| encrypt_credential(s, config))
        .transpose()?;
    let cmd = CreatePaymentChannelCmd {
        provider: req.provider,
        name: req.name,
        is_live: req.is_live.unwrap_or(false),
        credentials: encrypted_credentials,
        webhook_secret: encrypted_webhook_secret,
        settings: req.settings,
        is_active: true,
        sort_order: req.sort_order.unwrap_or(0),
    };
    let channel = crate::models::payment_channel::insert(pool, &cmd, auth.tenant_id()).await?;
    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id(),
        Some(auth.role()),
        "payment.channel.create",
        "payment_channel",
        Some(&channel.id.to_string()),
        None,
        None,
        None
    );
    Ok(channel)
}

pub async fn update_channel(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    config: &AppConfig,
    audit: &AuditService,
    id: SnowflakeId,
    req: UpdatePaymentChannelRequest,
) -> AppResult<PaymentChannel> {
    let channel = crate::models::payment_channel::find_by_id(pool, id, auth.tenant_id())
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

    let updated = crate::models::payment_channel::update(
        pool,
        &crate::commands::UpdatePaymentChannelCmd {
            id: channel.id,
            provider: channel.provider.clone(),
            name: req.name.clone().unwrap_or(channel.name.clone()),
            is_live: req.is_live.unwrap_or(channel.is_live != 0),
            credentials: encrypted_credentials,
            webhook_secret: encrypted_webhook_secret,
            settings: req.settings.clone().or(channel.settings.clone()),
            is_active: req.is_active.unwrap_or(channel.is_active != 0),
            sort_order: req.sort_order.unwrap_or(channel.sort_order),
            version: req.version,
        },
        auth.tenant_id(),
    )
    .await?;

    if !updated {
        return Err(AppError::Conflict("version_conflict".into()));
    }

    let result = crate::models::payment_channel::find_by_id(pool, channel.id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;

    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id(),
        Some(auth.role()),
        "payment.channel.update",
        "payment_channel",
        Some(&result.id.to_string()),
        None,
        None,
        None
    );

    Ok(result)
}

pub async fn delete_channel(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    audit: &AuditService,
    id: SnowflakeId,
) -> AppResult<()> {
    let channel = crate::models::payment_channel::find_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))?;
    let deleted =
        crate::models::payment_channel::delete_by_id(pool, channel.id, auth.tenant_id()).await?;
    if !deleted {
        return Err(AppError::not_found("payment_channel"));
    }
    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id(),
        Some(auth.role()),
        "payment.channel.delete",
        "payment_channel",
        Some(&channel.id.to_string()),
        None,
        None,
        None
    );
    Ok(())
}

pub async fn get_channel(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    id: SnowflakeId,
) -> AppResult<PaymentChannel> {
    crate::models::payment_channel::find_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_channel"))
}

pub async fn list_channels(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<PaymentChannel>> {
    crate::models::payment_channel::find_all_active(pool, auth.tenant_id()).await
}

pub async fn list_available_channels(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    order_id: &str,
    country: Option<&str>,
    language: Option<&str>,
) -> AppResult<AvailableChannelsResponse> {
    auth.ensure_snowflake_user_id()?;

    let order_id_parsed = crate::types::snowflake_id::parse_id(order_id)?;
    let order = crate::models::order::find_by_id(pool, order_id_parsed, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    let active = crate::models::payment_channel::find_all_active(pool, auth.tenant_id()).await?;
    let ctx = RoutingContext {
        currency: order.currency.clone(),
        country: country.map(String::from),
        language: language.map(String::from),
    };
    let ranked = select_channels(&active, &ctx);

    let recommended_channel_id = ranked
        .iter()
        .find(|r| r.is_recommended)
        .map(|r| r.channel.id.to_string());

    let channels = ranked
        .into_iter()
        .map(|r| AvailableChannelItem {
            channel_id: r.channel.id.to_string(),
            provider: r.channel.provider,
            name: r.channel.name,
            is_recommended: r.is_recommended,
            sort_order: r.channel.sort_order,
        })
        .collect();

    Ok(AvailableChannelsResponse {
        recommended_channel_id,
        channels,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_payment_order(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    user_id: SnowflakeId,
    req: CreatePaymentOrderRequest,
    config: &AppConfig,
    client_ip: Option<&str>,
    client_language: Option<&str>,
    client_user_agent: Option<&str>,
) -> AppResult<(PaymentOrder, Option<ProviderResponse>)> {
    auth.ensure_snowflake_user_id()?;

    let order_id_parsed = crate::types::snowflake_id::parse_id(&req.order_id)?;
    let order = crate::models::order::find_by_id(pool, order_id_parsed, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.user_id != user_id {
        return Err(AppError::Forbidden);
    }

    if order.total_amount <= 0 {
        return Err(AppError::BadRequest("order_amount_invalid".into()));
    }

    let (channel, channel_selected_by) = if let Some(ref ch_id_str) = req.channel_id {
        let ch_id = crate::types::snowflake_id::parse_id(ch_id_str)?;
        let ch = crate::models::payment_channel::find_by_id(pool, ch_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("payment_channel"))?;
        if ch.is_active == 0 {
            return Err(AppError::BadRequest("channel_inactive".into()));
        }
        (ch, "manual")
    } else {
        let active =
            crate::models::payment_channel::find_all_active(pool, auth.tenant_id()).await?;
        let ctx = RoutingContext {
            currency: order.currency.clone(),
            country: req.country.clone(),
            language: req
                .language
                .clone()
                .or_else(|| client_language.map(String::from)),
        };
        let ch = select_best_channel(&active, &ctx)?;
        (ch, "auto")
    };

    let idempotency_key = format!("{}_{}", order.id, channel.id);

    if let Some(existing) =
        crate::models::payment_order::find_by_idempotency_key(pool, &idempotency_key, None).await?
    {
        tracing::info!(
            "idempotent payment order found: id={}, status={:?}",
            existing.id,
            existing.status
        );
        if existing.status == PaymentStatus::Pending {
            let key = get_encrypt_key(config)?;
            let provider = crate::payment::providers::get_provider(&channel.provider, &key)?;
            let notify_url = format!("{}/api/v1/payment/callback/{}", config.base_url, channel.id);
            let provider_resp = provider
                .create(
                    &channel,
                    &existing,
                    req.return_url.as_deref(),
                    Some(&notify_url),
                )
                .await
                .map_err(|e| {
                    tracing::error!("creem create retry failed: {e}");
                    e
                })
                .ok();
            tracing::info!(
                "creem retry provider_resp: redirect_url={:?}",
                provider_resp.as_ref().and_then(|r| r.redirect_url.as_ref())
            );
            if let Some(ref resp) = provider_resp
                && let Err(e) = crate::models::payment_order::update_provider_order_id(
                    pool,
                    existing.id,
                    &resp.provider_order_id,
                    None,
                    auth.tenant_id(),
                )
                .await
            {
                tracing::warn!("failed to update provider_order_id on retry: {e}");
            }
            return Ok((existing, provider_resp));
        }
        return Ok((existing, None));
    }

    let title = format!("Order {}", order.order_no);

    let cmd = CreatePaymentOrderCmd {
        user_id,
        order_id: Some(order.id.to_string()),
        title,
        amount: order.total_amount,
        currency: order.currency.clone(),
        channel_id: channel.id,
        provider: channel.provider.clone(),
        reference_type: None,
        reference_id: None,
        return_url: req.return_url.clone(),
        idempotency_key: idempotency_key.clone(),
        client_ip: client_ip.map(String::from),
        client_language: client_language.map(String::from),
        client_country: req.country.clone(),
        client_user_agent: client_user_agent.map(String::from),
        channel_selected_by: Some(channel_selected_by.into()),
        metadata: req.metadata.clone(),
    };

    let payment_order =
        match crate::models::payment_order::insert(pool, &cmd, auth.tenant_id()).await {
            Ok(po) => po,
            Err(e) => {
                if is_unique_violation(&e)
                    && let Some(existing) = crate::models::payment_order::find_by_idempotency_key(
                        pool,
                        &idempotency_key,
                        None,
                    )
                    .await?
                {
                    return Ok((existing, None));
                }
                return Err(e);
            }
        };

    let key = get_encrypt_key(config)?;
    let provider = crate::payment::providers::get_provider(&channel.provider, &key)?;
    let notify_url = format!("{}/api/v1/payment/callback/{}", config.base_url, channel.id);
    let provider_response = match provider
        .create(
            &channel,
            &payment_order,
            req.return_url.as_deref(),
            Some(&notify_url),
        )
        .await
    {
        Ok(resp) => {
            if let Err(e) = crate::models::payment_order::update_provider_order_id(
                pool,
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
            tracing::warn!(
                "provider create failed for order {}: {e}",
                payment_order.id.to_string()
            );
            return Err(e);
        }
    };

    Ok((payment_order, provider_response))
}

#[allow(clippy::too_many_arguments)]
pub async fn cancel_payment_order(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    audit: &AuditService,
    config: &AppConfig,
    id: SnowflakeId,
    user_id: SnowflakeId,
) -> AppResult<()> {
    auth.ensure_snowflake_user_id()?;
    let order = crate::models::payment_order::find_by_id(pool, id, auth.tenant_id())
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
        let channel =
            crate::models::payment_channel::find_by_id(pool, order.channel_id, auth.tenant_id())
                .await?;
        if let Some(ch) = channel
            && let Ok(provider) = crate::payment::providers::get_provider(&order.provider, &key)
            && let Err(e) = provider.cancel(&ch, provider_order_id).await
        {
            tracing::warn!(
                "provider cancel failed for order {}: {e}",
                order.id.to_string()
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
        auth.user_id(),
        Some(auth.role()),
        "payment.order.cancel",
        "payment_order",
        Some(&order.id.to_string()),
        None,
        None,
        None
    );

    Ok(())
}

pub async fn get_payment_order(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    user_id: SnowflakeId,
    id: SnowflakeId,
) -> AppResult<PaymentOrder> {
    auth.ensure_snowflake_user_id()?;
    let order = crate::models::payment_order::find_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;
    if !auth.is_admin() && order.user_id != user_id {
        return Err(AppError::Forbidden);
    }
    Ok(order)
}

pub async fn list_user_payment_orders(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    user_id: SnowflakeId,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    auth.ensure_snowflake_user_id()?;
    crate::models::payment_order::find_by_user_paginated(
        pool,
        user_id,
        auth.tenant_id(),
        page,
        page_size,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_callback(
    pool: &crate::db::Pool,
    audit: &AuditService,
    config: &AppConfig,
    channel_id: &str,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> AppResult<PaymentOrder> {
    let ch_id = crate::types::snowflake_id::parse_id(channel_id)?;
    let channel = crate::models::payment_channel::find_by_id(pool, ch_id, None)
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
                Some(channel_id),
                Some(&format!("verification_error: {e}")),
                None,
                None
            );
            return Err(e);
        }
    };

    let payment_order = crate::models::payment_order::find_by_provider_order_id(
        pool,
        &callback.provider_order_id,
        None,
    )
    .await?
    .ok_or_else(|| AppError::not_found("payment_order"))?;

    if payment_order.channel_id != channel.id {
        return Err(AppError::BadRequest("channel_order_mismatch".into()));
    }

    if payment_order.status == PaymentStatus::Paid {
        return Ok(payment_order);
    }

    if payment_order.status != PaymentStatus::Pending {
        return Err(AppError::BadRequest("order_not_pending".into()));
    }

    if callback.amount > 0 && callback.amount != payment_order.amount {
        tracing::warn!(
            "callback amount mismatch: expected={}, got={} (provider may add tax as MoR)",
            payment_order.amount,
            callback.amount
        );
    }

    if callback.status != PaymentStatus::Paid {
        return Ok(payment_order);
    }

    if let Some(ref provider_tx_id) = callback.provider_tx_id
        && crate::models::payment_transaction::find_by_provider_tx_id(pool, provider_tx_id, None)
            .await?
            .is_some()
    {
        return Ok(payment_order);
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
                payment_order.id.to_string()
            );
        }

        if let Some(ref provider_tx_id) = callback.provider_tx_id {
            if crate::models::payment_transaction::find_by_provider_tx_id(
                pool,
                provider_tx_id,
                payment_order.tenant_id.as_deref(),
            )
            .await?
            .is_some()
            {
                tracing::info!(
                    "transaction already exists for provider_tx_id={provider_tx_id}, skipping"
                );
            } else {
                let raw_payload = serde_json::to_string(&callback).ok();
                let tx_cmd = CreatePaymentTransactionCmd {
                    payment_order_id: payment_order.id,
                    order_id: payment_order.order_id.clone(),
                    user_id: payment_order.user_id,
                    tx_type: "charge".into(),
                    amount: payment_order.amount,
                    currency: payment_order.currency.clone(),
                    provider_tx_id: provider_tx_id.clone(),
                    status: "succeeded".into(),
                    raw_payload,
                };
                crate::models::payment_transaction::tx_insert(
                    &mut tx,
                    &tx_cmd,
                    payment_order.tenant_id.as_deref(),
                )
                .await?;
            }
        }

        if let Some(ref order_id_str) = payment_order.order_id
            && let Ok(order_id) = order_id_str.parse::<i64>()
        {
            crate::models::order::tx_update_status_cas(
                &mut tx,
                SnowflakeId(order_id),
                crate::models::order::OrderStatus::Paid,
                Some("paid_at"),
                crate::models::order::OrderStatus::Pending,
            )
            .await?;
        }

        let outbox_cmd = CreateWalletOutboxCmd {
            user_id: payment_order.user_id,
            currency: payment_order.currency.clone(),
            amount: payment_order.amount,
            entry_type: "credit".into(),
            tx_type: WalletTxType::Recharge,
            transaction_no: format!("PAY-{}", payment_order.id),
            reference_type: Some(WalletReferenceType::Payment),
            reference_id: Some(payment_order.id.to_string()),
            metadata: None,
        };
        crate::models::wallet_outbox::tx_insert(
            &mut tx,
            &outbox_cmd,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        Ok(())
    })?;

    audit_log!(
        audit,
        "",
        None,
        None,
        "payment.callback.success",
        "payment_order",
        Some(&payment_order.id.to_string()),
        None,
        None,
        None
    );

    Ok(payment_order)
}

#[allow(clippy::too_many_arguments)]
pub async fn refund_payment_order(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    audit: &AuditService,
    config: &AppConfig,
    id: SnowflakeId,
    req: CreateRefundRequest,
) -> AppResult<PaymentRefund> {
    raisfast_derive::check_schema!("payment_refunds", "provider_refund_id");
    let payment_order = crate::models::payment_order::find_by_id(pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("payment_order"))?;

    if payment_order.status != PaymentStatus::Paid
        && payment_order.status != PaymentStatus::PartiallyRefunded
    {
        return Err(AppError::BadRequest("only_paid_can_refund".into()));
    }

    let wallet_tx_no = format!("PAYMENT_REFUND_{}", uuid::Uuid::now_v7());

    let refund = crate::in_transaction!(pool, tx, {
        let already_refunded_in_tx = crate::models::payment_refund::tx_sum_refunded_by_order(
            &mut tx,
            payment_order.id,
            auth.tenant_id(),
        )
        .await?;
        if already_refunded_in_tx
            .checked_add(req.amount)
            .ok_or_else(|| AppError::BadRequest("refund_amount_overflow".into()))?
            > payment_order.amount
        {
            return Err(AppError::BadRequest("refund_exceeds_payment".into()));
        }

        let provider_refund_id = if let Some(ref provider_order_id) =
            payment_order.provider_order_id
        {
            let key = get_encrypt_key(config)?;
            let channel = crate::models::payment_channel::find_by_id(
                pool,
                payment_order.channel_id,
                auth.tenant_id(),
            )
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

        let refund_cmd = CreatePaymentRefundCmd {
            payment_order_id: payment_order.id,
            order_id: payment_order.order_id.clone(),
            user_id: payment_order.user_id,
            amount: req.amount,
            currency: payment_order.currency.clone(),
            reason: req.reason.clone(),
            provider_refund_id: Some(provider_refund_id.clone()),
            status: "succeeded".into(),
            payment_tx_id: None,
            metadata: None,
        };
        crate::models::payment_refund::tx_insert(
            &mut tx,
            &refund_cmd,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        let provider_tx_id = format!("txr_{}", uuid::Uuid::now_v7());
        let tx_cmd = CreatePaymentTransactionCmd {
            payment_order_id: payment_order.id,
            order_id: payment_order.order_id.clone(),
            user_id: payment_order.user_id,
            tx_type: "refund".into(),
            amount: req.amount,
            currency: payment_order.currency.clone(),
            provider_tx_id,
            status: "succeeded".into(),
            raw_payload: None,
        };
        crate::models::payment_transaction::tx_insert(
            &mut tx,
            &tx_cmd,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        let already_refunded_in_tx = crate::models::payment_refund::tx_sum_refunded_by_order(
            &mut tx,
            payment_order.id,
            auth.tenant_id(),
        )
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

        let refund = crate::models::payment_refund::tx_find_by_provider_refund_id(
            &mut tx,
            &provider_refund_id,
        )
        .await?;

        let outbox_cmd = CreateWalletOutboxCmd {
            user_id: payment_order.user_id,
            currency: payment_order.currency.clone(),
            amount: req.amount,
            entry_type: "debit".into(),
            tx_type: WalletTxType::Refund,
            transaction_no: wallet_tx_no,
            reference_type: Some(WalletReferenceType::PaymentRefund),
            reference_id: Some(payment_order.id.to_string()),
            metadata: None,
        };
        crate::models::wallet_outbox::tx_insert(
            &mut tx,
            &outbox_cmd,
            payment_order.tenant_id.as_deref(),
        )
        .await?;

        Ok(refund)
    })?;

    audit_log!(
        audit,
        auth.tenant_id().unwrap_or(""),
        auth.user_id(),
        Some(auth.role()),
        "payment.refund.initiated",
        "payment_order",
        Some(&payment_order.id.to_string()),
        Some(&format!("amount={}", req.amount)),
        None,
        None
    );

    if req.amount > 100_000 {
        audit_log!(
            audit,
            auth.tenant_id().unwrap_or(""),
            auth.user_id(),
            Some(auth.role()),
            "payment.refund.large",
            "payment_order",
            Some(&payment_order.id.to_string()),
            Some(&format!("amount={} threshold=100000", req.amount)),
            None,
            None
        );
    }

    Ok(refund)
}

pub async fn list_admin_payment_orders(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    crate::models::payment_order::find_all_admin_paginated(
        pool,
        auth.tenant_id(),
        page,
        page_size,
        status,
    )
    .await
}

pub async fn list_admin_transactions(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentTransaction>, i64)> {
    crate::models::payment_transaction::find_all_admin_paginated(
        pool,
        auth.tenant_id(),
        page,
        page_size,
    )
    .await
}

pub async fn list_admin_refunds(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentRefund>, i64)> {
    crate::models::payment_refund::find_all_admin_paginated(pool, auth.tenant_id(), page, page_size)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreateOrderCmd;
    use crate::config::app::AppConfig;
    use crate::models::payment_order::PaymentStatus;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let _ = crate::models::currencies::create(&pool, "default", "CNY", "Chinese Yuan", 2).await;
        let _ = crate::models::currencies::create(&pool, "default", "USD", "US Dollar", 2).await;
        pool
    }

    fn test_config() -> AppConfig {
        let mut c = AppConfig::test_defaults();
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).unwrap();
        c.app_key = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
        c
    }

    #[allow(dead_code)]
    fn admin_auth() -> AuthUser {
        AuthUser::from_parts(Some(1), crate::models::user::UserRole::Admin, None)
    }

    fn user_auth(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            None,
        )
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

    async fn seed_admin(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("admin_{id}");
        sqlx::query(
            "INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'admin', 'active', 'email')",
        )
        .bind(id)
        .bind(&username)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_channel(pool: &crate::db::Pool, provider: &str) -> PaymentChannel {
        crate::models::payment_channel::insert(
            pool,
            &CreatePaymentChannelCmd {
                provider: provider.into(),
                name: format!("{provider}-test"),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: None,
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap()
    }

    async fn seed_order(
        pool: &crate::db::Pool,
        user_id: i64,
        amount: i64,
        currency: &str,
    ) -> crate::models::order::Order {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        crate::models::order::insert(
            pool,
            &CreateOrderCmd {
                user_id: SnowflakeId(user_id),
                order_no,
                subtotal: amount,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: amount,
                currency: currency.into(),
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
            },
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
        let idem_key = format!("idem_{}", uuid::Uuid::now_v7());
        crate::models::payment_order::insert(
            pool,
            &CreatePaymentOrderCmd {
                user_id: SnowflakeId(user_id),
                order_id: None,
                title: "Test Payment".into(),
                amount,
                currency: currency.into(),
                channel_id: SnowflakeId(channel_id),
                provider: "stripe".into(),
                reference_type: None,
                reference_id: None,
                return_url: None,
                idempotency_key: idem_key,
                client_ip: None,
                client_language: None,
                client_country: None,
                client_user_agent: None,
                channel_selected_by: None,
                metadata: None,
            },
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

        let other_auth = user_auth(other_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: Some(channel.id.to_string()),
            method: None,
            country: None,
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &other_auth,
            SnowflakeId(other_id),
            req,
            &config,
            None,
            None,
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

        let owner_auth = user_auth(owner_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: Some(channel.id.to_string()),
            method: None,
            country: None,
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &owner_auth,
            SnowflakeId(owner_id),
            req,
            &config,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
        let (payment_order, _) = result.unwrap();
        assert_eq!(payment_order.user_id, SnowflakeId(owner_id));
        assert_eq!(payment_order.amount, 1000);
    }

    #[tokio::test]
    async fn create_payment_order_rejects_zero_amount() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "creem").await;
        let order = seed_order(&pool, owner_id, 0, "CNY").await;

        let owner_auth = user_auth(owner_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: Some(channel.id.to_string()),
            method: None,
            country: None,
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &owner_auth,
            SnowflakeId(owner_id),
            req,
            &config,
            None,
            None,
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
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

        let other_auth = user_auth(other_id);
        let result =
            super::get_payment_order(&pool, &other_auth, SnowflakeId(other_id), po.id).await;

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
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

        let owner_auth = user_auth(owner_id);
        let result =
            super::get_payment_order(&pool, &owner_auth, SnowflakeId(owner_id), po.id).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().amount, 500);
    }

    #[tokio::test]
    async fn get_payment_order_admin_can_view_any() {
        let pool = setup_pool().await;
        let owner_id = seed_user(&pool).await;
        let admin_id = seed_admin(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

        let admin_auth_user =
            AuthUser::from_parts(Some(admin_id), crate::models::user::UserRole::Admin, None);
        let result =
            super::get_payment_order(&pool, &admin_auth_user, SnowflakeId(admin_id), po.id).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cancel_order_rejects_non_owner() {
        let pool = setup_pool().await;
        let config = test_config();
        let owner_id = seed_user(&pool).await;
        let other_id = seed_user(&pool).await;

        let channel = seed_channel(&pool, "stripe").await;
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

        let audit = AuditService::new(pool.clone());

        let other_auth = user_auth(other_id);
        let result = super::cancel_payment_order(
            &pool,
            &other_auth,
            &audit,
            &config,
            po.id,
            SnowflakeId(other_id),
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
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

        let audit = AuditService::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        super::cancel_payment_order(
            &pool,
            &owner_auth,
            &audit,
            &config,
            po.id,
            SnowflakeId(owner_id),
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
        let po = seed_payment_order(&pool, owner_id, *channel.id, 500, "CNY").await;

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

        let audit = AuditService::new(pool.clone());

        let owner_auth = user_auth(owner_id);
        let result = super::cancel_payment_order(
            &pool,
            &owner_auth,
            &audit,
            &config,
            po.id,
            SnowflakeId(owner_id),
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
        let po = seed_payment_order(&pool, admin_id, *channel.id, 1000, "CNY").await;

        let audit = AuditService::new(pool.clone());

        let admin_auth_user =
            AuthUser::from_parts(Some(admin_id), crate::models::user::UserRole::Admin, None);
        let result = super::refund_payment_order(
            &pool,
            &admin_auth_user,
            &audit,
            &config,
            po.id,
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
        let po = seed_payment_order(&pool, user_id, *channel.id, 1000, "CNY").await;

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

        let wallet_auth =
            AuthUser::from_parts(Some(user_id), crate::models::user::UserRole::Reader, None);
        crate::services::wallet::credit_wallet(
            &pool,
            &wallet_auth,
            "CNY",
            1000,
            WalletTxType::Recharge,
            &format!("PAY-{}", po.id),
            Some(WalletReferenceType::Payment),
            Some(&po.id.to_string()),
            None,
        )
        .await
        .unwrap();

        let audit = AuditService::new(pool.clone());

        let admin_auth_user =
            AuthUser::from_parts(Some(admin_id), crate::models::user::UserRole::Admin, None);
        let result = super::refund_payment_order(
            &pool,
            &admin_auth_user,
            &audit,
            &config,
            po.id,
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
        let po = seed_payment_order(&pool, user_id, *channel.id, 1000, "CNY").await;

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

        let wallet_auth =
            AuthUser::from_parts(Some(user_id), crate::models::user::UserRole::Reader, None);
        crate::services::wallet::credit_wallet(
            &pool,
            &wallet_auth,
            "CNY",
            1000,
            WalletTxType::Recharge,
            &format!("PAY-{}", po.id),
            Some(WalletReferenceType::Payment),
            Some(&po.id.to_string()),
            None,
        )
        .await
        .unwrap();

        let audit = AuditService::new(pool.clone());

        let admin_auth_user =
            AuthUser::from_parts(Some(admin_id), crate::models::user::UserRole::Admin, None);

        let refund1 = super::refund_payment_order(
            &pool,
            &admin_auth_user,
            &audit,
            &config,
            po.id,
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
            &admin_auth_user,
            &audit,
            &config,
            po.id,
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
            &pool, po.id, "prov_123", None, None,
        )
        .await
        .unwrap();

        let audit = AuditService::new(pool.clone());

        let result = super::handle_callback(
            &pool,
            &audit,
            &config,
            &channel_b.id.to_string(),
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
        let po = seed_payment_order(&pool, user_id, *channel.id, 500, "CNY").await;

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

        let audit = AuditService::new(pool.clone());

        let result = super::handle_callback(
            &pool,
            &audit,
            &config,
            &channel.id.to_string(),
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
        let po = seed_payment_order(&pool, user_id, *channel.id, 1000, "CNY").await;

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

        let idem_key = format!("{}_{}", order.id, channel.id);

        crate::models::payment_order::insert(
            &pool,
            &CreatePaymentOrderCmd {
                user_id: SnowflakeId(user_id),
                order_id: Some(order.id.to_string()),
                title: "Test".into(),
                amount: 1000,
                currency: "CNY".into(),
                channel_id: channel.id,
                provider: "creem".into(),
                reference_type: None,
                reference_id: None,
                return_url: None,
                idempotency_key: idem_key.clone(),
                client_ip: None,
                client_language: None,
                client_country: None,
                client_user_agent: None,
                channel_selected_by: None,
                metadata: None,
            },
            None,
        )
        .await
        .unwrap();

        let found = crate::models::payment_order::find_by_idempotency_key(&pool, &idem_key, None)
            .await
            .unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn list_available_channels_returns_ranked() {
        let pool = setup_pool().await;
        let user_id = seed_user(&pool).await;

        let ch_cny = crate::models::payment_channel::insert(
            &pool,
            &CreatePaymentChannelCmd {
                provider: "alipay".into(),
                name: "Alipay CN".into(),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: Some(
                    r#"{"countries":["CN"],"currencies":["CNY"],"priority":100}"#.into(),
                ),
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap();
        let _ch_global = crate::models::payment_channel::insert(
            &pool,
            &CreatePaymentChannelCmd {
                provider: "stripe".into(),
                name: "Stripe Global".into(),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: Some(
                    r#"{"countries":["*"],"currencies":["USD","CNY"],"priority":10}"#.into(),
                ),
                is_active: true,
                sort_order: 1,
            },
            None,
        )
        .await
        .unwrap();

        let order = seed_order(&pool, user_id, 1000, "CNY").await;
        let auth = user_auth(user_id);

        let result = super::list_available_channels(
            &pool,
            &auth,
            &order.id.to_string(),
            Some("CN"),
            Some("zh"),
        )
        .await
        .unwrap();

        assert_eq!(result.channels.len(), 2);
        assert_eq!(result.recommended_channel_id, Some(ch_cny.id.to_string()));
        assert!(result.channels[0].is_recommended);
        assert_eq!(result.channels[0].channel_id, ch_cny.id.to_string());
        assert!(!result.channels[1].is_recommended);
    }

    #[tokio::test]
    async fn list_available_channels_currency_filter() {
        let pool = setup_pool().await;
        let user_id = seed_user(&pool).await;

        crate::models::payment_channel::insert(
            &pool,
            &CreatePaymentChannelCmd {
                provider: "stripe".into(),
                name: "Stripe".into(),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: Some(r#"{"currencies":["USD"]}"#.into()),
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap();

        let order = seed_order(&pool, user_id, 1000, "JPY").await;
        let auth = user_auth(user_id);

        let result =
            super::list_available_channels(&pool, &auth, &order.id.to_string(), None, None)
                .await
                .unwrap();

        assert!(result.channels.is_empty());
        assert!(result.recommended_channel_id.is_none());
    }

    #[tokio::test]
    async fn list_available_channels_order_not_found() {
        let pool = setup_pool().await;
        let user_id = seed_user(&pool).await;

        let auth = user_auth(user_id);

        let result =
            super::list_available_channels(&pool, &auth, "nonexistent_order", None, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_payment_order_auto_routes() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;

        let _ch = crate::models::payment_channel::insert(
            &pool,
            &CreatePaymentChannelCmd {
                provider: "stripe".into(),
                name: "Stripe".into(),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: Some(r#"{"currencies":["CNY"]}"#.into()),
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap();

        let order = seed_order(&pool, user_id, 1000, "CNY").await;

        let auth = user_auth(user_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: None,
            method: None,
            country: None,
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &auth,
            SnowflakeId(user_id),
            req,
            &config,
            Some("127.0.0.1"),
            Some("zh-CN"),
            Some("Mozilla/5.0"),
        )
        .await;

        #[cfg(feature = "payment-stripe")]
        {
            let (po, _) = result.unwrap();
            assert_eq!(po.channel_id, ch.id);
            assert_eq!(po.channel_selected_by.unwrap(), "auto");
            assert_eq!(po.client_country, None);
            assert_eq!(po.client_language.unwrap(), "zh-CN");
            assert_eq!(po.client_user_agent.unwrap(), "Mozilla/5.0");
            assert_eq!(po.client_ip.unwrap(), "127.0.0.1");
        }
        let _ = result;
    }

    #[tokio::test]
    async fn create_payment_order_auto_no_channel_error() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;

        let order = seed_order(&pool, user_id, 1000, "JPY").await;

        let auth = user_auth(user_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: None,
            method: None,
            country: None,
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &auth,
            SnowflakeId(user_id),
            req,
            &config,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("no payment channel")),
            e => panic!("expected BadRequest, got: {e:?}"),
        }
    }

    #[tokio::test]
    #[cfg(feature = "payment-stripe")]
    async fn create_payment_order_manual_selected_by() {
        let pool = setup_pool().await;
        let config = test_config();
        let user_id = seed_user(&pool).await;

        let ch = seed_channel(&pool, "stripe").await;
        let order = seed_order(&pool, user_id, 1000, "CNY").await;

        let auth = user_auth(user_id);
        let req = CreatePaymentOrderRequest {
            order_id: order.id.to_string(),
            channel_id: Some(ch.id.to_string()),
            method: None,
            country: Some("CN".into()),
            language: None,
            return_url: None,
            metadata: None,
        };

        let result = super::create_payment_order(
            &pool,
            &auth,
            SnowflakeId(user_id),
            req,
            &config,
            Some("10.0.0.1"),
            None,
            None,
        )
        .await;

        let (po, _) = result.unwrap();
        assert_eq!(po.channel_selected_by.unwrap(), "manual");
        assert_eq!(po.client_country.unwrap(), "CN");
        assert_eq!(po.client_ip.unwrap(), "10.0.0.1");
    }
}
