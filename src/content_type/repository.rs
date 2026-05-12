//! Generic content repository — dynamic SQL CRUD
//!
//! Provides unified CRUD operations for all content types, dynamically building SQL.
//! Uses `crate::db::dialect::ph()` to support multi-database placeholders.
//!
//! Query results are extracted column-by-column via `Row::get()`, building `serde_json::Value`
//! directly, avoiding the performance overhead of `json_object()` double serialization.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::constants::*;
use crate::db::DbRow;
use crate::db::Pool;
use crate::errors::app_error::AppError;
use crate::middleware::auth::AuthUser;
use crate::protocols::ProtocolRegistry;
use sqlx::Row;

/// Save operation context (passed from handler layer to repository layer)
#[derive(Debug, Clone, Default)]
pub struct SaveContext {
    pub user_id: Option<String>,
    pub user_int_id: Option<i64>,
    pub user_role: Option<String>,
    pub tenant_id: Option<String>,
}

impl SaveContext {
    pub fn from_auth(auth: &AuthUser) -> Self {
        Self {
            user_id: auth.user_id().map(|s| s.to_string()),
            user_int_id: auth.user_int_id(),
            user_role: auth.is_authenticated().then(|| auth.role().to_string()),
            tenant_id: auth.tenant_id().map(|s| s.to_string()),
        }
    }
}

/// Common query parameters
#[derive(Debug, Clone, Default)]
pub struct ContentQuery {
    pub page: i64,
    pub page_size: i64,
    pub sort: Option<String>,
    pub filters: HashMap<String, Value>,
    pub status: Option<String>,
    pub search: Option<String>,
    pub fields: Option<Vec<String>>,
    pub tenant_id: Option<String>,
    pub include: Option<Vec<String>>,
    pub skip_total: bool,
    /// Additional WHERE clause compiled from API Rules
    pub rule_where: Option<String>,
    /// Additional parameters compiled from API Rules
    pub rule_params: Vec<String>,
    /// Maximum items per page (passed from handler via config)
    pub max_page_size: i64,
    /// Whether to include private fields (admin API sets this to true)
    pub include_private: bool,
    /// __meta JSON path query conditions: (json_path, value)
    pub meta_filters: Vec<(String, String)>,
}

/// Generic content repository
pub struct ContentRepository {
    pub pool: Pool,
}

impl ContentRepository {
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    fn resolve_tenant(&self, ct: &ContentTypeSchema, tenant_id: Option<&str>) -> Option<String> {
        if ct.implements_protocol("tenantable") {
            Some(crate::db::tenant::resolve_tenant(tenant_id).to_string())
        } else {
            None
        }
    }

    /// Paginated query
    pub async fn find(
        &self,
        ct: &ContentTypeSchema,
        query: ContentQuery,
    ) -> Result<(Vec<Value>, i64), AppError> {
        let columns = ct.column_names(query.fields.as_deref(), query.include_private);
        let select_cols = columns.join(", ");
        let table = &ct.table;

        let mut where_clauses = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut param_idx = 1;

        for (column, condition) in ct.query_filters() {
            where_clauses.push(format!("{} {}", column, condition));
        }
        if ct.query_filters().is_empty() && ct.is_soft_delete() {
            where_clauses.push(format!("{} IS NULL", COL_DELETED_AT));
        }
        let tid = self.resolve_tenant(ct, query.tenant_id.as_deref());
        if let Some(ref tid) = tid {
            where_clauses.push(format!(
                "{COL_TENANT_ID} = {}",
                crate::db::dialect::ph(param_idx)
            ));
            params.push(json!(tid));
            param_idx += 1;
        }

        for (key, val) in &query.filters {
            let matches_field = ct.get_field(key).is_some();
            let matches_fk = ct.fields.iter().any(|f| {
                f.relation
                    .as_ref()
                    .is_some_and(|r| r.foreign_key.as_deref() == Some(key.as_str()))
            });
            if (matches_field || matches_fk) && crate::db::dialect::is_safe_identifier(key) {
                where_clauses.push(format!("{key} = {}", crate::db::dialect::ph(param_idx)));
                params.push(val.clone());
                param_idx += 1;
            }
        }

        for (path, val) in &query.meta_filters {
            where_clauses.push(format!(
                "json_extract({}, {}) = {}",
                COL_META,
                crate::db::dialect::ph(param_idx),
                crate::db::dialect::ph(param_idx + 1)
            ));
            params.push(json!(format!("$.{path}")));
            params.push(json!(val));
            param_idx += 2;
        }

        let mut where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        if let Some(ref rule_where) = query.rule_where
            && !rule_where.is_empty()
        {
            let rule_params_owned = query.rule_params.clone();
            if where_sql.is_empty() {
                where_sql = format!(" WHERE {rule_where}");
            } else {
                where_sql = format!("{where_sql} AND ({rule_where})");
            }
            for p in rule_params_owned {
                params.push(Value::String(p));
            }
        }

        let count_row = if query.skip_total {
            -1
        } else {
            let count_sql = format!("SELECT COUNT(*) as cnt FROM {table}{where_sql}");

            let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
            for p in &params {
                count_q = count_q.bind(value_to_string(p));
            }
            count_q
                .fetch_one(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("count query failed: {e}")))?
        };

