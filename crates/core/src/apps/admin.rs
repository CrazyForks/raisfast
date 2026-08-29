//! Admin API handlers for App Bundle lifecycle (app-bundle.md §11, MVP
//! subset: preview/install/list/detail/enable/disable/uninstall).

use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::{AuthUser, TokenAction};

/// GET /admin/apps — installed apps with status.
pub async fn list_apps(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<Value>>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Read)?;
    let reg = state.apps.clone();
    let rows = reg.list().await?;
    Ok(ApiResponse::success(rows.iter().map(app_summary).collect()))
}

/// GET /admin/apps/{app_id} — detail incl. the full compensation log.
pub async fn get_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Read)?;
    let reg = state.apps.clone();
    let row = reg.detail(&app_id).await?;
    Ok(ApiResponse::success(app_detail(&row)))
}

/// POST /admin/apps/install-preview — multipart package upload → precheck
/// report (no mutation).
pub async fn install_preview(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Create)?;
    let reg = state.apps.clone();

    let upload = read_package_upload(&mut multipart).await?;
    let (pkg, _guard) = unpack_once(&reg, &state, upload).await?;
    let report = reg.install_preview(&pkg).await?;
    let value =
        serde_json::to_value(&report).map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    Ok(ApiResponse::success(value))
}

/// POST /admin/apps/install — multipart package upload + optional
/// `options` JSON field → full install.
pub async fn install(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Create)?;
    let reg = state.apps.clone();

    let upload = read_package_upload(&mut multipart).await?;
    let opts: crate::apps::InstallOptions = upload
        .options
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let (pkg, _guard) = unpack_once(&reg, &state, upload).await?;

    let result = reg.install(&pkg, &opts).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(result))))
}

/// POST /admin/apps/{app_id}/enable
pub async fn enable_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Update)?;
    let reg = state.apps.clone();
    reg.enable(&app_id).await?;
    Ok(ApiResponse::success(
        serde_json::json!({"app_id": app_id, "status": "enabled"}),
    ))
}

/// POST /admin/apps/{app_id}/disable
pub async fn disable_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Update)?;
    let reg = state.apps.clone();
    reg.disable(&app_id).await?;
    Ok(ApiResponse::success(
        serde_json::json!({"app_id": app_id, "status": "disabled"}),
    ))
}

/// POST /admin/apps/{app_id}/uninstall — body `{ "keep_data": bool? }`.
pub async fn uninstall_app(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    body: Option<Json<Value>>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("apps", TokenAction::Delete)?;
    let reg = state.apps.clone();
    let keep_data = body.as_ref().and_then(|Json(v)| {
        v.get("keep_data")
            .or_else(|| v.get("keep-data"))
            .and_then(Value::as_bool)
    });
    reg.uninstall(&app_id, keep_data).await?;
    Ok(ApiResponse::success(
        serde_json::json!({"app_id": app_id, "uninstalled": true}),
    ))
}

// ── helpers ─────────────────────────────────────────────────────────

struct PackageUpload {
    bytes: Vec<u8>,
    options: Option<String>,
}

async fn read_package_upload(multipart: &mut Multipart) -> AppResult<PackageUpload> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut options: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "package" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::PayloadTooLarge(format!("package read: {e}")))?;
                bytes = Some(data.to_vec());
            }
            "options" => {
                options = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("options read: {e}")))?,
                );
            }
            _ => {}
        }
    }
    let bytes = bytes.ok_or_else(|| {
        AppError::BadRequest("multipart field 'package' (.rafapp file) is required".into())
    })?;
    Ok(PackageUpload { bytes, options })
}

/// Unpack into a per-request temp dir; the guard removes it on drop.
struct UnpackGuard(std::path::PathBuf);

impl Drop for UnpackGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn unpack_once(
    reg: &crate::apps::AppRegistry,
    _state: &AppState,
    upload: PackageUpload,
) -> AppResult<(crate::apps::AppPackage, UnpackGuard)> {
    let token = format!(
        "{}-{}",
        crate::utils::id::new_id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let dir = reg.unpack_dir(&token);
    let pkg = crate::apps::AppPackage::unpack(&upload.bytes, &dir)?;
    Ok((pkg, UnpackGuard(dir)))
}

fn app_summary(row: &crate::apps::model::AppRow) -> Value {
    serde_json::json!({
        "id": row.id,
        "app_id": row.app_id,
        "version": row.version,
        "status": row.status,
        "source": row.source,
        "tenant_scope": row.tenant_scope,
        "last_error": row.last_error,
        "installed_at": row.installed_at,
        "updated_at": row.updated_at,
        "options": row.options,
        "step_count": row
            .install_log
            .as_ref()
            .and_then(|l| l.get("steps"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
    })
}

fn app_detail(row: &crate::apps::model::AppRow) -> Value {
    let mut v = app_summary(row);
    v["install_log"] = row.install_log.clone().unwrap_or(Value::Null);
    v
}
