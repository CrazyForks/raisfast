//! 服务层（业务逻辑）。
//!
//! 本模块包含raisfast 的核心业务逻辑。服务层位于处理器（handlers）与模型（models）之间：
//!
//! - 由 **handlers** 调用，接收已解析的请求参数。
//! - 调用 **models** 层执行数据库操作。
//! - 负责数据校验、权限检查、业务规则执行等职责。

pub mod api_token;
pub mod aspect_dispatch;
pub mod auth;
pub mod category;
pub mod comment;
pub mod email_verification;
pub mod media;
pub mod oauth;
pub mod options;
pub mod page;
pub mod password_reset;
pub mod post;
pub mod rbac;
pub mod reusable_block;
pub mod sms;
pub mod stats;
pub mod tag;
pub mod tenant;
pub mod user;
