use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto;
use crate::errors::app_error::AppError;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry, config: &crate::config::app::AppConfig) -> axum::Router<crate::AppState> {
    let _restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(r, registry, restful, "/wallets", get, list_wallets, "system public", "wallet");
    let r = reg_route!(r, registry, restful, "/wallets/{currency}", get, get_wallet, "system public", "wallet");
    let r = reg_route!(r, registry, restful, "/wallets/transactions", get, list_all_transactions, "system public", "wallet");
    let r = reg_route!(r, registry, restful, "/wallets/{currency}/transactions", get, list_transactions, "system public", "wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets", get, list_all_wallets, "admin wallet", "admin/wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets/transactions", get, list_all_transactions_admin, "admin wallet", "admin/wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets/credit", post, admin_credit, "admin wallet", "admin/wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets/debit", post, admin_debit, "admin wallet", "admin/wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets/{user_id}/transactions", get, list_user_all_transactions, "admin wallet", "admin/wallet");
    let r = reg_route!(r, registry, restful, "/admin/wallets/{user_id}/{currency}/transactions", get, list_user_transactions, "admin wallet", "admin/wallet");
    reg_route!(r, registry, restful, "/admin/wallets/{tx_doc_id}/reversal", post, admin_reversal, "admin wallet", "admin/wallet")
}

// ── User-facing ──

#[utoipa::path(get, path = "/wallets", tag = "wallets",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "User wallets list"))
)]
pub async fn list_wallets(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> Result<ApiResponse<Vec<dto::WalletResponse>>, AppError> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(&state.pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let wallets = state.wallet_repo.find_wallets_by_user(user.id).await?;
    let items: Vec<dto::WalletResponse> = wallets
        .into_iter()
        .map(dto::WalletResponse::from_wallet)
        .collect::<Result<_, _>>()?;
    Ok(ApiResponse::success(items))
}

#[utoipa::path(get, path = "/wallets/{currency}", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("currency" = String, Path, description = "Currency code")),
    responses((status = 200, description = "Wallet detail"))
)]
pub async fn get_wallet(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(currency): Path<String>,
) -> Result<ApiResponse<dto::WalletResponse>, AppError> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(&state.pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let w = state
        .wallet_repo
        .find_wallet_by_user_and_currency(user.id, &currency)
        .await?
        .ok_or_else(|| AppError::not_found("wallet"))?;
    Ok(ApiResponse::success(dto::WalletResponse::from_wallet(w)?))
}

#[utoipa::path(get, path = "/wallets/{currency}/transactions", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("currency" = String, Path, description = "Currency code")),
    responses((status = 200, description = "Wallet transactions"))
)]
pub async fn list_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(currency): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<
    ApiResponse<crate::errors::response::PaginatedData<dto::WalletTransactionResponse>>,
    AppError,
> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(&state.pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let w = state
        .wallet_repo
        .find_wallet_by_user_and_currency(user.id, &currency)
        .await?
        .ok_or_else(|| AppError::not_found("wallet"))?;

    let (rows, total) = state
        .wallet_repo
        .find_transactions_by_wallet(w.id, params.page, params.page_size)
        .await?;

    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

#[utoipa::path(get, path = "/wallets/transactions", tag = "wallets",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "All wallet transactions"))
)]
pub async fn list_all_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<
    ApiResponse<crate::errors::response::PaginatedData<dto::WalletTransactionResponse>>,
    AppError,
> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(&state.pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let (rows, total) = state
        .wallet_repo
        .find_transactions_by_user(user.id, params.page, params.page_size)
        .await?;

    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

// ── Admin ──

#[utoipa::path(get, path = "/admin/wallets", tag = "wallets",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin wallets list"))
)]
pub async fn list_all_wallets(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<ApiResponse<crate::errors::response::PaginatedData<dto::WalletResponse>>, AppError> {
    auth.ensure_admin()?;
    let (rows, total) = state
        .wallet_repo
        .find_all_wallets(params.page, params.page_size, auth.tenant_id())
        .await?;
    let items: Vec<dto::WalletResponse> = rows
        .into_iter()
        .map(dto::WalletResponse::from_wallet)
        .collect::<Result<_, _>>()?;
    Ok(params.paginate(items, total))
}

#[utoipa::path(get, path = "/admin/wallets/transactions", tag = "wallets",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin all transactions"))
)]
pub async fn list_all_transactions_admin(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<
    ApiResponse<crate::errors::response::PaginatedData<dto::WalletTransactionResponse>>,
    AppError,
> {
    auth.ensure_admin()?;
    let (rows, total) = state
        .wallet_repo
        .find_all_transactions(params.page, params.page_size, auth.tenant_id())
        .await?;
    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

#[utoipa::path(post, path = "/admin/wallets/credit", tag = "wallets",
    security(("bearer_auth" = [])),
    request_body = dto::AdminWalletOperationRequest,
    responses((status = 200, description = "Wallet credited"))
)]
pub async fn admin_credit(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<dto::AdminWalletOperationRequest>,
) -> Result<ApiResponse<dto::WalletTransactionResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let user = crate::models::user::find_by_id(&state.pool, &req.user_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let tx = crate::services::wallet::credit_wallet(
        state.wallet_repo.as_ref(),
        &state.pool,
        user.id,
        &req.currency,
        req.amount,
        WalletTxType::Recharge,
        &req.transaction_no,
        req.reference_type.or(Some(WalletReferenceType::Admin)),
        req.reference_id.as_deref(),
        req.metadata.as_deref(),
    )
    .await?;

    let resp = crate::services::wallet::tx_to_response(state.wallet_repo.as_ref(), tx).await?;
    Ok(ApiResponse::success(resp))
}

