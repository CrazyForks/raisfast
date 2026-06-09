//! Tauri Commands — business logic corresponding to HTTP handlers
//!
//! Each command calls the same function in the `services/` layer,
//! replacing Axum extractors with plain function parameters.

use crate::content_type::repository::SaveContext;
use crate::tauri::AppManagedState;
use tauri::State as TauriState;

// ── Auth ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn auth_register(
    state: TauriState<'_, AppManagedState>,
    username: String,
    email: String,
    password: String,
) -> Result<serde_json::Value, String> {
    let req = crate::dto::RegisterRequest {
        username,
        email,
        password,
    };
    crate::services::auth::register(
        state.0.user_repo.as_ref(),
        &state.0.eventbus,
        req,
        None,
        false,
        &state.0.pool,
    )
    .await
    .map_err(|e| e.to_string())
    .map(|r| serde_json::to_value(r).unwrap_or_default())
}

#[tauri::command]
pub async fn auth_login(
    state: TauriState<'_, AppManagedState>,
    email: String,
    password: String,
) -> Result<serde_json::Value, String> {
    let req = crate::dto::LoginRequest { email, password };
    crate::services::auth::login(
        state.0.user_repo.as_ref(),
        state.0.refresh_token_repo.as_ref(),
        &state.0.plugins,
        &state.0.eventbus,
        &req,
        &state.0.config.jwt_secret,
        state.0.config.jwt_access_expires,
        state.0.config.jwt_refresh_expires,
        None,
        false,
    )
    .await
    .map_err(|e| e.to_string())
    .map(|r| serde_json::to_value(r).unwrap_or_default())
}

#[tauri::command]
pub async fn auth_get_me(
    state: TauriState<'_, AppManagedState>,
    user_id: String,
) -> Result<serde_json::Value, String> {
    let uid: i64 = user_id.parse().map_err(|e| e.to_string())?;
    let auth = crate::middleware::auth::AuthUser::from_parts(
        Some(uid),
        crate::models::user::UserRole::Reader,
        None,
    );
    crate::services::user::get_me(state.0.user_repo.as_ref(), &auth)
        .await
        .map_err(|e| e.to_string())
        .map(|r| serde_json::to_value(r).unwrap_or_default())
}

// ── Posts ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn post_list(
    state: TauriState<'_, AppManagedState>,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<serde_json::Value, String> {
    let auth = crate::middleware::auth::AuthUser::from_parts(
        None,
        crate::models::user::UserRole::Reader,
        None,
    );
    let (items, total) = state
        .0
        .post_service
        .list(
            &auth,
            page.unwrap_or(1),
            page_size.unwrap_or(20),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page.unwrap_or(1),
        "page_size": page_size.unwrap_or(20),
    }))
}

#[tauri::command]
pub async fn post_get(
    state: TauriState<'_, AppManagedState>,
    slug: String,
) -> Result<serde_json::Value, String> {
    let auth = crate::middleware::auth::AuthUser::from_parts(
        None,
        crate::models::user::UserRole::Reader,
        None,
    );
    state
        .0
        .post_service
        .get(&auth, &slug)
        .await
        .map_err(|e| e.to_string())
        .map(|r| serde_json::to_value(r).unwrap_or_default())
}

#[tauri::command]
pub async fn post_create(
    state: TauriState<'_, AppManagedState>,
    user_id: String,
    title: String,
    content: String,
) -> Result<serde_json::Value, String> {
    let req = crate::dto::CreatePostRequest {
        title,
        content,
        excerpt: None,
        cover_image: None,
        image_ids: None,
        status: None,
        category_id: None,
        tag_ids: None,
        meta_title: None,
        meta_description: None,
        og_title: None,
        og_description: None,
        og_image: None,
        canonical_url: None,
    };
    let uid: i64 = user_id.parse().map_err(|e| e.to_string())?;
    let auth = crate::middleware::auth::AuthUser::from_parts(
        Some(uid),
        crate::models::user::UserRole::Author,
        None,
    );
    state
        .0
        .post_service
        .create(&auth, req)
        .await
        .map_err(|e| e.to_string())
        .map(|r| serde_json::to_value(r).unwrap_or_default())
}

// ── CMS Content Types ─────────────────────────────────────────────

