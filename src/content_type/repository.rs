//! 泛型内容 Repository — 动态 SQL CRUD
//!
//! 为所有 content type 提供统一的 CRUD 操作，动态构建 SQL。
//! 使用 `crate::db::dialect::translate()` 支持多数据库方言。
//!
//! 查询结果通过 `Row::get()` 逐列提取，直接构建 `serde_json::Value`，
//! 避免了 `json_object()` 双重序列化的性能开销。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::constants::*;
use crate::db::Pool;
use crate::errors::app_error::AppError;
use crate::middleware::auth::AuthUser;
use crate::protocols::ProtocolRegistry;
use sqlx::Row;

/// 保存操作上下文（从 handler 层传递到 repository 层）
#[derive(Debug, Clone, Default)]
pub struct SaveContext {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub tenant_id: Option<String>,
}

impl SaveContext {
    pub fn from_auth(auth: &AuthUser) -> Self {
        Self {
            user_id: auth.user_id().map(|s| s.to_string()),
            user_role: auth.is_authenticated().then(|| auth.role().to_string()),
            tenant_id: auth.tenant_id().map(|s| s.to_string()),
        }
    }
}

/// 通用查询参数
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
    /// API Rule 编译产生的额外 WHERE 子句
    pub rule_where: Option<String>,
    /// API Rule 编译产生的额外参数
    pub rule_params: Vec<String>,
    /// 单页最大条数（由 handler 从 config 传入）
    pub max_page_size: i64,
    /// 是否包含 private 字段（admin API 设为 true）
    pub include_private: bool,
}

/// 泛型内容 Repository
pub struct ContentRepository {
    pub pool: Pool,
}

impl ContentRepository {
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    async fn resolve_tenant(&self, table: &str, tenant_id: Option<&str>) -> Option<String> {
        if crate::db::tenant::has_tenant_id(&self.pool, table).await {
            Some(crate::db::tenant::resolve_tenant(tenant_id).to_string())
        } else {
            None
        }
    }

