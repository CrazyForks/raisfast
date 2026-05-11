//! HTTP 请求处理器（handlers）
//!
//! 本模块包含所有 Axum 路由处理函数。每个处理器遵循 **薄层** 原则：
//! 仅负责提取请求参数、调用业务逻辑层（services）、返回 HTTP 响应。
//!
//! 处理器中 **不包含** 任何业务逻辑，所有业务逻辑均在 `services` 层实现。
//!
//! # 子模块
//! - [`auth`] — 注册、登录、登出、刷新令牌
//! - [`user`] — 用户资料查看与修改
//! - [`category`] — 分类增删改查
//! - [`tag`] — 标签增删改查
//! - [`post`] — 文章增删改查与列表
//! - [`comment`] — 评论创建、审核、删除
//! - [`media`] — 媒体文件上传、列表、删除
//! - [`rss`] — RSS 订阅源
//! - [`health`] — 健康检查

pub mod api_token;
pub mod auth;
pub mod category;
pub mod comment;
pub mod content_revision;
pub mod cron;
pub mod health;
pub mod media;
pub mod oauth;
pub mod options;
pub mod page;
pub mod plugin;
pub mod post;
pub mod rbac;
pub mod reusable_block;
pub mod rss;
pub mod sse;
pub mod stats;
pub mod tag;
pub mod tenant;
pub mod user;
pub mod wallet;
pub mod ws;