        let order_sql = build_order_by(query.sort.as_deref(), ct);

        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, query.max_page_size.max(1));
        let offset = (page - 1) * page_size;

        let data_sql = format!(
            "SELECT {select_cols} FROM {table}{where_sql}{order_sql} LIMIT {page_size} OFFSET {offset}"
        );

        let rows = {
            let mut data_q = sqlx::query(&data_sql);
            for p in &params {
                data_q = data_q.bind(value_to_string(p));
            }
            data_q
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("data query failed: {e}")))?
        };

        let mut items: Vec<Value> = rows.iter().map(|row| row_to_value(row, &columns)).collect();

        if !ct.relation_fields().is_empty() {
            super::resolver::resolve_relations(
                &self.pool,
                ct,
                &mut items,
                query.include.as_deref(),
            )
            .await?;
        }

        Ok((items, count_row))
    }

    /// Find by ID
    pub async fn find_by_id(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        tenant_id: Option<&str>,
        include_private: bool,
    ) -> Result<Option<Value>, AppError> {
        let columns = ct.column_names(None, include_private);
        let select_cols = columns.join(", ");
        let tid = self.resolve_tenant(ct, tenant_id);

        let mut where_parts = vec![format!("{COL_DOCUMENT_ID} = {}", crate::db::dialect::ph(1))];
        let mut idx = 2;
        if tid.is_some() {
            where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(idx)));
            idx += 1;
        }

        let _ = idx;
        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE {}",
            ct.table,
            where_parts.join(" AND ")
        );

        let mut q = sqlx::query(&sql).bind(id);
        if let Some(ref tid) = tid {
            q = q.bind(tid);
        }

        let row = q
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        let mut result = row.map(|r| row_to_value(&r, &columns));

        if let Some(ref mut item) = result
            && !ct.relation_fields().is_empty()
        {
            super::resolver::resolve_relations(&self.pool, ct, std::slice::from_mut(item), None)
                .await?;
        }

        Ok(result)
    }

    /// Ensure the Single Type's unique record exists (auto-create if missing), returns the record
    pub async fn ensure_single(
        &self,
        ct: &ContentTypeSchema,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
        let tid = self.resolve_tenant(ct, tenant_id);

        let mut where_parts = Vec::new();
        if tid.is_some() {
            where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(1)));
        }

        let columns = ct.column_names(None, true);
        let select_cols = columns.join(", ");

        let sql = if where_parts.is_empty() {
            format!("SELECT {select_cols} FROM {} LIMIT 1", ct.table)
        } else {
            format!(
                "SELECT {select_cols} FROM {} WHERE {} LIMIT 1",
                ct.table,
                where_parts.join(" AND ")
            )
        };

        let mut q = sqlx::query(&sql);
        if let Some(ref tid) = tid {
            q = q.bind(tid);
        }

        let row = q
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        if let Some(r) = row {
            let mut result = row_to_value(&r, &columns);
            if !ct.relation_fields().is_empty() {
                super::resolver::resolve_relations(
                    &self.pool,
                    ct,
                    std::slice::from_mut(&mut result),
                    None,
                )
                .await?;
            }
            return Ok(result);
        }

        let save_ctx = SaveContext::default();
        self.create(
            ct,
            json!({
                "__single": true
            }),
            tenant_id,
            &save_ctx,
        )
        .await
    }

    /// Find by slug
    pub async fn find_by_slug(
        &self,
        ct: &ContentTypeSchema,
        slug: &str,
        _status: Option<&str>,
        tenant_id: Option<&str>,
        include_private: bool,
    ) -> Result<Option<Value>, AppError> {
        let columns = ct.column_names(None, include_private);
        let select_cols = columns.join(", ");
        let tid = self.resolve_tenant(ct, tenant_id);

        let mut where_parts = vec![format!("slug = {}", crate::db::dialect::ph(1))];

        for (column, condition) in ct.query_filters() {
            where_parts.push(format!("{} {}", column, condition));
        }
        if ct.query_filters().is_empty() && ct.is_soft_delete() {
            where_parts.push(format!("{} IS NULL", COL_DELETED_AT));
        }

        if tid.is_some() {
            where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(2)));
        }

        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE {}",
            ct.table,
            where_parts.join(" AND ")
        );

        let mut q = sqlx::query(&sql).bind(slug);
        if let Some(ref tid) = tid {
            q = q.bind(tid);
        }

        let row = q
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        let mut result = row.map(|r| row_to_value(&r, &columns));

        if let Some(ref mut item) = result
            && !ct.relation_fields().is_empty()
        {
            super::resolver::resolve_relations(&self.pool, ct, std::slice::from_mut(item), None)
                .await?;
        }

        Ok(result)
    }

    /// Create (with field validation, transaction-protected)
    pub async fn create(
        &self,
        ct: &ContentTypeSchema,
        mut data: Value,
        tenant_id: Option<&str>,
        _save_ctx: &SaveContext,
    ) -> Result<Value, AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("begin tx: {e}")))?;

        super::validation::validate_create_tx(&self.pool, ct, &data).await?;
        let document_id = crate::utils::id::new_document_id();

        let obj = data
            .as_object_mut()
            .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

        obj.remove(COL_ID);
        obj.insert(COL_DOCUMENT_ID.into(), json!(document_id));

        let tid = self.resolve_tenant(ct, tenant_id);

        let mut cols = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        if let Some(ref tid) = tid {
            cols.push(COL_TENANT_ID.to_string());
            placeholders.push(crate::db::dialect::ph(idx));
            idx += 1;
            values.push(tid.clone());
        }

        let mut fk_relation_map: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        let mut junction_fields: Vec<(String, String, String, String, String)> = Vec::new();

        for field in &ct.fields {
            if field.field_type != FieldType::Relation {
                continue;
            }
            let Some(ref rel) = field.relation else {
                continue;
            };
            match rel.relation_type {
                RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                    let fk = rel
                        .foreign_key
                        .clone()
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    fk_relation_map.insert(field.name.clone(), (fk, rel.target.clone()));
                }
                RelationType::ManyToMany | RelationType::ManyWay => {
                    let through = rel
                        .through
                        .clone()
                        .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));
                    let source_col = format!("{}_id", ct.singular);
                    let target_col = format!("{}_id", rel.target);
                    junction_fields.push((
                        field.name.clone(),
                        through,
                        rel.target.clone(),
                        source_col,
                        target_col,
                    ));
                }
                RelationType::OneToMany => {}
            }
        }

        let junction_field_names: Vec<&str> =
            junction_fields.iter().map(|(n, ..)| n.as_str()).collect();

        for (key, val) in obj.iter() {
            if key == COL_TENANT_ID || junction_field_names.contains(&key.as_str()) {
                continue;
            }
            if !crate::db::dialect::is_safe_identifier(key) {
                continue;
            }

            if let Some((fk_col, target_table)) = fk_relation_map.get(key) {
                let doc_id = value_to_string(val);
                if doc_id.is_empty() {
                    cols.push(fk_col.clone());
                    placeholders.push(crate::db::dialect::ph(idx));
                    idx += 1;
                    values.push(String::new());
                } else {
                    let int_id = resolve_document_id_to_int_id(&self.pool, target_table, &doc_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::BadRequest(format!(
                                "relation target '{doc_id}' not found in {target_table}"
                            ))
                        })?;
                    cols.push(fk_col.clone());
                    placeholders.push(crate::db::dialect::ph(idx));
                    idx += 1;
                    values.push(int_id.to_string());
                }
                continue;
            }

            cols.push(key.clone());
            placeholders.push(crate::db::dialect::ph(idx));
            idx += 1;
            values.push(value_to_string(val));
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            ct.table,
            cols.join(", "),
            placeholders.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for v in &values {
            query = query.bind(v);
        }

        query
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("insert failed: {e}")))?;

        let source_int_id: i64 = {
            let id_sql = format!(
                "SELECT {COL_ID} FROM {} WHERE {COL_DOCUMENT_ID} = {}",
                ct.table,
                crate::db::dialect::ph(1)
            );
            sqlx::query_scalar::<_, i64>(&id_sql)
                .bind(&document_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("fetch int_id failed: {e}")))?
        };

        for (field_name, through_table, target_table, source_col, target_col) in &junction_fields {
            if !crate::db::dialect::is_safe_identifier(through_table)
                || !crate::db::dialect::is_safe_identifier(source_col)
                || !crate::db::dialect::is_safe_identifier(target_col)
            {
                tracing::warn!(
                    "skipping junction with unsafe identifier: through={through_table}, source={source_col}, target={target_col}"
                );
                continue;
            }
            let Some(val) = obj.get(field_name) else {
                continue;
            };
            let doc_ids = extract_document_ids(val);
            if doc_ids.is_empty() {
                continue;
            }
            let int_ids = resolve_document_ids_batch(&self.pool, target_table, &doc_ids).await?;
            for target_int_id in int_ids {
                let jsql = format!(
                    "INSERT OR IGNORE INTO {through_table} ({source_col}, {target_col}) VALUES ({}, {})",
                    crate::db::dialect::ph(1),
                    crate::db::dialect::ph(2)
                );
                sqlx::query(&jsql)
                    .bind(source_int_id)
                    .bind(target_int_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("junction insert failed: {e}"))
                    })?;
            }
        }

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("commit failed: {e}")))?;

        let columns = ct.column_names(None, true);
        let select_cols = columns.join(", ");
        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE {COL_DOCUMENT_ID} = {}",
            ct.table,
            crate::db::dialect::ph(1)
        );
        let row = sqlx::query(&sql)
            .bind(&document_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        row.map(|r| row_to_value(&r, &columns))
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("created record not found")))
    }

    /// Update (with field validation, transaction-protected)
    ///
    /// When the content type has `versioning` enabled, automatically saves a snapshot
    /// of the current data to the `content_revisions` table before updating.
    pub async fn update(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        mut data: Value,
        tenant_id: Option<&str>,
        _save_ctx: &SaveContext,
    ) -> Result<Value, AppError> {
        if ct.declaration().snapshot_before_update
            && let Some(current) = self.find_by_id(ct, id, tenant_id, true).await?
        {
            let _ = crate::models::content_revision::create_revision(
                &self.pool,
                &ct.singular,
                id,
                &current,
                None,
            )
            .await;
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("begin tx: {e}")))?;

        super::validation::validate_update_tx(&self.pool, ct, id, &data).await?;

        let obj = data
            .as_object_mut()
            .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

        obj.remove(COL_ID);
        obj.remove(COL_DOCUMENT_ID);

        let tid = self.resolve_tenant(ct, tenant_id);

        let mut set_clauses = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        let mut fk_relation_map: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        let mut junction_fields: Vec<(String, String, String, String, String)> = Vec::new();

        for field in &ct.fields {
            if field.field_type != FieldType::Relation {
                continue;
            }
            let Some(ref rel) = field.relation else {
                continue;
            };
            match rel.relation_type {
                RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                    let fk = rel
                        .foreign_key
                        .clone()
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    fk_relation_map.insert(field.name.clone(), (fk, rel.target.clone()));
                }
                RelationType::ManyToMany | RelationType::ManyWay => {
                    let through = rel
                        .through
                        .clone()
                        .unwrap_or_else(|| format!("{}_{}", ct.table, rel.target));
                    let source_col = format!("{}_id", ct.singular);
                    let target_col = format!("{}_id", rel.target);
                    junction_fields.push((
                        field.name.clone(),
                        through,
                        rel.target.clone(),
                        source_col,
                        target_col,
                    ));
                }
                RelationType::OneToMany => {}
            }
        }

        let junction_field_names: Vec<&str> =
            junction_fields.iter().map(|(n, ..)| n.as_str()).collect();

        let decl = ct.declaration();

        let source_int_id: i64 = {
            let id_sql = format!(
                "SELECT {COL_ID} FROM {} WHERE {COL_DOCUMENT_ID} = {}",
                ct.table,
                crate::db::dialect::ph(1)
            );
            sqlx::query_scalar::<_, i64>(&id_sql)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("fetch int_id failed: {e}")))?
                .ok_or_else(|| AppError::not_found(&format!("{}/{}", ct.singular, id)))?
        };

        for (key, val) in obj.iter() {
            if ct.get_field(key).is_some() || ct.is_protocol_column(key) {
                if !crate::db::dialect::is_safe_identifier(key) {
                    continue;
                }
                if junction_field_names.contains(&key.as_str()) {
                    continue;
                }
                if let Some((fk_col, target_table)) = fk_relation_map.get(key) {
                    let doc_id = value_to_string(val);
                    if doc_id.is_empty() {
                        set_clauses.push(format!("{fk_col} = {}", crate::db::dialect::ph(idx)));
                        idx += 1;
                        values.push(String::new());
                    } else {
                        let int_id =
                            resolve_document_id_to_int_id(&self.pool, target_table, &doc_id)
                                .await?
                                .ok_or_else(|| {
                                    AppError::BadRequest(format!(
                                        "relation target '{doc_id}' not found in {target_table}"
                                    ))
                                })?;
                        set_clauses.push(format!("{fk_col} = {}", crate::db::dialect::ph(idx)));
                        idx += 1;
                        values.push(int_id.to_string());
                    }
                    continue;
                }
                set_clauses.push(format!("{key} = {}", crate::db::dialect::ph(idx)));
                idx += 1;
                values.push(value_to_string(val));
            }
        }

        if let Some(ref lock_col) = decl.lock_column {
            set_clauses.push(format!("{lock_col} = {lock_col} + 1"));
        }

        let has_junction_updates = junction_field_names.iter().any(|j| obj.contains_key(*j));
        if set_clauses.is_empty() && !has_junction_updates {
            return Err(AppError::BadRequest("no fields to update".into()));
        }

        if !set_clauses.is_empty() {
            let mut where_parts = vec![format!(
                "{COL_DOCUMENT_ID} = {}",
                crate::db::dialect::ph(idx)
            )];
            idx += 1;
            values.push(id.to_string());

            if let Some(ref tid) = tid {
                where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(idx)));
                values.push(tid.clone());
            }

            if let Some(ref lock_col) = decl.lock_column
                && let Some(current_version) = obj.get(lock_col).and_then(|v| v.as_i64())
            {
                where_parts.push(format!("{lock_col} = {}", crate::db::dialect::ph(idx)));
                values.push(current_version.to_string());
            }

            let sql = format!(
                "UPDATE {} SET {} WHERE {}",
                ct.table,
                set_clauses.join(", "),
                where_parts.join(" AND ")
            );

            let mut query = sqlx::query(&sql);
            for v in &values {
                query = query.bind(v);
            }

            let result = query
                .execute(&mut *tx)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("update failed: {e}")))?;

            if let Some(ref lock_col) = decl.lock_column
                && result.rows_affected() == 0
            {
                return Err(AppError::Conflict(format!(
                    "Record was modified by another user ({lock_col} conflict), please refresh and retry"
                )));
            }
        }

        for (field_name, through_table, target_table, source_col, target_col) in &junction_fields {
            if !crate::db::dialect::is_safe_identifier(through_table)
                || !crate::db::dialect::is_safe_identifier(source_col)
                || !crate::db::dialect::is_safe_identifier(target_col)
            {
                tracing::warn!(
                    "skipping junction with unsafe identifier: through={through_table}, source={source_col}, target={target_col}"
                );
                continue;
            }
            let Some(val) = obj.get(field_name) else {
                continue;
            };
            let del_sql = format!(
                "DELETE FROM {through_table} WHERE {source_col} = {}",
                crate::db::dialect::ph(1)
            );
            sqlx::query(&del_sql)
                .bind(source_int_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("junction delete failed: {e}")))?;

            let doc_ids = extract_document_ids(val);
            if doc_ids.is_empty() {
                continue;
            }
            let int_ids = resolve_document_ids_batch(&self.pool, target_table, &doc_ids).await?;
            for target_int_id in int_ids {
                let jsql = format!(
                    "INSERT OR IGNORE INTO {through_table} ({source_col}, {target_col}) VALUES ({}, {})",
                    crate::db::dialect::ph(1),
                    crate::db::dialect::ph(2)
                );
                sqlx::query(&jsql)
                    .bind(source_int_id)
                    .bind(target_int_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!("junction insert failed: {e}"))
                    })?;
            }
        }

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("commit failed: {e}")))?;

        self.find_by_id(ct, id, tenant_id, true)
            .await
            .transpose()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("updated record not found")))?
    }

    /// Delete
    ///
    /// Performs soft delete or hard delete based on the Protocol declaration strategy,
    /// and dispatches cleanup of related data via ProtocolRegistry.
    pub async fn delete(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        tenant_id: Option<&str>,
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<(), AppError> {
        let tid = self.resolve_tenant(ct, tenant_id);

        let mut idx = 1;
        let mut where_parts = vec![format!(
            "{COL_DOCUMENT_ID} = {}",
            crate::db::dialect::ph(idx)
        )];
        idx += 1;

        let mut values = vec![id.to_string()];

        if let Some(ref tid) = tid {
            where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(idx)));
            values.push(tid.clone());
        }

        if ct.is_soft_delete() {
            let decl = ct.declaration();
            let col = match &decl.delete_strategy {
                crate::protocols::DeleteStrategy::Soft { column } => column.clone(),
                _ => unreachable!(),
            };
            let now = crate::utils::tz::now_str();
            let sql = format!(
                "UPDATE {} SET {} = {} WHERE {}",
                ct.table,
                col,
                crate::db::dialect::ph(idx),
                where_parts.join(" AND ")
            );
            let mut query = sqlx::query(&sql);
            query = query.bind(now);
            for v in &values {
                query = query.bind(v);
            }
            query
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("delete failed: {e}")))?;
        } else {
            let sql = format!(
                "DELETE FROM {} WHERE {}",
                ct.table,
                where_parts.join(" AND ")
            );
            let mut query = sqlx::query(&sql);
            for v in &values {
                query = query.bind(v);
            }
            query
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("delete failed: {e}")))?;
        }

        let protocol_names: Vec<String> =
            ct.implements.iter().map(|p| p.name().to_string()).collect();
        let _ = protocol_registry
            .dispatch_after_delete(&protocol_names, &self.pool, &ct.singular, id)
            .await;

        Ok(())
    }

    pub async fn soft_delete(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        deleted_at: &str,
        deleted_by: Option<i64>,
        tenant_id: Option<&str>,
    ) -> Result<(), AppError> {
        let tid = self.resolve_tenant(ct, tenant_id);

        let mut idx = 1;
        let mut set_parts = vec![format!(
            "{} = {}",
            COL_DELETED_AT,
            crate::db::dialect::ph(idx)
        )];
        idx += 1;

        if deleted_by.is_some() {
            set_parts.push(format!(
                "{} = {}",
                COL_DELETED_BY,
                crate::db::dialect::ph(idx)
            ));
            idx += 1;
        }

        let mut where_parts = vec![format!(
            "{COL_DOCUMENT_ID} = {}",
            crate::db::dialect::ph(idx)
        )];
        idx += 1;

        if tid.is_some() {
            where_parts.push(format!("{COL_TENANT_ID} = {}", crate::db::dialect::ph(idx)));
        }

        let sql = format!(
            "UPDATE {} SET {} WHERE {}",
            ct.table,
            set_parts.join(", "),
            where_parts.join(" AND ")
        );

        let mut query = sqlx::query(&sql);
        query = query.bind(deleted_at);
        if let Some(by) = deleted_by {
            query = query.bind(by);
        }
        query = query.bind(id);
        if let Some(ref tid) = tid {
            query = query.bind(tid.as_str());
        }

        query
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("soft_delete failed: {e}")))?;

        Ok(())
    }

    /// Execute migration (create table + incremental column sync)
    ///
    /// - Table does not exist → `CREATE TABLE`
    /// - Table exists → compare schema with existing columns, `ALTER TABLE ADD COLUMN` for missing columns
    /// - Does not delete columns or modify column types (consistent with Strapi `forceMigration` policy)
    pub async fn migrate(
        &self,
        ct: &ContentTypeSchema,
        protocol_registry: &ProtocolRegistry,
    ) -> Result<(), AppError> {
        let names: Vec<String> = ct.implements.iter().map(|p| p.name().to_string()).collect();
        let protocol_columns = protocol_registry.columns_for(&names);
        let existing_columns = self.fetch_columns(&ct.table).await?;

        if existing_columns.is_empty() {
            let create_sql = super::migration::generate_create_table(ct, &protocol_columns);

            sqlx::query(&create_sql)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("CREATE TABLE {} failed: {}", ct.table, e))
                })?;

            tracing::info!("created table: {}", ct.table);
        } else {
            let alter_stmts =
                super::migration::generate_alter_table(ct, &existing_columns, &protocol_columns);
            if alter_stmts.is_empty() {
                tracing::debug!("table {} schema is up-to-date", ct.table);
            } else {
                for sql in &alter_stmts {
                    tracing::info!("syncing column: {}", sql);
                    sqlx::query(sql).execute(&self.pool).await.map_err(|e| {
                        AppError::Internal(anyhow::anyhow!(
                            "ALTER TABLE {} failed: {}",
                            ct.table,
                            e
                        ))
                    })?;
                }
                tracing::info!(
                    "synced {} column(s) for table {}",
                    alter_stmts.len(),
                    ct.table
                );
            }
        }

        for junction_sql in super::migration::generate_junction_tables(ct) {
            sqlx::query(&junction_sql)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("CREATE junction table failed: {e}"))
                })?;
        }

        for index_sql in super::migration::generate_indexes(ct) {
            if let Err(e) = sqlx::query(&index_sql).execute(&self.pool).await {
                tracing::warn!("index creation skipped: {}", e);
            }
        }

        tracing::info!("migrated content type: {} (table={})", ct.name, ct.table);
        Ok(())
    }

    /// Query existing column names of a table
    async fn fetch_columns(&self, table: &str) -> Result<Vec<String>, AppError> {
        let (sql, col_index): (String, usize) = fetch_columns_sql(table);

        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        let mut columns = Vec::new();
        for row in &rows {
            let col_name: String = row.try_get(col_index).unwrap_or_default();
            if !col_name.is_empty() {
                columns.push(col_name);
            }
        }

        Ok(columns)
    }
}

