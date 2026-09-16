//! Redemption code endpoints (wallet 域): admin 发码/列表/禁用 + 用户兑换。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use validator::Validate;

use crate::dto;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::redemption_code::{RedemptionCode, RedemptionCodeFilters, RedemptionCodeStatus};
use crate::types::price::Price;
use crate::types::snowflake_id::{SnowflakeId, parse_id};

fn routes_internal(
    r: axum::Router<crate::AppState>,
    registry: &mut crate::server::RouteRegistry,
    restful: bool,
) -> axum::Router<crate::AppState> {
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes",
        get,
        list_codes,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes",
        post,
        generate_codes,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes/{id}/disable",
        post,
        disable_code,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes/{id}/activate",
        post,
        activate_code,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes/{id}/enable",
        post,
        enable_code,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    let r = crate::reg_route!(
        r,
        registry,
        restful,
        "/admin/redemption-codes/{id}",
        delete,
        delete_code,
        "finance",
        "admin/redemption-codes",
        "admin"
    );
    crate::reg_route!(
        r,
        registry,
        restful,
        "/redemption-codes/redeem",
        post,
        redeem_code,
        "finance",
        "redemption-codes",
        "wallets:read"
    )
}

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    routes_internal(axum::Router::new(), registry, config.api_restful)
}

/// Generate-code payload. `user_id` 不填 = 公共码(谁兑谁得);填了 = 定向码。
#[derive(Debug, Deserialize, Validate, utoipa::ToSchema)]
pub struct GenerateCodesReq {
    #[validate(custom(function = "crate::dto::validate_currency_code"))]
    pub currency: String,
    /// 面额(主单位,元/美元——`Price` 线格式)。
    pub amount: Price,
    /// 定向属主(可选)。
    pub user_id: Option<SnowflakeId>,
    /// 批量张数,1-100,默认 1。
    #[serde(default = "default_count")]
    pub count: i64,
    /// 过期日 `YYYY-MM-DD`(UTC 当日 23:59:59 止),可选。
    pub expires_at: Option<String>,
}

fn default_count() -> i64 {
    1
}

fn params_expires(s: &Option<String>) -> AppResult<Option<crate::utils::tz::Timestamp>> {
    match s.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => crate::handlers::wallet::parse_utc_day(v, true).map(Some),
        None => Ok(None),
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RedeemCodeReq {
    pub code: String,
}

/// Admin: list codes.
#[utoipa::path(get, path = "/api/v1/admin/redemption-codes", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("page" = Option<i64>, Query), ("page_size" = Option<i64>, Query),
           ("user_id" = Option<String>, Query, description = "定向属主过滤"),
           ("status" = Option<String>, Query, description = "pending/redeemed/disabled")),
    responses((status = 200, description = "Redemption codes")))]
pub async fn list_codes(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(params): Query<ListCodesQuery>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<CodeResponse>>> {
    auth.ensure_admin()?;
    let user_id = match params
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(parse_id(s)?),
        None => None,
    };
    let filters = RedemptionCodeFilters {
        user_id,
        status: params.status.clone(),
    };
    let pagination =
        crate::utils::pagination::PaginationParams::from_options(params.page, params.page_size);
    let (rows, total) = crate::services::redemption::list(
        &state.pool,
        auth.tenant_id(),
        &filters,
        pagination.page,
        pagination.page_size,
    )
    .await?;
    // Owner usernames (定向码属主 + 激活人) — batch lookup.
    let mut ids: Vec<SnowflakeId> = rows.iter().filter_map(|r| r.user_id).collect();
    ids.extend(rows.iter().filter_map(|r| r.redeemed_by));
    let names = crate::models::user::find_usernames_by_ids(&state.pool, &ids).await?;
    let items: Vec<CodeResponse> = rows
        .into_iter()
        .map(|row| {
            let mut resp = CodeResponse::from_row(&row);
            resp.owner_username = row.user_id.and_then(|u| names.get(&u.0).cloned());
            resp.redeemed_by_username = row.redeemed_by.and_then(|u| names.get(&u.0).cloned());
            resp
        })
        .collect();
    Ok(pagination.paginate(items, total))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListCodesQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub user_id: Option<String>,
    pub status: Option<String>,
}

/// Admin-facing code row (hash 不出库)。
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct CodeResponse {
    pub id: SnowflakeId,
    /// 解密后的明文码(仅管理端;APP_KEY 加密落库)。
    pub code: Option<String>,
    pub currency: String,
    pub amount: Price,
    pub status: RedemptionCodeStatus,
    pub user_id: Option<SnowflakeId>,
    pub owner_username: Option<String>,
    pub redeemed_by: Option<SnowflakeId>,
    pub redeemed_by_username: Option<String>,
    #[schema(value_type = Option<String>)]
    pub redeemed_at: Option<crate::utils::tz::Timestamp>,
    /// 过期时间(UTC);过期后不可兑换。
    #[schema(value_type = Option<String>)]
    pub expires_at: Option<crate::utils::tz::Timestamp>,
    #[schema(value_type = String)]
    pub created_at: crate::utils::tz::Timestamp,
}

