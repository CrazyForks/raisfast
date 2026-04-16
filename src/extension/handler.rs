//! Extension Admin API Handler

use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AdminUser;

/// Extension 列表项（合并 DB 记录和运行时信息）
#[derive(Debug, Serialize)]
pub struct ExtensionListItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub installed: bool,
    pub has_content_types: bool,
    pub has_plugin: bool,
    pub content_types: Vec<String>,
    pub dependencies: std::collections::HashMap<String, String>,
    pub installed_at: Option<String>,
}

/// GET /admin/extensions — 列出所有 Extension
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<ExtensionListItem>>> {
    let loaded = state.extension_manager.list_loaded();
    let installed = state
        .extension_service
        .list_installed()
        .await
        .unwrap_or_default();

    let installed_map: std::collections::HashMap<String, String> = installed
        .into_iter()
        .map(|r| (r.id, r.installed_at))
        .collect();

    let items: Vec<ExtensionListItem> = loaded
        .into_iter()
        .map(|ext| {
            let installed_at = installed_map.get(&ext.manifest.extension.id).cloned();
            ExtensionListItem {
                id: ext.manifest.extension.id.clone(),
                name: ext.manifest.extension.name.clone(),
                version: ext.manifest.extension.version.clone(),
                description: ext.manifest.extension.description.clone(),
                enabled: ext.enabled,
                installed: installed_at.is_some(),
                has_content_types: ext.manifest.has_content_types(),
                has_plugin: ext.has_plugin,
                content_types: ext.content_type_names.clone(),
                dependencies: ext.manifest.extension.dependencies.clone(),
                installed_at,
            }
        })
        .collect();

    Ok(ApiResponse::success(items))
}

/// GET /admin/extensions/:id — 获取单个 Extension 详情
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ExtensionListItem>> {
    let ext = state
        .extension_manager
        .get_loaded(&id)
        .ok_or_else(|| crate::errors::app_error::AppError::not_found("extension"))?;

    let db_record = state.extension_service.get(&id).await.ok().flatten();

    let item = ExtensionListItem {
        id: ext.manifest.extension.id.clone(),
        name: ext.manifest.extension.name.clone(),
        version: ext.manifest.extension.version.clone(),
        description: ext.manifest.extension.description.clone(),
        enabled: ext.enabled,
        installed: db_record.is_some(),
        has_content_types: ext.manifest.has_content_types(),
        has_plugin: ext.has_plugin,
        content_types: ext.content_type_names.clone(),
        dependencies: ext.manifest.extension.dependencies.clone(),
        installed_at: db_record.map(|r| r.installed_at),
    };

    Ok(ApiResponse::success(item))
}

/// POST /admin/extensions/:id/enable — 启用 Extension
pub async fn enable(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.extension_manager.enable(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /admin/extensions/:id/disable — 禁用 Extension
pub async fn disable(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.extension_manager.disable(&id).await?;
    Ok(ApiResponse::success(()))
}

/// DELETE /admin/extensions/:id — 卸载 Extension
pub async fn uninstall(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<UninstallRequest>,
) -> AppResult<ApiResponse<()>> {
    state
        .extension_manager
        .uninstall(&id, body.drop_tables)
        .await?;
    Ok(ApiResponse::success(()))
}

#[derive(Debug, Deserialize)]
pub struct UninstallRequest {
    #[serde(default)]
    pub drop_tables: bool,
}
