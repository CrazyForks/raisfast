//! Webhook subscription management module
//!
//! Supports CRUD subscriptions + HMAC-SHA256 signed delivery + event filtering.
//! The EventBus subscriber automatically POSTs matching events to subscriber URLs based on subscription rules.

pub mod handler;
pub mod model;
pub mod service;

pub use service::WebhookService;

#[cfg(feature = "export-types")]
export_types!(model::WebhookSubscription);