/// Build SELECT column name list (replaces json_object, for direct SELECT col1, col2, ...)
pub fn build_column_names(
    ct: &ContentTypeSchema,
    requested: Option<&[String]>,
    include_private: bool,
) -> Vec<String> {
    let mut cols = Vec::new();
    cols.push(COL_ID.into());
    cols.push(COL_DOCUMENT_ID.into());

    for field in &ct.fields {
        if !include_private && field.private {
            continue;
        }

        if let Some(req) = requested
            && !req.contains(&field.name)
        {
            continue;
        }

        if field.field_type == FieldType::Relation {
            match field.relation.as_ref().map(|r| &r.relation_type) {
                Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) => {
                    let fk = field
                        .relation
                        .as_ref()
                        .and_then(|r| r.foreign_key.clone())
                        .unwrap_or_else(|| format!("{}_id", field.name));
                    cols.push(fk);
                }
                Some(RelationType::ManyToMany | RelationType::OneToMany) => {}
                _ => {}
            }
            continue;
        }

        cols.push(field.name.clone());
    }

    for col in ct.protocol_column_names() {
        cols.push(col.to_string());
    }

    cols
}

/// Extract values column-by-column from an sqlx Row, building a serde_json::Value
///
/// SQLite stores all values as TEXT, so it tries to parse as bool/int/f64 first,
/// falling back to the raw string.
pub(crate) fn row_to_value(row: &DbRow, columns: &[String]) -> Value {
    let mut map = serde_json::Map::with_capacity(columns.len());
    for col in columns {
        let val = cell_to_json(row, col.as_str());
        map.insert(col.clone(), val);
    }
    Value::Object(map)
}

