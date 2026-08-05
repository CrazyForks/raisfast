//! Webhook subscription API handlers

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::{AuthUser, TokenAction};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::pagination::PaginationParams;
use crate::webhook::model::{CreateWebhookRequest, UpdateWebhookRequest};

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks",
        get,
        list,
        "system",
        "admin/webhooks"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks",
        create,
        create,
        "system",
        "admin/webhooks"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks/{id}",
        get,
        self::get,
        "system",
        "admin/webhooks"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks/{id}",
        put,
        update,
        "system",
        "admin/webhooks"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks/{id}",
        delete,
        self::delete,
        "system",
        "admin/webhooks"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks/{id}/deliveries",
        get,
        deliveries,
        "system",
        "admin/webhooks"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/webhooks/batch",
        post,
        admin_batch,
        "system",
        "admin/webhooks"
    )
}

/// GET /admin/webhooks — paginated list of webhook subscriptions
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::webhook::model::WebhookSubscription>>,
> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Read)?;
    params.sanitize();
    let (items, total) = state
        .webhook
        .list(auth.tenant_id(), params.page, params.page_size)
        .await?;
    Ok(params.paginate(items, total))
}

/// GET /admin/webhooks/:id — get a single subscription
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Read)?;
    let sub = state
        .webhook
        .get(crate::types::snowflake_id::parse_id(&id)?)
        .await?;
    Ok(ApiResponse::success(sub))
}

/// POST /admin/webhooks — create a subscription
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<CreateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Create)?;
    if req.url.is_empty() {
        return Err(crate::errors::app_error::AppError::BadRequest(
            "url is required".into(),
        ));
    }
    if req.events.is_empty() {
        return Err(crate::errors::app_error::AppError::BadRequest(
            "events must not be empty".into(),
        ));
    }
    let tenant_id = auth.tenant_id();
    let sub = state
        .webhook
        .create(
            tenant_id,
            req.name.unwrap_or_default(),
            req.url,
            req.events,
            req.description,
            req.enabled.unwrap_or(true),
            req.secret,
        )
        .await?;
    Ok(ApiResponse::success(sub))
}

/// PUT /admin/webhooks/:id — update a subscription
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<UpdateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Update)?;
    tracing::debug!(
        id = %id,
        name = ?req.name,
        url = ?req.url,
        secret = ?req.secret,
        enabled = ?req.enabled,
        "webhook update request"
    );
    let sub = state
        .webhook
        .update(
            crate::types::snowflake_id::parse_id(&id)?,
            req.name,
            req.url,
            req.events,
            req.description,
            req.enabled,
            req.secret,
        )
        .await?;
    Ok(ApiResponse::success(sub))
}

/// DELETE /admin/webhooks/:id — delete a subscription
pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Delete)?;
    state
        .webhook
        .delete(crate::types::snowflake_id::parse_id(&id)?)
        .await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Delete)?;
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    for raw_id in &req.ids {
        let id = match raw_id.parse::<i64>() {
            Ok(v) => SnowflakeId(v),
            Err(_) => continue,
        };
        match req.action.as_str() {
            "delete" if state.webhook.delete(id).await.is_ok() => {
                affected += 1;
            }
            "enable" | "disable" => {
                let enabled = req.action == "enable";
                if state
                    .webhook
                    .update(id, None, None, None, None, Some(enabled), None)
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}

/// GET /admin/webhooks/:id/deliveries — delivery history for a webhook
pub async fn deliveries(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::webhook::model::WebhookDelivery>>,
> {
    auth.ensure_admin()?;
    auth.ensure_scope("webhooks", TokenAction::Read)?;
    params.sanitize();
    let wid = crate::types::snowflake_id::parse_id(&id)?;
    let (items, total) = crate::webhook::model::find_deliveries_by_webhook(
        &state.pool,
        wid,
        params.page,
        params.page_size,
    )
    .await?;
    Ok(ApiResponse::success(
        crate::errors::response::PaginatedData {
            items,
            total,
            page: params.page,
            page_size: params.page_size,
        },
    ))
}
