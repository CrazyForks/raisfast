//! 统一 JSON 响应格式定义
//!
//! 本模块定义了应用所有 API 接口的统一响应结构，确保客户端收到的
//! JSON 格式始终一致：
//!
//! ```json
//! { "code": 0, "message": "操作成功", "data": { ... } }
//! ```
//!
//! 包含以下核心类型：
//!
//! - [`ApiResponse`]：通用响应包装器，支持成功和错误响应
//! - [`PaginatedData`]：分页数据信封，用于列表接口

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// 统一 API 响应结构
///
/// 所有 API 接口均使用此结构包装返回数据，保证响应格式一致。
///
/// # 字段说明
///
/// - `code` — 业务状态码（`0` 表示成功，`40000`–`50000` 表示各类错误）
/// - `message` — 可读的状态描述（支持 i18n 多语言翻译）
/// - `data` — 响应负载数据，错误响应时为 `None`
///
/// # 泛型参数
///
/// - `T` — 响应数据的序列化类型，必须实现 [`Serialize`]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    /// 构造成功响应
    ///
    /// 创建一个 `code` 为 `0` 的成功响应，`data` 字段包含传入的数据，
    /// `message` 通过 i18n 键 `messages.success` 翻译为当前 locale 的"成功"消息。
    ///
    /// # 参数
    ///
    /// - `data` — 要返回给客户端的业务数据
    ///
    /// # 返回值
    ///
    /// 返回完整的 [`ApiResponse`] 实例，可直接作为 Axum handler 的返回值
    /// （通过 [`IntoResponse`] 实现）。
    pub fn success(data: T) -> Self {
        let locale = crate::middleware::locale::current_locale();
        rust_i18n::set_locale(&locale);
        let message = rust_i18n::t!("messages.success").to_string();
        Self {
            code: 0,
            message,
            data: Some(data),
        }
    }
}

impl ApiResponse<()> {
    /// 构造错误响应
    ///
    /// 创建一个业务错误响应。HTTP 状态码固定为 `200 OK`，实际错误通过
    /// JSON body 中的 `code` 字段区分（遵循 40000–50000 范围约定）。
    ///
    /// # 参数
    ///
    /// - `code` — 业务错误码（如 `40000`、`40400`）
    /// - `message` — 错误描述消息（通常已通过 i18n 翻译）
    ///
    /// # 返回值
    ///
    /// 返回 `axum::response::Response`，可直接从 handler 返回。
    ///
    /// # 注意
    ///
    /// 此方法定义在 `ApiResponse<()>` 上，仅用于构造无数据的错误响应。
    /// 正常的错误处理流程应优先使用 [`AppError`](crate::errors::AppError)
    /// 及其 [`IntoResponse`] 实现，它会自动设置正确的 HTTP 状态码。
    pub fn error(code: i32, message: String) -> Response {
        let body = Self {
            code,
            message,
            data: None,
        };
        (StatusCode::OK, Json(body)).into_response()
    }
}

/// 分页数据信封
///
/// 用于列表接口的响应数据包装，包含分页元信息和当前页数据列表。
/// 客户端可据此实现翻页、计算总页数等功能。
///
/// # 字段说明
///
/// - `items` — 当前页的数据列表
/// - `total` — 符合查询条件的总记录数
/// - `page` — 当前页码（从 1 开始）
/// - `page_size` — 每页记录数
///
/// # 使用示例
///
/// ```ignore
/// let paginated = PaginatedData {
///     items: posts,
///     total: 100,
///     page: 1,
///     page_size: 20,
/// };
/// Ok(ApiResponse::success(paginated))
/// ```
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PaginatedData<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// 将 `ApiResponse` 转换为 Axum HTTP 响应
///
/// 序列化为 JSON 并设置 `Content-Type: application/json`。
/// 成功响应的 HTTP 状态码默认为 `200 OK`。
impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}
