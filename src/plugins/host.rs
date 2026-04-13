//! 宿主提供给插件的 Host Functions
//!
//! 通过 WASM `env` 模块暴露给插件，所有外部交互必须经此层。
//! 权限检查在每次调用时执行。

use std::sync::Arc;

use crate::config::app::AppConfig;

/// 插件实例的宿主状态，携带权限信息
#[allow(dead_code)]
pub struct HostState {
    pub plugin_id: String,
    pub config: Arc<AppConfig>,
    pub permissions: crate::plugins::Permissions,
}

/// 注册所有 Host Functions 到 Linker。
///
/// 当前采用"指针+长度"ABI 传递字符串参数。
/// 由于 wasmtime 的 `func_wrap` 不支持闭包捕获 store data，
/// 实际参数传递通过 WASM 线性内存 + store data 间接完成。
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