/// Convert a single SQLite cell to a JSON Value
///
/// Try order: i64 → f64 → bool → String → Null
///
/// Note: bool is placed after i64 because SQLite does not distinguish bool from int;
/// non-0/1 integers would be misidentified as true by bool.
fn cell_to_json(row: &DbRow, col: &str) -> Value {
    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(col) {
        return json!(v);
    }
    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(col) {
        return json!(v);
    }
    if let Ok(Some(v)) = row.try_get::<Option<bool>, _>(col) {
        return json!(v);
    }
    if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col) {
        if s.is_empty() {
            return Value::Null;
        }
        return json!(s);
    }
    Value::Null
}

fn build_order_by(sort: Option<&str>, ct: &ContentTypeSchema) -> String {
    let default = if let Some((col, dir)) = &ct.declaration().default_sort {
        let d = match dir {
            crate::protocols::SortDir::Asc => "asc",
            crate::protocols::SortDir::Desc => "desc",
        };
        format!("{col}:{d}")
    } else {
        String::new()
    };

    let sort_str = match sort {
        Some(s) if !s.is_empty() => s,
        _ => &default,
    };
    let mut parts = Vec::new();

    let valid_sort_columns = ct.column_names(None, true);

    for segment in sort_str.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let (col, dir) = if let Some((c, d)) = segment.split_once(':') {
            (c.trim(), d.trim())
        } else {
            (segment, "")
        };
        if !valid_sort_columns
            .iter()
            .any(|v| v.eq_ignore_ascii_case(col))
            || !crate::db::dialect::is_safe_identifier(col)
        {
            continue;
        }
        let dir = if dir.eq_ignore_ascii_case("asc") {
            "ASC"
        } else {
            "DESC"
        };
        parts.push(format!("{col} {dir}"));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "1".into()
            } else {
                "0".into()
            }
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn extract_document_ids(val: &Value) -> Vec<String> {
    match val {
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) async fn resolve_document_id_to_int_id(
    pool: &Pool,
    target_table: &str,
    document_id: &str,
) -> Result<Option<i64>, AppError> {
    if document_id.is_empty() {
        return Ok(None);
    }
    if !crate::db::dialect::is_safe_identifier(target_table) {
        return Err(AppError::BadRequest(format!(
            "invalid target table: {target_table}"
        )));
    }
    let sql = format!(
        "SELECT {COL_ID} FROM {target_table} WHERE {COL_DOCUMENT_ID} = {}",
        crate::db::dialect::ph(1)
    );
    let result = sqlx::query_scalar::<_, i64>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("resolve document_id failed: {e}")))?;
    Ok(result)
}

