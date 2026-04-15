//! 应用核心错误类型定义
//!
//! 本模块定义了 [`AppError`] 枚举，作为 handler 层与业务逻辑层的统一错误类型。
//! 每个变体都映射到对应的 HTTP 状态码，并通过 i18n（`rust_i18n`）实现错误消息的
//! 多语言翻译。实现了 [`IntoResponse`] trait，可直接作为 Axum handler 的返回类型。
//!
//! # 错误码约定
//!
//! 错误码遵循 `docs/guide.md` 中的规范，范围 40000–50000：
//!
//! | 变体           | HTTP 状态码               | 错误码  |
//! |---------------|--------------------------|---------|
//! | `BadRequest`  | 400 Bad Request          | 40000   |
//! | `Unauthorized`| 401 Unauthorized         | 40100   |
//! | `Forbidden`   | 403 Forbidden            | 40300   |
//! | `NotFound`    | 404 Not Found            | 40400   |
//! | `Conflict`    | 409 Conflict             | 40900   |
//! | `Internal`    | 500 Internal Server Error| 50000   |
//!
//! # i18n 消息格式
//!
//! 各变体通过 `rust_i18n::t!` 宏查找对应的翻译键，例如：
//! - `BadRequest` → `errors.bad_request`
//! - `Unauthorized` → `errors.unauthorized`
//! - `NotFound` → 先翻译资源名称 `resources.{key}`，再代入 `errors.not_found`

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::middleware::locale::current_locale;

/// 应用统一错误类型
///
/// 使用 `thiserror` 派生 `Error` trait，每个变体对应一类 HTTP 错误。
/// 在 handler 边界通过 [`IntoResponse`] 自动转换为 JSON 响应。
///
/// # 变体说明
///
/// - [`BadRequest`](AppError::BadRequest)：400 — 请求参数不合法，附带描述信息
/// - [`Unauthorized`](AppError::Unauthorized)：401 — 未提供有效的身份凭证
/// - [`Forbidden`](AppError::Forbidden)：403 — 已认证但无权访问该资源
/// - [`NotFound`](AppError::NotFound)：404 — 请求的资源不存在，附带资源标识
/// - [`Conflict`](AppError::Conflict)：409 — 资源冲突（如唯一约束违反），附带消息键
/// - [`Internal`](AppError::Internal)：500 — 服务器内部错误，包装 `anyhow::Error`
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// 400 Bad Request — 请求参数校验失败或业务规则不满足
    ///
    /// 附带的 `String` 为错误描述，会通过 i18n 键 `errors.bad_request` 翻译。
    #[error("bad request: {0}")]
    BadRequest(String),
    /// 401 Unauthorized — 未提供或提供了无效的身份认证凭证
    #[error("unauthorized")]
    Unauthorized,
    /// 403 Forbidden — 已认证用户无权执行此操作
    #[error("forbidden")]
    Forbidden,
    /// 404 Not Found — 请求的资源不存在
    ///
    /// 附带的 `String` 为资源标识键（如 `"user"`），会先通过
    /// `resources.{key}` 翻译为本地化的资源名称，再代入 `errors.not_found` 模板。
    #[error("not found: {0}")]
    NotFound(String),
    /// 409 Conflict — 资源冲突（如唯一约束违反、重复操作）
    ///
    /// 附带的 `String` 为消息键（如 `"duplicate_entry"`），会通过
    /// `messages.{key}` 翻译为本地化消息，再代入 `errors.conflict` 模板。
    #[error("conflict: {0}")]
    Conflict(String),
    /// 500 Internal Server Error — 服务器内部未预期的错误
    ///
    /// 通过 `#[from]` 自动从 `anyhow::Error` 转换，避免手动映射。
    /// 向客户端隐藏内部细节，仅返回通用错误消息。
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    /// 快捷构造 `NotFound` 错误。
    ///
    /// # 参数
    ///
    /// - `resource` — 资源名称键（如 `"post"`），用于 i18n 翻译
    #[must_use]
    pub fn not_found(resource: &str) -> Self {
        AppError::NotFound(resource.into())
    }

    /// 检查 DELETE/UPDATE 影响行数，为 0 时返回 `NotFound`。
    ///
    /// 用于 model 层的 `delete()` 和 `update_status()` 函数，
    /// 避免每次手动检查 `rows_affected() == 0`。
    pub fn expect_affected(
        result: &sqlx::sqlite::SqliteQueryResult,
        resource: &str,
    ) -> AppResult<()> {
        if result.rows_affected() == 0 {
            Err(AppError::NotFound(resource.into()))
        } else {
            Ok(())
        }
    }

    /// 根据当前 locale 翻译错误消息
    ///
    /// 每个 `AppError` 变体都有对应的 i18n 翻译键。此方法根据传入的 `locale`
    /// 参数查找翻译，并将动态参数（如资源名称、错误描述）代入翻译模板。
    ///
    /// # 参数
    ///
    /// - `locale` — 目标语言标识（如 `"en"`、`"zh-CN"`），通常从请求中间件获取
    ///
    /// # 返回值
    ///
    /// 返回翻译后的错误消息字符串。若翻译键不存在，`rust_i18n::t!` 会回退到键名本身。
    fn i18n_message(&self, locale: &str) -> String {
        rust_i18n::set_locale(locale);
        match self {
            AppError::BadRequest(msg) => {
                rust_i18n::t!("errors.bad_request", message = msg).to_string()
            }
            AppError::Unauthorized => rust_i18n::t!("errors.unauthorized").to_string(),
            AppError::Forbidden => rust_i18n::t!("errors.forbidden").to_string(),
            AppError::NotFound(resource_key) => {
                let res_key = format!("resources.{resource_key}");
                let resource = rust_i18n::t!(&res_key);
                rust_i18n::t!("errors.not_found", resource = resource).to_string()
            }
            AppError::Conflict(msg_key) => {
                let msg_key_full = format!("messages.{msg_key}");
                let message = rust_i18n::t!(&msg_key_full);
                rust_i18n::t!("errors.conflict", message = message).to_string()
            }
            AppError::Internal(_) => rust_i18n::t!("errors.internal").to_string(),
        }
    }
}

