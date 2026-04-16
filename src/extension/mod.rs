//! Extension 系统 — 统一管理 Content Type 和 Plugin
//!
//! Extension 是平台的核心扩展单元，可包含以下组件的任意组合：
//! - Content Type（TOML Schema → 自动建表 + CRUD API）
//! - Plugin（WASM/JS/Lua 代码 → Hook + 自定义路由 + Cron）
//!
//! # 目录结构
//!
//! ```text
//! extensions/
//! ├── blog-core/
//! │   ├── extension.toml
//! │   └── content_types/
//! │       ├── post.toml
//! │       └── category.toml
//! ├── seo-optimizer/
//! │   ├── extension.toml
//! │   └── plugin/
//! │       ├── manifest.toml
//! │       └── main.js
//! └── ecommerce/
//!     ├── extension.toml
//!     ├── content_types/
//!     │   └── product.toml
//!     └── plugin/
//!         ├── manifest.toml
//!         └── main.js
//! ```
//!
//! # 生命周期
//!
//! Install → Enable → (运行中) → Disable → Uninstall

pub mod handler;
pub mod manager;
pub mod manifest;
pub mod model;
pub mod service;
