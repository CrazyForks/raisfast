//! Tauri application initialization and startup
//!
//! In an actual Tauri project, you need to create a `src-tauri/` directory and configure `tauri.conf.json`.
//! This module provides command registration and state management for use with `tauri::Builder`.

use crate::config::app::AppConfig;
use crate::tauri::AppManagedState;

/// Returns a registration closure for all Tauri commands.
///
/// Usage:
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

/// Build AppState and wrap it as Tauri managed state
pub async fn build_state(config: &AppConfig) -> anyhow::Result<AppManagedState> {
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let state = crate::build_app_state(config, rx).await?;
    Ok(AppManagedState(state))
}
