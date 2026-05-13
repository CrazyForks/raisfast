//! HTTP request handlers
//!
//! This module contains all Axum route handler functions. Each handler follows a **thin layer** principle:
//! only responsible for extracting request parameters, calling the business logic layer (services),
//! and returning HTTP responses.
//!
//! Handlers do **not** contain any business logic; all business logic is implemented in the `services` layer.
//!
//! # Sub-modules
//! - [`auth`] — Register, login, logout, refresh token
//! - [`user`] — User profile view and edit
//! - [`category`] — Category CRUD
//! - [`tag`] — Tag CRUD
//! - [`post`] — Post CRUD and listing
//! - [`comment`] — Comment creation, moderation, deletion
//! - [`media`] — Media file upload, listing, deletion
//! - [`rss`] — RSS feed
//! - [`health`] — Health check

pub mod api_token;
pub mod auth;
pub mod category;
pub mod comment;
pub mod content_revision;
pub mod cron;
pub mod currencies;
pub mod health;
pub mod media;
pub mod oauth;
pub mod options;
pub mod order;
pub mod page;
pub mod payment;
pub mod plugin;
pub mod post;
pub mod product;
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