    /// 分页查询
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
        let tid = self.resolve_tenant(table, query.tenant_id.as_deref()).await;
        if let Some(ref tid) = tid {
            where_clauses.push(format!("tenant_id = {}", placeholder(param_idx)));
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
            if matches_field || matches_fk {
                where_clauses.push(format!("{} = {}", key, placeholder(param_idx)));
                params.push(val.clone());
                param_idx += 1;
            }
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
            let count_sql = crate::db::dialect::translate(&count_sql);

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
        let data_sql = crate::db::dialect::translate(&data_sql);

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

    /// 按 ID 查找
    pub async fn find_by_id(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        tenant_id: Option<&str>,
        include_private: bool,
    ) -> Result<Option<Value>, AppError> {
        let columns = ct.column_names(None, include_private);
        let select_cols = columns.join(", ");
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut where_parts = vec![format!("id = {}", placeholder(1))];
        let mut idx = 2;
        if tid.is_some() {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            idx += 1;
        }

        let _ = idx;
        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE {}",
            ct.table,
            where_parts.join(" AND ")
        );
        let sql = crate::db::dialect::translate(&sql);

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

    /// 确保 Single Type 的唯一记录存在（不存在则自动创建），返回该记录
    pub async fn ensure_single(
        &self,
        ct: &ContentTypeSchema,
        tenant_id: Option<&str>,
    ) -> Result<Value, AppError> {
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut where_parts = Vec::new();
        if tid.is_some() {
            where_parts.push(format!("tenant_id = {}", placeholder(1)));
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
        let sql = crate::db::dialect::translate(&sql);

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

    /// 按 slug 查找
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
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut where_parts = vec![format!("slug = {}", placeholder(1))];

        for (column, condition) in ct.query_filters() {
            where_parts.push(format!("{} {}", column, condition));
        }
        if ct.query_filters().is_empty() && ct.is_soft_delete() {
            where_parts.push(format!("{} IS NULL", COL_DELETED_AT));
        }

        if tid.is_some() {
            where_parts.push(format!("tenant_id = {}", placeholder(2)));
        }

        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE {}",
            ct.table,
            where_parts.join(" AND ")
        );
        let sql = crate::db::dialect::translate(&sql);

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

    /// 创建（含字段校验，事务保护）
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
        let id = uuid::Uuid::now_v7().to_string();

        let obj = data
            .as_object_mut()
            .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

        obj.insert("id".into(), json!(id));

        if !ct.builtin && !ct.implements.is_empty() {
            let mut meta = serde_json::Map::new();
            let protocol_names: Vec<&str> = ct.implements.iter().map(|p| p.name()).collect();
            meta.insert("protocols".into(), json!(protocol_names));
            obj.insert(COL_META.into(), json!(meta));
        }

        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut cols = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        if let Some(ref tid) = tid {
            cols.push(COL_TENANT_ID.to_string());
            placeholders.push(placeholder(idx));
            idx += 1;
            values.push(tid.clone());
        }

        let relation_column_map: std::collections::HashMap<String, String> = ct
            .fields
            .iter()
            .filter(|f| f.field_type == super::schema::FieldType::Relation)
            .map(|f| {
                let fk = f
                    .relation
                    .as_ref()
                    .and_then(|r| r.foreign_key.clone())
                    .unwrap_or_else(|| format!("{}_id", f.name));
                (f.name.clone(), fk)
            })
            .collect();

        for (key, val) in obj.iter() {
            let col = relation_column_map
                .get(key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            cols.push(col);
            placeholders.push(placeholder(idx));
            idx += 1;
            values.push(value_to_string(val));
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            ct.table,
            cols.join(", "),
            placeholders.join(", ")
        );
        let sql = crate::db::dialect::translate(&sql);

        let mut query = sqlx::query(&sql);
        for v in &values {
            query = query.bind(v);
        }

        query
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("insert failed: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("commit failed: {e}")))?;

        let columns = ct.column_names(None, true);
        let select_cols = columns.join(", ");
        let sql = format!(
            "SELECT {select_cols} FROM {} WHERE id = {}",
            ct.table,
            placeholder(1)
        );
        let sql = crate::db::dialect::translate(&sql);
        let row = sqlx::query(&sql)
            .bind(&id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

        row.map(|r| row_to_value(&r, &columns))
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("created record not found")))
    }

    /// 更新（含字段校验，事务保护）
    ///
    /// 当 content type 启用 `versioning` 时，更新前自动保存当前数据快照到
    /// `content_revisions` 表。
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

        obj.remove("id");

        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut set_clauses = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        let relation_column_map: std::collections::HashMap<String, String> = ct
            .fields
            .iter()
            .filter(|f| f.field_type == super::schema::FieldType::Relation)
            .map(|f| {
                let fk = f
                    .relation
                    .as_ref()
                    .and_then(|r| r.foreign_key.clone())
                    .unwrap_or_else(|| format!("{}_id", f.name));
                (f.name.clone(), fk)
            })
            .collect();

        let decl = ct.declaration();

        for (key, val) in obj.iter() {
            if ct.get_field(key).is_some() || ct.is_protocol_column(key) {
                let col = relation_column_map
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| key.clone());
                set_clauses.push(format!("{col} = {}", placeholder(idx)));
                idx += 1;
                values.push(value_to_string(val));
            }
        }

        if let Some(ref lock_col) = decl.lock_column {
            set_clauses.push(format!("{lock_col} = {lock_col} + 1"));
        }

        if set_clauses.is_empty() {
            return Err(AppError::BadRequest("no fields to update".into()));
        }

        let mut where_parts = vec![format!("id = {}", placeholder(idx))];
        idx += 1;
        values.push(id.to_string());

        if let Some(ref tid) = tid {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            values.push(tid.clone());
        }

        if let Some(ref lock_col) = decl.lock_column
            && let Some(current_version) = data.get(lock_col).and_then(|v| v.as_i64())
        {
            where_parts.push(format!("{lock_col} = {}", placeholder(idx)));
            values.push(current_version.to_string());
        }

        let sql = format!(
            "UPDATE {} SET {} WHERE {}",
            ct.table,
            set_clauses.join(", "),
            where_parts.join(" AND ")
        );
        let sql = crate::db::dialect::translate(&sql);

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
                "记录已被他人修改（{lock_col} 冲突），请刷新后重试"
            )));
        }

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("commit failed: {e}")))?;

        self.find_by_id(ct, id, tenant_id, true)
            .await
            .transpose()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("updated record not found")))?
    }

    /// 删除
    ///
    /// 根据 Protocol 声明的策略执行软删除或硬删除，
    /// 并通过 ProtocolRegistry dispatch 清理关联数据。
    pub async fn delete(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        tenant_id: Option<&str>,
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<(), AppError> {
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut idx = 1;
        let mut where_parts = vec![format!("id = {}", placeholder(idx))];
        idx += 1;

        let mut values = vec![id.to_string()];

        if let Some(ref tid) = tid {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
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
                placeholder(idx),
                where_parts.join(" AND ")
            );
            let sql = crate::db::dialect::translate(&sql);
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
            let sql = crate::db::dialect::translate(&sql);
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
        deleted_by: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<(), AppError> {
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut idx = 1;
        let mut set_parts = vec![format!("{} = {}", COL_DELETED_AT, placeholder(idx))];
        let mut values: Vec<String> = vec![deleted_at.to_string()];
        idx += 1;

        if let Some(by) = deleted_by {
            set_parts.push(format!("{} = {}", COL_DELETED_BY, placeholder(idx)));
            values.push(by.to_string());
            idx += 1;
        }

        let mut where_parts = vec![format!("id = {}", placeholder(idx))];
        values.push(id.to_string());
        idx += 1;

        if let Some(ref tid) = tid {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            values.push(tid.clone());
        }

        let raw_sql = format!(
            "UPDATE {} SET {} WHERE {}",
            ct.table,
            set_parts.join(", "),
            where_parts.join(" AND ")
        );
        let sql = crate::db::dialect::translate(&raw_sql);

        let mut query = sqlx::query(&sql);
        for v in &values {
            query = query.bind(v);
        }

        query
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("soft_delete failed: {e}")))?;

        Ok(())
    }

    /// 执行 migration（建表 + 增量同步列）
    ///
    /// - 表不存在 → `CREATE TABLE`
    /// - 表已存在 → 对比 schema 与现有列，`ALTER TABLE ADD COLUMN` 补齐缺失列
    /// - 不删除列、不修改列类型（与 Strapi `forceMigration` 策略一致）
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
            let create_sql = crate::db::dialect::translate(&create_sql);

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
                    let sql = crate::db::dialect::translate(sql);
                    tracing::info!("syncing column: {}", sql);
                    sqlx::query(&sql).execute(&self.pool).await.map_err(|e| {
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
            let sql = crate::db::dialect::translate(&junction_sql);
            sqlx::query(&sql).execute(&self.pool).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!("CREATE junction table failed: {e}"))
            })?;
        }

        for index_sql in super::migration::generate_indexes(ct) {
            let sql = crate::db::dialect::translate(&index_sql);
            if let Err(e) = sqlx::query(&sql).execute(&self.pool).await {
                tracing::warn!("index creation skipped: {}", e);
            }
        }

        tracing::info!("migrated content type: {} (table={})", ct.name, ct.table);
        Ok(())
    }

    /// 查询表的现有列名
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

