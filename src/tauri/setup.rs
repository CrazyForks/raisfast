//! Tauri 应用初始化和启动
//!
//! 在实际 Tauri 项目中，需要创建 `src-tauri/` 目录并配置 `tauri.conf.json`。
//! 本模块提供命令注册和状态管理，供 `tauri::Builder` 使用。

use crate::config::app::AppConfig;
use crate::tauri::AppManagedState;

/// 获取所有 Tauri command 的注册闭包。
///
/// 用法：
/// ```ignore
/// tauri::Builder::default()
///     .manage(state)
///     .invoke_handler(register_commands())
///     .run(tauri::generate_context!())
/// ```
pub fn register_commands() -> impl Fn(tauri::ipc::Invoke) -> bool {
    tauri::generate_handler![
        crate::tauri::commands::auth_register,
        crate::tauri::commands::auth_login,
        crate::tauri::commands::auth_get_me,
        crate::tauri::commands::post_list,
        crate::tauri::commands::post_get,
        crate::tauri::commands::post_create,
        crate::tauri::commands::cms_list,
        crate::tauri::commands::cms_get,
        crate::tauri::commands::cms_create,
        crate::tauri::commands::cms_update,
        crate::tauri::commands::cms_delete,
        crate::tauri::commands::cms_single_get,
        crate::tauri::commands::cms_single_update,
        crate::tauri::commands::stats_overview,
        crate::tauri::commands::options_get,
        crate::tauri::commands::options_set,
        crate::tauri::commands::media_list,
        crate::tauri::commands::schema_list,
        crate::tauri::commands::schema_get,
        crate::tauri::commands::schema_create,
        crate::tauri::commands::schema_delete,
    ]
}

/// 构建 AppState 并包装为 Tauri managed state
pub async fn build_state(config: &AppConfig) -> anyhow::Result<AppManagedState> {
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let state = crate::build_app_state(config, rx).await?;
    Ok(AppManagedState(state))
}
