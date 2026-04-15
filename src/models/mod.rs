//! 数据模型层（models）
//!
//! 本模块定义了博客系统的所有数据结构以及通过 sqlx 执行的原始 SQL 查询。
//!
//! 每个子模块对应一个领域实体，包含：
//! - 数据库行模型（完整字段，直接映射数据库表）
//! - API 响应模型（面向外部的安全视图，如过滤掉密码哈希）
//! - 请求验证结构体（附带 `validator` 约束）
//! - 增删改查等数据库操作函数
//!
//! # 子模块
//! - [`user`] — 用户模型与认证相关查询
//! - [`post`] — 文章模型与查询
//! - [`category`] — 分类模型与查询
//! - [`tag`] — 标签模型与查询
//! - [`comment`] — 评论模型与查询
//! - [`media`] — 媒体文件模型与查询
//! - [`refresh_token`] — 刷新令牌模型与查询

pub mod category;
pub mod comment;
pub mod media;
pub mod options;
pub mod plugin_storage;
pub mod post;
pub mod rbac;
pub mod refresh_token;
pub mod tag;
pub mod tenant;
pub mod user;
