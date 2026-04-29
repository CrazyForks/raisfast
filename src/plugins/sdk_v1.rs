//! 插件 SDK 源码（编译时嵌入二进制）
//!
//! SDK 文件位于项目根目录 `sdk/` 下，通过 `include_str!` 编译进 Rust 二进制。
//! 不同版本的 SDK 对应不同的常量，插件通过 `plugin.toml` 的 `sdk_version` 选择版本。

pub const JS_SDK_V1: &str = include_str!("../../sdk/js_plugin_v1.js");
pub const JS_SDK_V1_VERSION: &str = "1.0.0";

pub const LUA_SDK_V1: &str = include_str!("../../sdk/lua_plugin_v1.lua");
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";

/// 根据 runtime 和 version 返回对应的 SDK 源码
#[must_use]
pub fn get_sdk_source(runtime: &str, version: &str) -> Option<&'static str> {
    match (runtime, version) {
        ("js", "v1") => Some(JS_SDK_V1),
        ("lua", "v1") => Some(LUA_SDK_V1),
        _ => None,
    }
}
