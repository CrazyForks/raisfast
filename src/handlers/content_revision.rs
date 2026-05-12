//! Content revision history API handler
//!
//! Provides revision management endpoints for content types with `versioning` enabled:
//! - List revision history
//! - Get a specific revision snapshot
//! - Restore to a specific revision
//! - Diff between two revisions

use axum::extract::{Path, State};
use serde_json::json;

use crate::AppState;
use crate::constants::COL_ID;
use crate::content_type::repository::ContentRepository;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;

/// GET /admin/cms/{plural}/{id}/revisions — List all revisions of a record
pub async fn list_revisions(
    State(state): State<AppState>,
    Path((plural, id)): Path<(String, String)>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let ct = state
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| AppError::not_found(&plural))?;

    if !ct.has_revision_routes() {
        return Err(AppError::BadRequest(
            "versioning is not enabled for this content type".into(),
        ));
    }

    let summaries =
        crate::models::content_revision::list_revisions(&state.pool, &ct.singular, &id).await?;

    Ok(ApiResponse::success(json!({
        "items": summaries,
        "total": summaries.len(),
    })))
}

/// GET /admin/cms/{plural}/{id}/revisions/{revision_id} — Get a specific revision snapshot
pub async fn get_revision(
    State(state): State<AppState>,
    Path((plural, id, revision_id)): Path<(String, String, String)>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let ct = state
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| AppError::not_found(&plural))?;

    if !ct.has_revision_routes() {
        return Err(AppError::BadRequest(
            "versioning is not enabled for this content type".into(),
        ));
    }

    let rev_id: i64 = revision_id
        .parse()
        .map_err(|_| AppError::BadRequest("invalid revision_id".into()))?;

    let revision =
        crate::models::content_revision::get_revision(&state.pool, &ct.singular, &id, rev_id)
            .await?
            .ok_or_else(|| AppError::not_found(&revision_id))?;

    let snapshot: serde_json::Value = serde_json::from_str(&revision.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;

    Ok(ApiResponse::success(json!({
        "id": revision.id,
        "revision_number": revision.revision_number,
        "snapshot": snapshot,
        "created_by": revision.created_by,
        "created_at": revision.created_at,
    })))
}

/// POST /admin/cms/{plural}/{id}/revisions/{revision_id}/restore — Restore to a specific revision
pub async fn restore_revision(
    State(state): State<AppState>,
    Path((plural, id, revision_id)): Path<(String, String, String)>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let ct = state
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| AppError::not_found(&plural))?;

    if !ct.has_revision_routes() {
        return Err(AppError::BadRequest(
            "versioning is not enabled for this content type".into(),
        ));
    }

    let rev_id: i64 = revision_id
        .parse()
        .map_err(|_| AppError::BadRequest("invalid revision_id".into()))?;

    let revision =
        crate::models::content_revision::get_revision(&state.pool, &ct.singular, &id, rev_id)
            .await?
            .ok_or_else(|| AppError::not_found(&revision_id))?;

    let mut snapshot: serde_json::Value = serde_json::from_str(&revision.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;

    if let Some(obj) = snapshot.as_object_mut() {
        obj.remove(COL_ID);
        obj.remove("created_at");
        obj.remove("updated_at");
    }

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo
        .update(&ct, &id, snapshot, None, &Default::default())
        .await?;

    let value = serde_json::to_value(result)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize result: {e}")))?;
    Ok(ApiResponse::success(value))
}

/// GET /admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b} — Compare two revisions
pub async fn diff_revisions(
    State(state): State<AppState>,
    Path((plural, id, rev_a, rev_b)): Path<(String, String, String, String)>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let ct = state
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| AppError::not_found(&plural))?;

    if !ct.has_revision_routes() {
        return Err(AppError::BadRequest(
            "versioning is not enabled for this content type".into(),
        ));
    }

    let rev_a_id: i64 = rev_a
        .parse()
        .map_err(|_| AppError::BadRequest("invalid revision_id".into()))?;
    let rev_b_id: i64 = rev_b
        .parse()
        .map_err(|_| AppError::BadRequest("invalid revision_id".into()))?;

    let a = crate::models::content_revision::get_revision(&state.pool, &ct.singular, &id, rev_a_id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("revision {rev_a}")))?;

    let b = crate::models::content_revision::get_revision(&state.pool, &ct.singular, &id, rev_b_id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("revision {rev_b}")))?;

    let snap_a: serde_json::Value = serde_json::from_str(&a.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot A parse: {e}")))?;
    let snap_b: serde_json::Value = serde_json::from_str(&b.snapshot)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot B parse: {e}")))?;

    let diff = crate::models::content_revision::compute_diff(&snap_a, &snap_b);

    Ok(ApiResponse::success(json!({
        "revision_a": {
            "id": a.id,
            "revision_number": a.revision_number,
            "created_at": a.created_at,
        },
        "revision_b": {
            "id": b.id,
            "revision_number": b.revision_number,
            "created_at": b.created_at,
        },
        "diff": diff,
    })))
}

/// utoipa path annotation placeholder (to be added uniformly later)
const _UNUSED: () = {
    fn _assert_send() {
        fn check<T: Send>() {}
        check::<fn() -> ()>();
    }
};
