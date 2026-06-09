//! Data model layer (models)
//!
//! This module defines all data structures for raisfast and the raw SQL queries executed via sqlx.
//!
//! Each sub-module corresponds to a domain entity, containing:
//! - Database row models (full fields, directly mapped to database tables)
//! - API response models (safe external views, e.g. filtering out password hashes)
//! - Request validation structs (with `validator` constraints)
//! - CRUD and other database operation functions
//!
//! # Sub-modules
//! - [`user`] — User model and auth-related queries
//! - [`post`] — Post model and queries
//! - [`category`] — Category model and queries
//! - [`tag`] — Tag model and queries
//! - [`comment`] — Comment model and queries
//! - [`media`] — Media file model and queries
//! - [`refresh_token`] — Refresh token model and queries

pub mod api_token;
pub mod audit_log;
pub mod cart_item;
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
pub mod order_item;
pub mod page;
pub mod password_reset;
pub mod payment_channel;
pub mod payment_order;
pub mod payment_refund;
pub mod payment_transaction;
pub mod plugin_storage;
pub mod post;
pub mod product;
pub mod product_category;
pub mod product_comment;
pub mod product_variant;
pub mod rbac;
pub mod refresh_token;
pub mod reusable_block;
pub mod shipping_template;
pub mod sms_code;
pub mod tag;
pub mod tagging;
pub mod tenant;
pub mod user;
pub mod user_address;
pub mod user_credential;
pub mod wallet;
pub mod wallet_outbox;
pub mod wallet_transaction;

#[cfg(feature = "export-types")]
export_types!(
    user::UserRole,
    user::UserStatus,
    user::RegisteredVia,
    user_credential::AuthType,
    post::PostStatus,
    post::CommentOpenStatus,
    comment::CommentStatus,
    comment::AdminCommentRow,
    page::PageStatus,
    page::Page,
    page::PageBlock,
    page::GalleryImage,
    page::TestimonialItem,
    page::FaqItem,
    page::StatItem,
    page::TimelineItem,
    page::TeamMember,
    page::SocialLink,
    page::PricingPlan,
    page::FormFieldDef,
    page::ColumnDef,
    category::Category,
    tag::Tag,
    post::TagBrief,
    rbac::Role,
    rbac::Permission,
    tenant::Tenant,
    tenant::TenantStatus,
    api_token::ApiTokenListItem,
    audit_log::AuditEntry,
    content_revision::ContentRevision,
    content_revision::RevisionSummary,
    reusable_block::ReusableBlock,
    wallet::WalletStatus,
    wallet_transaction::WalletEntryType,
    wallet_transaction::WalletTxType,
    wallet_transaction::WalletReferenceType,
    options::OptionType,
    comment::CommentResponse,
    product_comment::ProductCommentStatus,
    product_comment::ProductCommentStats,
    product_comment::RatingBucket,
    coupon::CouponType,
    coupon::CouponStatus,
);