#[utoipa::path(post, path = "/admin/wallets/debit", tag = "wallets",
    security(("bearer_auth" = [])),
    request_body = dto::AdminWalletOperationRequest,
    responses((status = 200, description = "Wallet debited"))
)]
pub async fn admin_debit(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<dto::AdminWalletOperationRequest>,
) -> Result<ApiResponse<dto::WalletTransactionResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let user = crate::models::user::find_by_id(&state.pool, &req.user_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let tx = crate::services::wallet::debit_wallet(
        state.wallet_repo.as_ref(),
        &state.pool,
        user.id,
        &req.currency,
        req.amount,
        WalletTxType::Payment,
        &req.transaction_no,
        req.reference_type.or(Some(WalletReferenceType::Admin)),
        req.reference_id.as_deref(),
        req.metadata.as_deref(),
    )
    .await?;

    let resp = crate::services::wallet::tx_to_response(state.wallet_repo.as_ref(), tx).await?;
    Ok(ApiResponse::success(resp))
}

#[utoipa::path(get, path = "/admin/wallets/{user_id}/{currency}/transactions", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("user_id" = String, Path, description = "User ID"), ("currency" = String, Path, description = "Currency code")),
    responses((status = 200, description = "User wallet transactions"))
)]
pub async fn list_user_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path((user_doc_id, currency)): Path<(String, String)>,
    Query(params): Query<PaginationParams>,
) -> Result<
    ApiResponse<crate::errors::response::PaginatedData<dto::WalletTransactionResponse>>,
    AppError,
> {
    auth.ensure_admin()?;
    let user = crate::models::user::find_by_id(&state.pool, &user_doc_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let w = state
        .wallet_repo
        .find_wallet_by_user_and_currency(user.id, &currency)
        .await?
        .ok_or_else(|| AppError::not_found("wallet"))?;

    let (rows, total) = state
        .wallet_repo
        .find_transactions_by_wallet(w.id, params.page, params.page_size)
        .await?;

    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

#[utoipa::path(get, path = "/admin/wallets/{user_id}/transactions", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("user_id" = String, Path, description = "User ID")),
    responses((status = 200, description = "User all transactions"))
)]
pub async fn list_user_all_transactions(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(user_doc_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<
    ApiResponse<crate::errors::response::PaginatedData<dto::WalletTransactionResponse>>,
    AppError,
> {
    auth.ensure_admin()?;
    let user = crate::models::user::find_by_id(&state.pool, &user_doc_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let (rows, total) = state
        .wallet_repo
        .find_transactions_by_user(user.id, params.page, params.page_size)
        .await?;

    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

#[utoipa::path(post, path = "/admin/wallets/{tx_doc_id}/reversal", tag = "wallets",
    security(("bearer_auth" = [])),
    params(("tx_doc_id" = String, Path, description = "Transaction document ID")),
    request_body = dto::ReversalRequest,
    responses((status = 200, description = "Transaction reversed"))
)]
pub async fn admin_reversal(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(tx_doc_id): Path<String>,
    Json(req): Json<dto::ReversalRequest>,
) -> Result<ApiResponse<dto::WalletTransactionResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;

    let original = state
        .wallet_repo
        .find_tx_by_document_id(&tx_doc_id)
        .await?
        .ok_or_else(|| AppError::not_found("transaction"))?;

    if let Some(tid) = auth.tenant_id() {
        let wallet = state
            .wallet_repo
            .find_wallet_by_id(original.wallet_id)
            .await?
            .ok_or_else(|| AppError::not_found("wallet"))?;
        let user =
            crate::models::user::find_by_id(&state.pool, &wallet.user_id.to_string(), Some(tid))
                .await?;
        if user.is_none() {
            return Err(AppError::not_found("transaction"));
        }
    }

    let tx = crate::services::wallet::reverse_transaction(
        state.wallet_repo.as_ref(),
        &state.pool,
        original.id,
        &req.transaction_no,
    )
    .await?;

    let resp = crate::services::wallet::tx_to_response(state.wallet_repo.as_ref(), tx).await?;
    Ok(ApiResponse::success(resp))
}
