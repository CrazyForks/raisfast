//! 服务层（业务逻辑）。
//!
//! 本模块包含博客系统的核心业务逻辑。服务层位于处理器（handlers）与模型（models）之间：
//!
//! - 由 **handlers** 调用，接收已解析的请求参数。
//! - 调用 **models** 层执行数据库操作。
//! - 负责数据校验、权限检查、业务规则执行等职责。

pub mod auth;
pub mod comment;
pub mod media;
pub mod options;
pub mod post;
pub mod rbac;
pub mod stats;
pub mod tenant;
