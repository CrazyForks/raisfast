//! Plugin SDK source code (embedded in binary at compile time)
//!
//! SDK files are located in the project root `plugin-sdk/` directory and compiled into the Rust binary via `include_str!`.
//! Different SDK versions correspond to different constants; plugins select a version via `sdk_version` in `plugin.toml`.

pub const JS_SDK_V1: &str = include_str!("../../plugin-sdk/js/js_plugin_v1.js");
pub const JS_SDK_V1_VERSION: &str = "1.0.0";

pub const LUA_SDK_V1: &str = include_str!("../../plugin-sdk/lua/lua_plugin_v1.lua");
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";

/// Return the corresponding SDK source code based on runtime and version
#[must_use]
pub fn get_sdk_source(runtime: &str, version: &str) -> Option<&'static str> {
    match (runtime, version) {
        ("js", "v1") => Some(JS_SDK_V1),
        ("lua", "v1") => Some(LUA_SDK_V1),
        ("rhai", _) => Some(""),
        _ => None,
    }
}
