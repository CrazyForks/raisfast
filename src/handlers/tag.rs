//! 标签相关处理器
//!
//! 处理标签的列表、创建和删除请求。

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthorUser;
use crate::models::tag::CreateTagRequest;
use crate::services::post;

/// 获取所有标签列表
///
/// - **方法/路径：** `GET /api/tags`
/// - **认证：** 无需认证
/// - **说明：** 返回所有标签，按 `name` 字母顺序排列。
/// - **返回：** `ApiResponse<Vec<Tag>>`
pub async fn list(
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<crate::models::tag::Tag>>> {
    let tags = post::list_tags(&state.pool).await?;
    Ok(ApiResponse::success(tags))
}

/// 创建新标签
///
/// - **方法/路径：** `POST /api/tags`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据请求体创建新标签，自动生成 slug。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<Tag>`
pub async fn create(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    validation::validate(&req)?;
    let tag = post::create_tag(&state.pool, req).await?;
    Ok(ApiResponse::success(tag))
}

/// 删除标签
///
/// - **方法/路径：** `DELETE /api/tags/:id`
/// - **认证：** 需要作者或以上权限（`AuthorUser`）
/// - **说明：** 根据标签 ID 删除标签。若标签不存在返回 404。
/// - **返回：** `ApiResponse<()>`
pub async fn delete(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post::delete_tag(&state.pool, &id).await?;
    Ok(ApiResponse::success(()))
}
