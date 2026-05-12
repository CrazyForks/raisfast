//! Webhook 订阅 API Handler

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::utils::pagination::PaginationParams;
use crate::webhook::model::{CreateWebhookRequest, UpdateWebhookRequest};

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/admin/webhooks",
        get(list).post(create),
        "system admin",
        "admin/webhooks",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/webhooks/{id}",
        get(self::get).put(update).delete(self::delete),
        "system admin",
        "admin/webhooks",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/webhooks/batch",
        http_post(admin_batch),
        "system admin",
        "admin/webhooks",
        ["POST"]
    )
}

/// GET /admin/webhooks — 分页查询 webhook 订阅
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::webhook::model::WebhookSubscription>>,
> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = state
        .webhook
        .list(auth.tenant_id(), params.page, params.page_size)
        .await?;
    Ok(params.paginate(items, total))
}

/// GET /admin/webhooks/:id — 获取单个订阅
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
    let sub = state.webhook.get(&id).await?;
    Ok(ApiResponse::success(sub))
}

/// POST /admin/webhooks — 创建订阅
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<CreateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
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
            req.url,
            req.events,
            req.description,
            req.enabled.unwrap_or(true),
            req.secret,
        )
        .await?;
    Ok(ApiResponse::success(sub))
}

/// PUT /admin/webhooks/:id — 更新订阅
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<UpdateWebhookRequest>,
) -> AppResult<ApiResponse<crate::webhook::model::WebhookSubscription>> {
    auth.ensure_admin()?;
    let sub = state
        .webhook
        .update(&id, req.url, req.events, req.description, req.enabled)
        .await?;
    Ok(ApiResponse::success(sub))
}

/// DELETE /admin/webhooks/:id — 删除订阅
pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.webhook.delete(&id).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if state.webhook.delete(id).await.is_ok() {
                    affected += 1;
                }
            }
            "enable" | "disable" => {
                let enabled = req.action == "enable";
                if state
                    .webhook
                    .update(id, None, None, None, Some(enabled))
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
