//! 中间件模块
//!
//! 本模块包含跨切面（cross-cutting）中间件，为所有 HTTP 请求提供统一的基础设施支持：
//!
//! - **认证（auth）**：基于 JWT 的用户身份验证与角色鉴权
//! - **国际化（locale）**：请求级别的语言区域检测，支持 i18n 错误消息
//! - **`限流（rate_limit`）**：基于 IP 的滑动窗口速率限制，防止接口滥用

pub mod auth;
pub mod locale;
pub mod permission;
pub mod rate_limit;
pub mod tenant;
