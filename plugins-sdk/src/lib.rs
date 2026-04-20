//! rust-blog 插件开发 SDK（Component Model）
//!
//! 基于 WIT 接口定义，提供插件开发所需的类型和辅助工具。
//!
//! # 快速开始
//!
//! 插件 crate 需要直接调用 `wit_bindgen::generate!`（在插件 crate 内生成 export! 宏）：
//!
//! ```rust,ignore
//! // lib.rs
//! use rust_blog_plugin_sdk::*;
//!
//! wit_bindgen::generate!({
//!     path: "../plugins-protocol/wit/plugin.wit",
//!     world: "plugin-world",
//! });
//!
//! struct MyPlugin;
//!
//! impl Guest for MyPlugin {
//!     fn on_post_creating(input: PostInput) -> Option<PostInput> {
//!         let mut input = input;
//!         input.excerpt = Some("auto".into());
//!         Some(input)
//!     }
//! }
//!
//! export!(MyPlugin);
//! ```

pub use wit_bindgen;

// 预生成类型定义，这样 SDK 的 Plugin trait 可以引用它们
wit_bindgen::generate!({
    path: "../plugins-protocol/wit/plugin.wit",
    world: "plugin-world",
});

pub use exports::rust_blog::plugin_protocol::plugin_hooks::Guest;
pub use rust_blog::plugin_protocol::types::{CommentInput, ContentEvent, PostInput, PostOutput};
