//! WASM Component Model 宿主端绑定
//!
//! 由 `wasmtime::component::bindgen!` 从 WIT 文件自动生成。
//! 生成 `PluginWorld` 结构体用于类型化调用插件导出函数。

#[cfg(feature = "plugin-wasm")]
wasmtime::component::bindgen!({
    path: "plugins-protocol/wit/plugin.wit",
    world: "plugin-world",
});
