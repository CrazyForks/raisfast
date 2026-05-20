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
use crate::content_type::repository::ContentRepository;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;

/// GET /admin/cms/{plural}/{id}/revisions — List all revisions of a record
#[utoipa::path(get, path = "/admin/cms/{plural}/{id}/revisions", tag = "revisions",
    security(("bearer_auth" = [])),
    params(("plural" = String, Path, description = "Content type plural"), ("id" = String, Path, description = "Record ID")),
    responses((status = 200, description = "Revision list"))
)]
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

    let int_id = crate::types::snowflake_id::parse_id(&id)?;

    let (summaries, total) =
        crate::services::content_revision::list_revisions(&state.pool, &ct.singular, int_id, 0, 0)
            .await?;

    Ok(ApiResponse::success(json!({
        "items": summaries,
        "total": total,
    })))
}

/// GET /admin/cms/{plural}/{id}/revisions/{revision_id} — Get a specific revision snapshot
#[utoipa::path(get, path = "/admin/cms/{plural}/{id}/revisions/{revision_id}", tag = "revisions",
    security(("bearer_auth" = [])),
    params(("plural" = String, Path, description = "Content type plural"), ("id" = String, Path, description = "Record ID"), ("revision_id" = String, Path, description = "Revision ID")),
    responses((status = 200, description = "Revision detail"))
)]
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

    let rev_id = crate::types::snowflake_id::parse_id(&revision_id)?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;

    let revision =
        crate::services::content_revision::get_revision(&state.pool, &ct.singular, int_id, rev_id)
            .await?;

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
#[utoipa::path(post, path = "/admin/cms/{plural}/{id}/revisions/{revision_id}/restore", tag = "revisions",
    security(("bearer_auth" = [])),
    params(("plural" = String, Path, description = "Content type plural"), ("id" = String, Path, description = "Record ID"), ("revision_id" = String, Path, description = "Revision ID")),
    responses((status = 200, description = "Revision restored"))
)]
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

    let rev_id = crate::types::snowflake_id::parse_id(&revision_id)?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;

    let snapshot = crate::services::content_revision::restore_revision(
        &state.pool,
        &ct.singular,
        int_id,
        rev_id,
    )
    .await?;

    let repo = ContentRepository::new(state.pool.clone());
    let result = repo
        .update(&ct, int_id, snapshot, None, &Default::default())
        .await?;

    let value = serde_json::to_value(result)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize result: {e}")))?;
    Ok(ApiResponse::success(value))
}

/// GET /admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b} — Compare two revisions
#[utoipa::path(get, path = "/admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b}", tag = "revisions",
    security(("bearer_auth" = [])),
    params(("plural" = String, Path, description = "Content type plural"), ("id" = String, Path, description = "Record ID"), ("rev_a" = String, Path, description = "Revision A"), ("rev_b" = String, Path, description = "Revision B")),
    responses((status = 200, description = "Revision diff"))
)]
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

    let rev_a_id = crate::types::snowflake_id::parse_id(&rev_a)?;
    let rev_b_id = crate::types::snowflake_id::parse_id(&rev_b)?;
    let int_id = crate::types::snowflake_id::parse_id(&id)?;

    let result = crate::services::content_revision::diff_revisions(
        &state.pool,
        &ct.singular,
        int_id,
        *rev_a_id,
        *rev_b_id,
    )
    .await?;

    Ok(ApiResponse::success(json!({
        "revision_a": {
            "id": result.revision_a.id,
            "revision_number": result.revision_a.revision_number,
            "created_at": result.revision_a.created_at,
        },
        "revision_b": {
            "id": result.revision_b.id,
            "revision_number": result.revision_b.revision_number,
            "created_at": result.revision_b.created_at,
        },
        "diff": result.diff,
    })))
}

/// utoipa path annotation placeholder (to be added uniformly later)
const _UNUSED: () = {
    fn _assert_send() {
        fn check<T: Send>() {}
        check::<fn() -> ()>();
    }
};
