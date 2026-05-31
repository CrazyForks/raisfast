use crate::dto::user_address::{
    CreateUserAddressRequest, UpdateUserAddressRequest, UserAddressResponse,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;

use axum::Json;
use axum::extract::{Path, State};

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
        "/user/addresses",
        get,
        list_addresses,
        "system authed",
        "user_addresses"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/user/addresses",
        create,
        create_address,
        "system authed",
        "user_addresses"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/user/addresses/{id}",
        put,
        update_address,
        "system authed",
        "user_addresses"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/user/addresses/{id}",
        delete,
        delete_address,
        "system authed",
        "user_addresses"
    )
}

pub async fn list_addresses(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<UserAddressResponse>>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let addrs = state.user_address_service.list(&auth, user_id).await?;
    let resp: Vec<UserAddressResponse> = addrs.into_iter().map(Into::into).collect();
    Ok(ApiResponse::success(resp))
}

pub async fn create_address(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateUserAddressRequest>,
) -> AppResult<ApiResponse<UserAddressResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    validation::validate(&req)?;
    let addr = state
        .user_address_service
        .create(&auth, user_id, req)
        .await?;
    Ok(ApiResponse::success(UserAddressResponse::from(addr)))
}

pub async fn update_address(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserAddressRequest>,
) -> AppResult<ApiResponse<UserAddressResponse>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let addr = state
        .user_address_service
        .update(&auth, user_id, id, req)
        .await?;
    Ok(ApiResponse::success(UserAddressResponse::from(addr)))
}

pub async fn delete_address(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .user_address_service
        .delete(&auth, user_id, id)
        .await?;
    Ok(ApiResponse::success(()))
}