async fn resolve_document_ids_batch(
    pool: &Pool,
    target_table: &str,
    document_ids: &[String],
) -> Result<Vec<i64>, AppError> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    if !crate::db::dialect::is_safe_identifier(target_table) {
        return Err(AppError::BadRequest(format!(
            "invalid target table: {target_table}"
        )));
    }
    let placeholders: Vec<String> = (1..=document_ids.len())
        .map(crate::db::dialect::ph)
        .collect();
    let sql = format!(
        "SELECT {COL_ID}, {COL_DOCUMENT_ID} FROM {target_table} WHERE {COL_DOCUMENT_ID} IN ({})",
        placeholders.join(", ")
    );
    let mut q = sqlx::query(&sql);
    for id in document_ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| {
        AppError::Internal(anyhow::anyhow!("batch resolve document_ids failed: {e}"))
    })?;
    let mut lookup: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for row in &rows {
        let int_id: i64 = row.try_get(COL_ID).unwrap_or(0);
        let doc_id: String = row.try_get(COL_DOCUMENT_ID).unwrap_or_default();
        if int_id > 0 && !doc_id.is_empty() {
            lookup.insert(doc_id, int_id);
        }
    }
    let mut result = Vec::new();
    for doc_id in document_ids {
        let int_id = lookup.remove(doc_id).ok_or_else(|| {
            AppError::BadRequest(format!(
                "relation target document_id '{doc_id}' not found in {target_table}"
            ))
        })?;
        result.push(int_id);
    }
    Ok(result)
}

