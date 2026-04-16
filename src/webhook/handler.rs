//! Webhook 订阅 API Handler

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AdminUser;
use crate::middleware::tenant::ResolvedTenant;
use crate::utils::pagination::PaginationParams;
use crate::webhook::model::{CreateWebhookRequest, UpdateWebhookRequest};

/// GET /admin/webhooks — 分页查询 webhook 订阅
pub async fn list(
    _admin: AdminUser,
    tenant: ResolvedTenant,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::webhook::model::WebhookSubscription>>,
> {
    params.sanitize();
    let (items, total) = state
        .webhook
        .list(tenant.as_str(), params.page, params.page_size)
        .await?;
    Ok(params.paginate(items, total))
}

/// GET /admin/webhooks/:id — 获取单个订阅
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    let sub = state.webhook.get(&id).await?;
    Ok(ApiResponse::success(sub))
}

/// POST /admin/webhooks — 创建订阅
pub async fn create(
    _admin: AdminUser,
    tenant: ResolvedTenant,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<CreateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
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
    let tenant_id = tenant
        .tenant_id
        .unwrap_or_else(|| crate::db::tenant::DEFAULT_TENANT.to_string());
    let sub = state
        .webhook
        .create(
            &tenant_id,
            req.url,
            req.events,
            req.description,
            req.enabled.unwrap_or(true),
        )
        .await?;
    Ok(ApiResponse::success(sub))
}

/// PUT /admin/webhooks/:id — 更新订阅
pub async fn update(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<UpdateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    let sub = state
        .webhook
        .update(&id, req.url, req.events, req.description, req.enabled)
        .await?;
    Ok(ApiResponse::success(sub))
}

/// DELETE /admin/webhooks/:id — 删除订阅
pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.webhook.delete(&id).await?;
    Ok(ApiResponse::success(()))
}
