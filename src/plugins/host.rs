//! 宿主提供给插件的 Host Functions
//!
//! 这些函数在 WASM 线性内存中通过 `env` 模块暴露给插件。

use std::sync::Arc;

use crate::config::app::AppConfig;

#[allow(dead_code)]
pub fn register_host_functions(
    linker: &mut wasmtime::Linker<HostState>,
    _config: Arc<AppConfig>,
) -> anyhow::Result<()> {
    linker.func_wrap("env", "host_log", |level: i32, msg: i32| {
        let level_str = match level {
            0 => "trace",
            1 => "debug",
            2 => "info",
            3 => "warn",
            _ => "error",
        };
        tracing::info!("[plugin] {level_str}: message_code={msg}");
    })?;

    Ok(())
}

#[allow(dead_code)]
pub struct HostState {
    pub plugin_id: String,
}
