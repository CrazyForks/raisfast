//! 泛型内容 Repository — 动态 SQL CRUD
//!
//! 为所有 content type 提供统一的 CRUD 操作，动态构建 SQL。
//! 使用 `crate::db::dialect::translate()` 支持多数据库方言。
//!
//! 查询结果通过 `Row::get()` 逐列提取，直接构建 `serde_json::Value`，
//! 避免了 `json_object()` 双重序列化的性能开销。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::schema::{AutoFillSource, ContentTypeSchema, FieldType, RelationType};
use crate::db::Pool;
use crate::errors::app_error::AppError;
use crate::middleware::auth::OptionalAuth;
use sqlx::Row;

/// 保存操作上下文（从 handler 层传递到 repository 层）
///
/// 携带当前请求的认证信息，供 auto_fill 机制注入字段值。
#[derive(Debug, Clone, Default)]
pub struct SaveContext {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub tenant_id: Option<String>,
}

impl SaveContext {
    pub fn from_optional_auth(auth: &OptionalAuth) -> Self {
        Self {
            user_id: auth.0.as_ref().map(|a| a.user_id.clone()),
            user_role: auth.0.as_ref().map(|a| a.role.clone()),
            tenant_id: auth.0.as_ref().map(|a| a.tenant_id.clone()),
        }
    }

    fn resolve_auto_fill(&self, source: &AutoFillSource) -> Option<Value> {
        match source {
            AutoFillSource::UserId => self.user_id.as_ref().map(|id| json!(id)),
            AutoFillSource::UserRole => self.user_role.as_ref().map(|r| json!(r)),
            AutoFillSource::CurrentTenantId => self.tenant_id.as_ref().map(|t| json!(t)),
            AutoFillSource::CurrentTimestamp => Some(json!(crate::utils::tz::now_str())),
        }
    }

    fn inject_auto_fill(&self, ct: &ContentTypeSchema, obj: &mut serde_json::Map<String, Value>) {
        for field in ct.auto_fill_fields() {
            if let Some(ref source) = field.auto_fill
                && let Some(value) = self.resolve_auto_fill(source)
            {
                match field.field_type {
                    FieldType::Relation => {
                        if let Some(ref rel) = field.relation {
                            let fk = rel
                                .foreign_key
                                .clone()
                                .unwrap_or_else(|| format!("{}_id", field.name));
                            obj.insert(fk, value);
                        }
                    }
                    _ => {
                        obj.insert(field.name.clone(), value);
                    }
                }
            }
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
        let columns = ct.column_names(query.fields.as_deref());
        let select_cols = columns.join(", ");
        let table = &ct.table;

        let mut where_clauses = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut param_idx = 1;

        let tid = self.resolve_tenant(table, query.tenant_id.as_deref()).await;
        if let Some(ref tid) = tid {
            where_clauses.push(format!("tenant_id = {}", placeholder(param_idx)));
            params.push(json!(tid));
            param_idx += 1;
        }

        if let Some(ref status) = query.status
            && ct.draft_publish
        {
            where_clauses.push(format!("status = {}", placeholder(param_idx)));
            params.push(json!(status));
            param_idx += 1;
        }

        for (key, val) in &query.filters {
            if let Some(_field) = ct.get_field(key) {
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
        let page_size = query.page_size.clamp(1, 100);
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
    ) -> Result<Option<Value>, AppError> {
        let columns = ct.column_names(None);
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

    /// 按 slug 查找
    pub async fn find_by_slug(
        &self,
        ct: &ContentTypeSchema,
        slug: &str,
        status: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<Option<Value>, AppError> {
        let columns = ct.column_names(None);
        let select_cols = columns.join(", ");
        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut where_parts = vec![format!("slug = {}", placeholder(1))];
        let mut idx = 2;

        if tid.is_some() {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            idx += 1;
        }

        if status.is_some() && ct.draft_publish {
            where_parts.push(format!("status = {}", placeholder(idx)));
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
        if let Some(s) = status
            && ct.draft_publish
        {
            q = q.bind(s);
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
        save_ctx: &SaveContext,
    ) -> Result<Value, AppError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("begin tx: {e}")))?;

        super::validation::validate_create_tx(&self.pool, ct, &data).await?;
        let id = uuid::Uuid::now_v7().to_string();
        let now = crate::utils::tz::now_str();

        let obj = data
            .as_object_mut()
            .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

        obj.insert("id".into(), json!(id));

        if ct.timestamps {
            obj.insert("created_at".into(), json!(now.clone()));
            obj.insert("updated_at".into(), json!(now));
        }

        if ct.draft_publish && obj.get("status").is_none() {
            obj.insert("status".to_string(), json!("draft"));
        }

        save_ctx.inject_auto_fill(ct, obj);

        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut cols = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        if let Some(ref tid) = tid {
            cols.push("tenant_id".to_string());
            placeholders.push(placeholder(idx));
            idx += 1;
            values.push(tid.clone());
        }

        for (key, val) in obj.iter() {
            cols.push(key.clone());
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

        self.find_by_id(ct, &id, tenant_id)
            .await
            .transpose()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("created record not found")))?
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
        save_ctx: &SaveContext,
    ) -> Result<Value, AppError> {
        if ct.versioning
            && let Some(current) = self.find_by_id(ct, id, tenant_id).await?
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
        let now = crate::utils::tz::now_str();

        let obj = data
            .as_object_mut()
            .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

        obj.remove("id");
        obj.remove("created_at");
        obj.remove("updated_at");

        save_ctx.inject_auto_fill(ct, obj);

        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut set_clauses = Vec::new();
        let mut values: Vec<String> = Vec::new();
        let mut idx = 1;

        for (key, val) in obj.iter() {
            if ct.get_field(key).is_some() || key == "status" || key == "published_at" {
                set_clauses.push(format!("{} = {}", key, placeholder(idx)));
                idx += 1;
                values.push(value_to_string(val));
            }
        }

        if set_clauses.is_empty() {
            return Err(AppError::BadRequest("no fields to update".into()));
        }

        if ct.timestamps {
            set_clauses.push(format!("updated_at = {}", placeholder(idx)));
            idx += 1;
            values.push(now);
        }

        let mut where_parts = vec![format!("id = {}", placeholder(idx))];
        idx += 1;
        values.push(id.to_string());

        if let Some(ref tid) = tid {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            values.push(tid.clone());
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

        query
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("update failed: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("commit failed: {e}")))?;

        self.find_by_id(ct, id, tenant_id)
            .await
            .transpose()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("updated record not found")))?
    }

    /// 删除
    ///
    /// 同时清理该记录在 `content_revisions` 表中的所有版本历史。
    pub async fn delete(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), AppError> {
        if ct.versioning {
            let _ = crate::models::content_revision::delete_revisions(&self.pool, &ct.singular, id)
                .await;
        }

        let tid = self.resolve_tenant(&ct.table, tenant_id).await;

        let mut idx = 1;
        let mut where_parts = vec![format!("id = {}", placeholder(idx))];
        idx += 1;

        let mut values = vec![id.to_string()];

        if let Some(ref tid) = tid {
            where_parts.push(format!("tenant_id = {}", placeholder(idx)));
            values.push(tid.clone());
        }

        let sql = if ct.soft_delete {
            let now = crate::utils::tz::now_str();
            format!(
                "UPDATE {} SET deleted_at = '{}' WHERE {}",
                ct.table,
                now,
                where_parts.join(" AND ")
            )
        } else {
            format!(
                "DELETE FROM {} WHERE {}",
                ct.table,
                where_parts.join(" AND ")
            )
        };
        let sql = crate::db::dialect::translate(&sql);

        let mut query = sqlx::query(&sql);
        for v in &values {
            query = query.bind(v);
        }

        query
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("delete failed: {e}")))?;

        Ok(())
    }

    /// 执行 migration（建表 + 增量同步列）
    ///
    /// - 表不存在 → `CREATE TABLE`
    /// - 表已存在 → 对比 schema 与现有列，`ALTER TABLE ADD COLUMN` 补齐缺失列
    /// - 不删除列、不修改列类型（与 Strapi `forceMigration` 策略一致）
    pub async fn migrate(&self, ct: &ContentTypeSchema) -> Result<(), AppError> {
        let existing_columns = self.fetch_columns(&ct.table).await?;

        if existing_columns.is_empty() {
            let create_sql = super::migration::generate_create_table(ct);
            let create_sql = crate::db::dialect::translate(&create_sql);

            sqlx::query(&create_sql)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("CREATE TABLE {} failed: {}", ct.table, e))
                })?;

