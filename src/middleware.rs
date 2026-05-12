//! Middleware module
//!
//! This module contains cross-cutting middlewares that provide unified infrastructure
//! support for all HTTP requests:
//!
//! - **Auth**: JWT-based user authentication and role authorization
//! - **Locale**: Request-level language region detection, supporting i18n error messages
//! - **Rate limit**: IP-based sliding window rate limiting to prevent API abuse
//! - **Request ID**: Generates a unique ID for each request, linking logs together
//! - **AOP HTTP**: Request/response interception, connecting AspectEngine HTTP Layer

pub mod aop_http;
pub mod auth;
pub mod locale;
pub mod metrics;
pub mod permission;
pub mod rate_limit;
pub mod request_id;
pub mod security_headers;
pub mod tenant;
