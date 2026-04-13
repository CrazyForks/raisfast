//! 分类相关处理器
//!
//! 处理分类的列表、创建、更新和删除请求。

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthorUser;
use crate::models::category::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::services::post;

/// 获取所有分类列表
///
/// - **方法/路径：** `GET /api/categories`
/// - **认证：** 无需认证
/// - **说明：** 返回所有分类，按 `sort_order` 和 `name` 排序。
/// - **返回：** `ApiResponse<Vec<Category>>`
pub async fn list(
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<crate::models::category::Category>>> {
    let categories = post::list_categories(&state.pool).await?;
    Ok(ApiResponse::success(categories))
}

/// 创建新分类
///
/// - **方法/路径：** `POST /api/categories`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据请求体创建新分类，自动生成 slug。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<Category>`
pub async fn create(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    validation::validate(&req)?;
    let category = post::create_category(&state.pool, req).await?;
    Ok(ApiResponse::success(category))
}

/// 更新分类
///
/// - **方法/路径：** `PUT /api/categories/:id`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据分类 ID 更新分类信息，仅修改提供的字段。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<Category>`
pub async fn update(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    validation::validate(&req)?;
    let category = post::update_category(&state.pool, &id, req).await?;
    Ok(ApiResponse::success(category))
}

/// 删除分类
///
/// - **方法/路径：** `DELETE /api/categories/:id`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据分类 ID 删除分类。若分类不存在返回 404。
/// - **返回：** `ApiResponse<()>`
pub async fn delete(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post::delete_category(&state.pool, &id).await?;
    Ok(ApiResponse::success(()))
}
