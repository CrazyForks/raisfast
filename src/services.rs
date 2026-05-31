//! Service layer (business logic).
//!
//! This module contains the core business logic of raisfast. The service layer sits between handlers and models:
//!
//! - Called by **handlers** with parsed request parameters.
//! - Calls the **models** layer to perform database operations.
//! - Responsible for data validation, permission checks, and business rule enforcement.

pub mod api_token;
pub mod audit;
pub mod auth;
pub mod cart;
pub mod category;
pub mod comment;
pub mod content_revision;
pub mod coupon;
pub mod currencies;
pub mod email_verification;
pub mod media;
pub mod oauth;
pub mod options;
pub mod order;
pub mod page;
pub mod password_reset;
pub mod payment;
pub mod post;
pub mod product;
pub mod product_category;
pub mod product_comment;
pub mod product_variant;
pub mod rbac;
pub mod reusable_block;
pub mod shipping_template;
pub mod sms;
pub mod stats;
pub mod tag;
pub mod tenant;
pub mod user;
pub mod user_address;
pub mod wallet;

#[cfg(feature = "export-types")]
export_types!(
    options::OptionGroup,
    options::OptionEntry,
    api_token::CreateTokenResult,
    oauth::OAuthBindingInfo,
);