/// 将 `AppError` 转换为 Axum HTTP 响应
///
/// 实现了 [`IntoResponse`] trait，使 `AppError` 可以直接作为 handler 返回类型。
/// 转换流程：
///
/// 1. 根据变体确定 HTTP 状态码和业务错误码（40000–50000 范围）
/// 2. 通过 [`i18n_message`](AppError::i18n_message) 翻译错误消息
/// 3. 记录错误日志（`tracing::error!`）
/// 4. 构造 [`ErrorBody`] JSON 响应体
///
/// # 响应格式
///
/// ```json
/// { "code": 40000, "message": "错误描述", "data": null }
/// ```
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, 40000),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, 40100),
            AppError::Forbidden => (StatusCode::FORBIDDEN, 40300),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, 40400),
            AppError::Conflict(_) => (StatusCode::CONFLICT, 40900),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, 50000),
        };

        let locale = current_locale();
        let message = self.i18n_message(&locale);

        match status {
            StatusCode::BAD_REQUEST
            | StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::CONFLICT => {
                tracing::warn!(%code, %message, "client error");
            }
            _ => {
                tracing::error!(%code, %message, "server error");
            }
        }

        let body = ErrorBody {
            code,
            message,
            data: (),
        };

        (status, Json(body)).into_response()
    }
}

/// 错误响应体结构
///
/// 与成功响应保持一致的 JSON 结构，`data` 字段始终为 `()`（空），
/// 便于客户端使用统一的解析逻辑。
///
/// # 序列化示例
///
/// ```json
/// { "code": 40400, "message": "资源未找到", "data": null }
/// ```
#[derive(Serialize)]
struct ErrorBody {
    code: i32,
    message: String,
    data: (),
}

/// 数据库错误到 `AppError` 的自动映射
///
/// 将 `sqlx::Error` 转换为语义化的 `AppError` 变体，避免在业务代码中
/// 手动处理数据库层错误：
///
/// - `RowNotFound` → [`AppError::NotFound`]（资源标识默认为 `"resource"`）
/// - `UNIQUE constraint failed` → [`AppError::Conflict`]（消息键为 `"duplicate_entry"`）
/// - 其他数据库错误 → [`AppError::Internal`]（包装为 `anyhow::Error`）
///
/// # 设计决策
///
/// 将数据库层细节屏蔽在 `AppError` 内部，handler 和 service 层只需
/// 使用 `?` 操作符即可将 SQL 错误传播为合适的 HTTP 响应。
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("resource".into()),
            sqlx::Error::Database(ref e) => {
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint failed") {
                    AppError::Conflict("duplicate_entry".into())
                } else {
                    AppError::Internal(err.into())
                }
            }
            other => AppError::Internal(other.into()),
        }
    }
}

/// 应用层 Result 类型别名
///
/// 所有 handler 和 service 函数的返回类型统一使用 `AppResult<T>`，
/// 等价于 `Result<T, AppError>`，简化函数签名并保证错误处理的一致性。
pub type AppResult<T> = Result<T, AppError>;