/// Generate SQL and column index for querying table column names
///
/// - `SQLite` `PRAGMA table_info`: column name is in column 2 (index=1)
/// - PostgreSQL/MySQL `information_schema`: column name is in column 1 (index=0)
///
/// # Panics
///
/// Panics if the table name contains invalid characters (table names come from schema definitions
/// and should have been validated before this point).
pub(crate) fn fetch_columns_sql(table: &str) -> (String, usize) {
    assert!(
        crate::db::dialect::is_safe_identifier(table),
        "invalid table name: {table}"
    );
    #[cfg(feature = "db-sqlite")]
    {
        (format!("PRAGMA table_info({table})"), 1)
    }
    #[cfg(feature = "db-postgres")]
    {
        (
            format!(
                "SELECT column_name FROM information_schema.columns WHERE table_name = '{}'",
                table
            ),
            0,
        )
    }
    #[cfg(feature = "db-mysql")]
    {
        (
            format!(
                "SELECT column_name FROM information_schema.columns WHERE table_name = '{}'",
                table
            ),
            0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_protocol_registry() -> crate::protocols::ProtocolRegistry {
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(crate::protocols::timestampable::TimestampableProtocol);
        reg.register(crate::protocols::soft_deletable::SoftDeletableProtocol);
        reg.register(crate::protocols::versionable::VersionableProtocol);
        reg.register(crate::protocols::lockable::LockableProtocol);
        reg.register(crate::protocols::sortable::SortableProtocol);
        reg.register(crate::protocols::expirable::ExpirableProtocol);
        reg.register(crate::protocols::nestable::NestableProtocol);
        reg.register(crate::protocols::tenantable::TenantableProtocol);
        reg
    }

    #[test]
    fn build_column_names_basic() {
        let mut ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Tag"
singular = "tag"
plural = "tags"
table = "tags"
implements = ["ownable", "timestampable"]

[fields.name]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true
"#,
        )
        .unwrap();
        ct.cache_protocol_columns(&test_protocol_registry());

        let cols = build_column_names(&ct, None, false);
        assert!(cols.contains(&"id".to_string()));
        assert!(cols.contains(&"document_id".to_string()));
        assert!(cols.contains(&"name".to_string()));
        assert!(cols.contains(&"slug".to_string()));
        assert!(cols.contains(&"created_at".to_string()));
        assert!(cols.contains(&"updated_at".to_string()));
    }

    #[test]
    fn build_order_by_default() {
        let mut ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
implements = ["sortable"]

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        ct.cache_protocol_columns(&test_protocol_registry());

        let order = build_order_by(None, &ct);
        assert_eq!(order, " ORDER BY created_at DESC");
    }

    #[test]
    fn build_order_by_custom() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"
"#,
        )
        .unwrap();

        let order = build_order_by(Some("title:asc"), &ct);
        assert_eq!(order, " ORDER BY title ASC");
    }

    #[test]
    fn value_to_string_string() {
        assert_eq!(value_to_string(&json!("hello")), "hello");
    }

    #[test]
    fn value_to_string_number() {
        assert_eq!(value_to_string(&json!(42)), "42");
    }

    #[test]
    fn value_to_string_bool() {
        assert_eq!(value_to_string(&json!(true)), "1");
        assert_eq!(value_to_string(&json!(false)), "0");
    }

    #[test]
    fn value_to_string_null() {
        assert_eq!(value_to_string(&Value::Null), "");
    }

    #[test]
    fn extract_document_ids_string() {
        let ids = extract_document_ids(&json!("abc-123"));
        assert_eq!(ids, vec!["abc-123"]);
    }

    #[test]
    fn extract_document_ids_array() {
        let ids = extract_document_ids(&json!(["a", "b", "c"]));
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_document_ids_empty_string() {
        let ids = extract_document_ids(&json!(""));
        assert!(ids.is_empty());
    }

    #[test]
    fn extract_document_ids_non_string() {
        let ids = extract_document_ids(&json!(42));
        assert!(ids.is_empty());
    }

    #[test]
    fn extract_document_ids_array_filters_empty() {
        let ids = extract_document_ids(&json!(["a", "", "c"]));
        assert_eq!(ids, vec!["a", "c"]);
    }

    #[test]
    fn fetch_columns_sql_sqlite() {
        let (sql, idx) = fetch_columns_sql("my_table");
        assert!(sql.contains("my_table"));
        assert_eq!(idx, 1);
    }

    #[test]
    fn build_column_names_excludes_private() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.pub_field]
