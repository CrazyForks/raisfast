//! Service layer (business logic).
//!
//! This module contains the core business logic of raisfast. The service layer sits between handlers and models:
//!
//! - Called by **handlers** with parsed request parameters.
//! - Calls the **models** layer to perform database operations.
//! - Responsible for data validation, permission checks, and business rule enforcement.

pub mod api_token;
pub mod aspect_dispatch;
pub mod auth;
pub mod category;
pub mod comment;
pub mod email_verification;
pub mod media;
pub mod oauth;
pub mod order;
pub mod options;
pub mod page;
pub mod password_reset;
pub mod post;
pub mod product;
pub mod rbac;
pub mod reusable_block;
pub mod sms;
pub mod stats;
pub mod tag;
pub mod tenant;
pub mod user;
pub mod wallet;
