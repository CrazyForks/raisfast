//! Webhook 订阅管理模块
//!
//! 支持 CRUD 订阅 + HMAC-SHA256 签名投递 + 事件过滤。
//! EventBus 订阅者根据订阅规则自动将匹配的事件 POST 到订阅者 URL。

pub mod handler;
pub mod model;
pub mod service;

pub use service::WebhookService;