#[tauri::command]
pub async fn cms_single_get(
    state: TauriState<'_, AppManagedState>,
    singular: String,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| format!("content type '{}' not found", singular))?;
    if !ct.is_single() {
        return Err(format!("content type '{}' is not a single type", singular));
    }
    crate::content_type::handler::do_single_get(&state.0, &ct, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_single_update(
    state: TauriState<'_, AppManagedState>,
    singular: String,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| format!("content type '{}' not found", singular))?;
    if !ct.is_single() {
        return Err(format!("content type '{}' is not a single type", singular));
    }
    let save_ctx = crate::content_type::repository::SaveContext::default();
    crate::content_type::handler::do_single_update(&state.0, &ct, data, &save_ctx, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_list(
    state: TauriState<'_, AppManagedState>,
    plural: String,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| format!("content type '{}' not found", plural))?;
    let params = crate::content_type::handler::ListParams {
        page,
        page_size,
        sort: None,
        status: None,
        search: None,
        include: None,
        skip_total: None,
        extra: Default::default(),
    };
    crate::content_type::handler::do_list(&state.0, &ct, params, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_get(
    state: TauriState<'_, AppManagedState>,
    plural: String,
    id: String,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| format!("content type '{}' not found", plural))?;
    crate::content_type::handler::do_get(&state.0, &ct, &id, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_create(
    state: TauriState<'_, AppManagedState>,
    plural: String,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| format!("content type '{}' not found", plural))?;
    let save_ctx = SaveContext::default();
    crate::content_type::handler::do_create(&state.0, &ct, data, &save_ctx)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_update(
    state: TauriState<'_, AppManagedState>,
    plural: String,
    id: String,
    data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| format!("content type '{}' not found", plural))?;
    let save_ctx = SaveContext::default();
    crate::content_type::handler::do_update(&state.0, &ct, &id, data, &save_ctx, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cms_delete(
    state: TauriState<'_, AppManagedState>,
    plural: String,
    id: String,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get_by_plural(&plural)
        .ok_or_else(|| format!("content type '{}' not found", plural))?;
    crate::content_type::handler::do_delete(&state.0, &ct, &id, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"deleted": true}))
}

// ── Stats / Dashboard ─────────────────────────────────────────────

#[tauri::command]
pub async fn stats_overview(
    state: TauriState<'_, AppManagedState>,
) -> Result<serde_json::Value, String> {
    let svc = crate::services::stats::StatsService::new(state.0.pool.clone());
    svc.overview(None).await.map_err(|e| e.to_string())
}

// ── Options ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn options_get(
    state: TauriState<'_, AppManagedState>,
    key: String,
) -> Result<serde_json::Value, String> {
    Ok(state
        .0
        .options
        .get(&key)
        .await
        .unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub async fn options_set(
    state: TauriState<'_, AppManagedState>,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    state
        .0
        .options
        .set(&key, value)
        .await
        .map_err(|e| e.to_string())
}

// ── Media ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn media_list(
    state: TauriState<'_, AppManagedState>,
    user_id: String,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<serde_json::Value, String> {
    let uid: i64 = user_id.parse().map_err(|e| e.to_string())?;
    let auth = crate::middleware::auth::AuthUser::from_parts(
        Some(uid),
        crate::models::user::UserRole::Reader,
        None,
    );
    let (items, total) = crate::services::media::list(
        state.0.media_repo.as_ref(),
        &auth,
        page.unwrap_or(1),
        page_size.unwrap_or(20),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page.unwrap_or(1),
        "page_size": page_size.unwrap_or(20),
    }))
}

// ── Content Type Schema Management ────────────────────────────────

#[tauri::command]
pub async fn schema_list(
    state: TauriState<'_, AppManagedState>,
) -> Result<serde_json::Value, String> {
    let schemas = state.0.content_type_registry.all();
    let list: Vec<serde_json::Value> = schemas
        .iter()
        .map(|s| serde_json::to_value(s.as_ref()).unwrap_or_default())
        .collect();
    Ok(serde_json::json!({"items": list, "total": list.len()}))
}

#[tauri::command]
pub async fn schema_get(
    state: TauriState<'_, AppManagedState>,
    singular: String,
) -> Result<serde_json::Value, String> {
    let ct = state
        .0
        .content_type_registry
        .get(&singular)
        .ok_or_else(|| format!("content type '{}' not found", singular))?;
    Ok(serde_json::to_value(ct.as_ref()).unwrap_or_default())
}

#[tauri::command]
pub async fn schema_create(
    state: TauriState<'_, AppManagedState>,
    req: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let req: crate::content_type::schema::CreateContentTypeRequest =
        serde_json::from_value(req).map_err(|e| format!("invalid request: {e}"))?;

    let schema = crate::content_type::schema::ContentTypeSchema {
        name: req.name,
        singular: req.singular.clone(),
        plural: req.plural,
        table: req.table.clone(),
        description: req.description,
        kind: req.kind,
        slug_field: req.slug_field,
        builtin: req.builtin,
        implements: req.implements,
        fields: req.fields,
        indexes: vec![],
        api: crate::content_type::schema::ApiConfig::default(),
        cached_column_names: None,
        cached_protocol_column_names: None,
        cached_behaviors: None,
        cached_declaration: None,
        cached_rules: None,
    };

    if crate::plugins::permissions::PermissionChecker::is_protected_table(
        &schema.table,
        &state.0.config.builtins.protected_tables(),
    ) {
        return Err(format!(
            "table '{}' is a protected system table",
            schema.table
        ));
    }

    if state
        .0
        .content_type_registry
        .get(&schema.singular)
        .is_some()
    {
        return Err(format!("content type '{}' already exists", schema.singular));
    }

    let dir = std::path::Path::new(&state.0.config.content_type_dir);
    schema
        .save_to_dir(dir)
        .map_err(|e| format!("save failed: {e}"))?;

    let repo = crate::content_type::repository::ContentRepository::new(state.0.pool.clone());
    repo.migrate(&schema, &state.0.protocol_registry)
        .await
        .map_err(|e| format!("migration failed: {e}"))?;

    let reserved = state.0.config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = state.0.protocol_registry.names();
    state
        .0
        .content_type_registry
        .register(
            schema.clone(),
            &state.0.config.rule_engine,
            &reserved,
            &protocol_names,
            &state.0.protocol_registry,
        )
        .map_err(|e| e.to_string())?;

    Ok(serde_json::to_value(&schema).unwrap_or_default())
}

#[tauri::command]
pub async fn schema_delete(
    state: TauriState<'_, AppManagedState>,
    singular: String,
) -> Result<serde_json::Value, String> {
    if state.0.content_type_registry.get(&singular).is_none() {
        return Err(format!("content type '{}' not found", singular));
    }

    let path =
        std::path::Path::new(&state.0.config.content_type_dir).join(format!("{singular}.toml"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
    }

    state.0.content_type_registry.unregister(&singular);

    Ok(serde_json::json!({"deleted": true}))
}