type = "text"

[fields.priv_field]
type = "text"
private = true
"#,
        )
        .unwrap();
        let cols = build_column_names(&ct, None, false);
        assert!(cols.contains(&"pub_field".to_string()));
        assert!(!cols.contains(&"priv_field".to_string()));
    }

    #[test]
    fn build_column_names_includes_private_when_requested() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.pub_field]
type = "text"

[fields.priv_field]
type = "text"
private = true
"#,
        )
        .unwrap();
        let cols = build_column_names(&ct, None, true);
        assert!(cols.contains(&"pub_field".to_string()));
        assert!(cols.contains(&"priv_field".to_string()));
    }

    #[test]
    fn build_column_names_requested_filter() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.a]
type = "text"

[fields.b]
type = "integer"
"#,
        )
        .unwrap();
        let cols = build_column_names(&ct, Some(&["a".to_string()]), true);
        assert!(cols.contains(&"a".to_string()));
        assert!(!cols.contains(&"b".to_string()));
    }

    #[test]
    fn build_column_names_m2o_uses_fk() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
"#,
        )
        .unwrap();
        let cols = build_column_names(&ct, None, true);
        assert!(cols.contains(&"author_id".to_string()));
        assert!(!cols.contains(&"author".to_string()));
    }

    #[test]
    fn build_order_by_invalid_column_ignored() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        let order = build_order_by(Some("nonexistent:asc"), &ct);
        assert!(order.is_empty());
    }

    #[test]
    fn build_order_by_multiple_columns() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ts"

[fields.a]
type = "text"

[fields.b]
type = "integer"
"#,
        )
        .unwrap();
        let order = build_order_by(Some("a:asc,b:desc"), &ct);
        assert!(order.contains("a ASC"));
        assert!(order.contains("b DESC"));
    }

    #[test]
    fn save_context_from_auth() {
        let auth = crate::middleware::auth::AuthUser::from_parts(
            Some("u1".to_string()),
            Some(42),
            crate::models::user::UserRole::Admin,
            Some("t1".to_string()),
        );
        let ctx = SaveContext::from_auth(&auth);
        assert_eq!(ctx.user_id, Some("u1".to_string()));
        assert_eq!(ctx.user_int_id, Some(42));
        assert_eq!(
            ctx.user_role,
            Some(crate::models::user::UserRole::Admin.to_string())
        );
        assert_eq!(ctx.tenant_id, Some("t1".to_string()));
    }
}