/// 构建 SELECT 列名列表（替代 json_object，用于直接 SELECT col1, col2, ...）
pub fn build_column_names(
    ct: &ContentTypeSchema,
    requested: Option<&[String]>,
    include_private: bool,
) -> Vec<String> {
    let mut cols = Vec::new();
    cols.push("id".into());

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
                Some(RelationType::ManyToOne | RelationType::OneToOne) => {
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
    if !ct.builtin {
        cols.push(COL_META.into());
    }

    cols
}

/// 从 sqlx Row 逐列提取值，构建 serde_json::Value
///
/// SQLite 将所有值存储为 TEXT，因此优先尝试解析为 bool/int/f64，
/// 回退到原始字符串。
pub fn row_to_value(row: &sqlx::sqlite::SqliteRow, columns: &[String]) -> Value {
    let mut map = serde_json::Map::with_capacity(columns.len());
    for col in columns {
        let val = cell_to_json(row, col.as_str());
        map.insert(col.clone(), val);
    }
    Value::Object(map)
}

/// 将单个 SQLite 单元格转为 JSON Value
///
/// 尝试顺序：i64 → f64 → bool → String → Null
///
/// 注意：bool 放在 i64 之后，因为 SQLite 不区分 bool 和 int，
/// 非 0/1 的整数会被 bool 误判为 true。
fn cell_to_json(row: &sqlx::sqlite::SqliteRow, col: &str) -> Value {
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

    for segment in sort_str.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((col, dir)) = segment.split_once(':') {
            let dir = if dir.eq_ignore_ascii_case("asc") {
                "ASC"
            } else {
                "DESC"
            };
            parts.push(format!("{col} {dir}"));
        } else {
            parts.push(format!("{segment} DESC"));
        }
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

fn placeholder(idx: usize) -> String {
    #[cfg(feature = "db-postgres")]
    {
        format!("${}", idx)
    }
    #[cfg(not(feature = "db-postgres"))]
    {
        let _ = idx;
        "?".to_string()
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

/// 生成查询表列名的 SQL 和列名所在的列索引
///
/// - `SQLite` `PRAGMA table_info`: 列名在第 2 列 (index=1)
/// - PostgreSQL/MySQL `information_schema`: 列名在第 1 列 (index=0)
fn fetch_columns_sql(table: &str) -> (String, usize) {
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
        reg.register(crate::protocols::cacheable::CacheableProtocol);
        reg.register(crate::protocols::lockable::LockableProtocol);
        reg.register(crate::protocols::sortable::SortableProtocol);
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
        assert_eq!(order, " ORDER BY created_by DESC");
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
}