            tracing::info!("created table: {}", ct.table);
        } else {
            let alter_stmts = super::migration::generate_alter_table(ct, &existing_columns);
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
pub fn build_column_names(ct: &ContentTypeSchema, requested: Option<&[String]>) -> Vec<String> {
    let mut cols = Vec::new();
    cols.push("id".into());

    for field in &ct.fields {
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

        if let Some(req) = requested {
            if req.contains(&field.name) {
                cols.push(field.name.clone());
            }
        } else {
            cols.push(field.name.clone());
        }
    }

    if ct.draft_publish {
        cols.push("status".into());
        cols.push("published_at".into());
    }
    if ct.timestamps {
        cols.push("created_at".into());
        cols.push("updated_at".into());
    }
    if ct.soft_delete {
        cols.push("deleted_at".into());
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
    let default = ct
        .list_view
        .as_ref()
        .map_or_else(|| "created_at:desc".into(), |lv| lv.default_sort.clone());

    let sort_str = sort.unwrap_or(&default);
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

    #[test]
    fn build_column_names_basic() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Tag"
singular = "tag"
plural = "tags"
table = "tags"
timestamps = true

[fields.name]
type = "text"
required = true

[fields.slug]
type = "uid"
unique = true
"#,
        )
        .unwrap();

        let cols = build_column_names(&ct, None);
        assert!(cols.contains(&"id".to_string()));
        assert!(cols.contains(&"name".to_string()));
        assert!(cols.contains(&"slug".to_string()));
        assert!(cols.contains(&"created_at".to_string()));
        assert!(cols.contains(&"updated_at".to_string()));
    }

    #[test]
    fn build_order_by_default() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"

[list_view]
default_sort = "is_pinned:desc,created_at:desc"
"#,
        )
        .unwrap();

        let order = build_order_by(None, &ct);
        assert_eq!(order, " ORDER BY is_pinned DESC, created_at DESC");
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