impl CodeResponse {
    fn from_row(row: &RedemptionCode) -> CodeResponse {
        CodeResponse {
            id: row.id,
            code: row
                .code_enc
                .as_deref()
                .and_then(crate::llm::crypto::decrypt),
            currency: row.currency.clone(),
            amount: row.amount,
            status: row.status,
            user_id: row.user_id,
            owner_username: None,
            redeemed_by: row.redeemed_by,
            redeemed_by_username: None,
            redeemed_at: row.redeemed_at,
            expires_at: row.expires_at,
            created_at: row.created_at,
        }
    }
}

/// Admin: generate codes — plaintext codes appear exactly once in the response.
#[utoipa::path(post, path = "/api/v1/admin/redemption-codes", tag = "wallets",
    security(("bearer_auth" = [])),
    request_body = GenerateCodesReq,
    responses((status = 200, description = "Generated codes (plaintext shown once)")))]
pub async fn generate_codes(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(body): Json<GenerateCodesReq>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    validation::validate(&body)?;
    auth.ensure_admin()?;
    let admin_id = auth.ensure_snowflake_user_id()?;
    let expires_at = params_expires(&body.expires_at)?;
    let codes = crate::services::redemption::generate(
        &state.pool,
        auth.tenant_id(),
        admin_id,
        body.user_id,
        &body.currency,
        body.amount,
        body.count,
        expires_at,
    )
    .await?;
    Ok(ApiResponse::success(serde_json::json!({
        "items": codes.iter().map(|c| serde_json::json!({
            "id": c.id,
            "code": c.code,
        })).collect::<Vec<_>>(),
    })))
}

/// Admin: disable a pending code.
#[utoipa::path(post, path = "/api/v1/admin/redemption-codes/{id}/disable", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Disabled (false = already redeemed)")))]
pub async fn disable_code(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<bool>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let ok = crate::services::redemption::disable(&state.pool, id).await?;
    if !ok {
        return Err(AppError::BadRequest(
            "redemption_code_not_redeemable".into(),
        ));
    }
    Ok(ApiResponse::success(true))
}

/// Activate payload — 公共码必填到账用户;定向码忽略。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ActivateCodeReq {
    pub user_id: Option<String>,
}

/// Admin: 直接激活——定向码到账属主;公共码到账指定用户并绑定。
#[utoipa::path(post, path = "/api/v1/admin/redemption-codes/{id}/activate", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    request_body = ActivateCodeReq,
    responses((status = 200, description = "Activated, wallet credited")))]
pub async fn activate_code(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(body): Json<ActivateCodeReq>,
) -> AppResult<ApiResponse<CodeResponse>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let bind_user = match body
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(parse_id(s)?),
        None => None,
    };
    let row = crate::services::redemption::activate_for_owner(
        &state.pool,
        auth.tenant_id(),
        id,
        bind_user,
    )
    .await?;
    let mut resp = CodeResponse::from_row(&row);
    if let Some(owner) = row.user_id {
        let names = crate::models::user::find_usernames_by_ids(&state.pool, &[owner]).await?;
        resp.owner_username = names.get(&owner.0).cloned();
    }
    Ok(ApiResponse::success(resp))
}

/// Admin: re-enable a disabled code.
#[utoipa::path(post, path = "/api/v1/admin/redemption-codes/{id}/enable", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Enabled (false = not disabled)")))]
pub async fn enable_code(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<bool>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let ok = crate::services::redemption::enable(&state.pool, id).await?;
    if !ok {
        return Err(AppError::BadRequest("redemption_code_not_disabled".into()));
    }
    Ok(ApiResponse::success(true))
}

/// Admin: delete a code (redeemed codes are kept for audit).
#[utoipa::path(delete, path = "/api/v1/admin/redemption-codes/{id}", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Deleted (false = redeemed, kept for audit)")))]
pub async fn delete_code(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<bool>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let ok = crate::services::redemption::delete(&state.pool, id).await?;
    if !ok {
        return Err(AppError::BadRequest("redemption_code_redeemed_keep".into()));
    }
    Ok(ApiResponse::success(true))
}

/// User: redeem a code — credits the bound wallet; idempotent by code state.
#[utoipa::path(post, path = "/api/v1/redemption-codes/redeem", tag = "wallets",
    security(("bearer_auth" = [])),
    request_body = RedeemCodeReq,
    responses((status = 200, description = "Redeemed (wallet credited)")))]
pub async fn redeem_code(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(body): Json<RedeemCodeReq>,
) -> AppResult<ApiResponse<dto::WalletTransactionResponse>> {
    if body.code.trim().is_empty() {
        return Err(AppError::BadRequest("code is required".into()));
    }
    let user_id = auth.ensure_snowflake_user_id()?;
    let code =
        crate::services::redemption::redeem(&state.pool, auth.tenant_id(), user_id, &body.code)
            .await?;
    let tx = crate::models::wallet_transaction::find_tx_by_transaction_no(
        &state.pool,
        &format!("redemption-{}", code.id.0),
    )
    .await?
    .ok_or_else(|| AppError::not_found("transaction"))?;
    Ok(ApiResponse::success(
        dto::WalletTransactionResponse::from_tx(tx)?,
    ))
}
