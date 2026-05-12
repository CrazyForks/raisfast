use axum::Json;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post as http_post};

use crate::dto;
use crate::errors::app_error::AppError;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        "/wallets",
        get(list_wallets),
        "system public",
        "wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/wallets/{currency}",
        get(get_wallet),
        "system public",
        "wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/wallets/transactions",
        get(list_all_transactions),
        "system public",
        "wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/wallets/{currency}/transactions",
        get(list_transactions),
        "system public",
        "wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets",
        get(list_all_wallets),
        "admin wallet",
        "admin/wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets/transactions",
        get(list_all_transactions_admin),
        "admin wallet",
        "admin/wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets/credit",
        http_post(admin_credit),
        "admin wallet",
        "admin/wallet",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets/debit",
        http_post(admin_debit),
        "admin wallet",
        "admin/wallet",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets/{user_id}/transactions",
        get(list_user_all_transactions),
        "admin wallet",
        "admin/wallet",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/wallets/{user_id}/{currency}/transactions",
        get(list_user_transactions),
        "admin wallet",
        "admin/wallet",
        ["GET"]
    );
    crate::reg_route!(
        r,
        registry,
        "/admin/wallets/{tx_doc_id}/reversal",
        http_post(admin_reversal),
        "admin wallet",
        "admin/wallet",
        ["POST"]
    )
}

// ── User-facing ──

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

pub async fn list_all_wallets(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<ApiResponse<crate::errors::response::PaginatedData<dto::WalletResponse>>, AppError> {
    auth.ensure_admin()?;
    let (rows, total) = state
        .wallet_repo
        .find_all_wallets(params.page, params.page_size)
        .await?;
    let items: Vec<dto::WalletResponse> = rows
        .into_iter()
        .map(dto::WalletResponse::from_wallet)
        .collect::<Result<_, _>>()?;
    Ok(params.paginate(items, total))
}

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
        .find_all_transactions(params.page, params.page_size)
        .await?;
    let items =
        crate::services::wallet::tx_list_to_response(state.wallet_repo.as_ref(), rows).await?;
    Ok(params.paginate(items, total))
}

pub async fn admin_credit(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<dto::AdminWalletOperationRequest>,
) -> Result<ApiResponse<dto::WalletTransactionResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let user = crate::models::user::find_by_id(&state.pool, &req.user_id, None)
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

pub async fn admin_debit(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<dto::AdminWalletOperationRequest>,
) -> Result<ApiResponse<dto::WalletTransactionResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let user = crate::models::user::find_by_id(&state.pool, &req.user_id, None)
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
    let user = crate::models::user::find_by_id(&state.pool, &user_doc_id, None)
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
    let user = crate::models::user::find_by_id(&state.pool, &user_doc_id, None)
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
