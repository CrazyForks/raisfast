//! Site options service.
//!
//! Multi-tenant aware option store backed by an LRU cache (moka).
//!
//! - Global options (`tenant_id = None`) are preloaded at startup.
//! - Per-tenant options are loaded lazily: on first cache miss, **all** options
//!   for that tenant are fetched in one query and cached. Subsequent reads hit cache.
//! - Cold entries are evicted automatically by moka's TTL + size policy.
//! - Writes invalidate the cache entry; next read reloads from DB.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppError;
use crate::models::options::OptionRow;

const CACHE_MAX_ENTRIES: usize = 10_000;
const CACHE_TTL_SECS: u64 = 600;
const CACHE_IDLE_SECS: u64 = 300;

fn parse_value(value_str: &str) -> Value {
    serde_json::from_str::<Value>(value_str).unwrap_or(Value::String(value_str.to_string()))
}

fn cache_key(tenant_id: Option<&str>, option_key: &str) -> String {
    match tenant_id {
        Some(tid) => format!("{tid}:{option_key}"),
        None => format!("GLOBAL:{option_key}"),
    }
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptionGroup {
    pub option_key: String,
    pub label: String,
    pub options: Vec<OptionEntry>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptionEntry {
    pub option_key: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub value: Value,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "export-types", ts(rename = "type"))]
    pub type_: String,
    pub label: String,
    pub description: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub validation: Option<Value>,
    pub is_public: bool,
    tenant_id: Option<String>,
}

impl From<&OptionRow> for OptionEntry {
    fn from(row: &OptionRow) -> Self {
        Self {
            option_key: row.option_key.clone(),
            value: parse_value(&row.value),
            type_: row.type_.to_string(),
            label: row.label.clone(),
            description: row.description.clone(),
            validation: row
                .validation
                .as_ref()
                .and_then(|v| serde_json::from_str::<Value>(v).ok()),
            is_public: row.is_public,
            tenant_id: row.tenant_id.clone(),
        }
    }
}

pub struct OptionsService {
    cache: moka::sync::Cache<String, OptionEntry>,
    warmed: dashmap::DashSet<String>,
    pool: Arc<crate::db::Pool>,
}

impl OptionsService {
    pub async fn new(pool: Arc<crate::db::Pool>, _builtin_tenantable: bool) -> Self {
        let cache = moka::sync::Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES as u64)
            .time_to_idle(std::time::Duration::from_secs(CACHE_IDLE_SECS))
            .time_to_live(std::time::Duration::from_secs(CACHE_TTL_SECS))
            .build();

        let service = Self {
            cache,
            warmed: dashmap::DashSet::new(),
            pool,
        };
        if let Err(e) = service.load_default_tenant_autoload().await {
            tracing::error!("failed to autoload global options: {}", e);
        }
        service
    }

    async fn load_default_tenant_autoload(&self) -> Result<(), AppError> {
        let rows = crate::models::options::find_autoload(&self.pool).await?;
        let default_tenant = crate::constants::DEFAULT_TENANT;
        let mut count = 0usize;
        for row in &rows {
            if row.tenant_id.as_deref() != Some(default_tenant) {
                continue;
            }
            let entry = OptionEntry::from(row);
            let key = cache_key(Some(default_tenant), &row.option_key);
            self.cache.insert(key, entry);
            self.warmed.insert(format!("__warmed__{default_tenant}"));
            count += 1;
        }
        tracing::info!("loaded {count} autoload option(s) for default tenant");
        Ok(())
    }

    /// On first miss for a tenant, load ALL their options in one query.
    async fn warm_tenant(&self, tenant_id: &str) {
        let warm_key = format!("__warmed__{tenant_id}");
        if self.warmed.contains(&warm_key) {
            return;
        }
        let Ok(rows) = crate::models::options::find_all(&self.pool, Some(tenant_id)).await else {
            return;
        };
        for row in &rows {
            let entry = OptionEntry::from(row);
            let key = cache_key(Some(tenant_id), &row.option_key);
            self.cache.insert(key, entry);
        }
        self.warmed.insert(warm_key);
    }

    pub async fn get(&self, tenant_id: Option<&str>, key: &str) -> Option<Value> {
        let ck = cache_key(tenant_id, key);
        if let Some(entry) = self.cache.get(&ck) {
            return Some(entry.value.clone());
        }
        if let Some(tid) = tenant_id {
            self.warm_tenant(tid).await;
            if let Some(entry) = self.cache.get(&ck) {
                return Some(entry.value.clone());
            }
        }
        let row = crate::models::options::find_by_key(&self.pool, key, tenant_id)
            .await
            .ok()
            .flatten()?;
        let entry = OptionEntry::from(&row);
        let value = entry.value.clone();
        self.cache.insert(ck, entry);
        Some(value)
    }

    pub async fn get_entry(&self, tenant_id: Option<&str>, key: &str) -> Option<OptionEntry> {
        let ck = cache_key(tenant_id, key);
        if let Some(entry) = self.cache.get(&ck) {
            return Some(entry.clone());
        }
        if let Some(tid) = tenant_id {
            self.warm_tenant(tid).await;
            if let Some(entry) = self.cache.get(&ck) {
                return Some(entry.clone());
            }
        }
        let row = crate::models::options::find_by_key(&self.pool, key, tenant_id)
            .await
            .ok()
            .flatten()?;
        let entry = OptionEntry::from(&row);
        self.cache.insert(ck, entry.clone());
        Some(entry)
    }

    pub async fn set(
        &self,
        tenant_id: Option<&str>,
        key: &str,
        value: Value,
    ) -> Result<(), AppError> {
        let value_str = serde_json::to_string(&value).map_err(|e| AppError::Internal(e.into()))?;
        crate::models::options::upsert_value(&self.pool, key, &value_str, tenant_id).await?;
        let ck = cache_key(tenant_id, key);
        self.cache.invalidate(&ck);
        Ok(())
    }

    pub async fn set_batch(
        &self,
        tenant_id: Option<&str>,
        pairs: HashMap<String, Value>,
    ) -> Result<(), AppError> {
        for (key, value) in &pairs {
            let value_str =
                serde_json::to_string(value).map_err(|e| AppError::Internal(e.into()))?;
            crate::models::options::upsert_value(&self.pool, key, &value_str, tenant_id).await?;
        }
        for key in pairs.keys() {
            let ck = cache_key(tenant_id, key);
            self.cache.invalidate(&ck);
        }
        Ok(())
    }

    pub async fn delete(&self, tenant_id: Option<&str>, key: &str) -> Result<(), AppError> {
        crate::models::options::delete_by_key(&self.pool, key, tenant_id).await?;
        let ck = cache_key(tenant_id, key);
        self.cache.invalidate(&ck);
        Ok(())
    }

    pub async fn get_grouped(&self, tenant_id: Option<&str>) -> Result<Vec<OptionGroup>, AppError> {
        let rows = crate::models::options::find_all(&self.pool, tenant_id).await?;
        let mut group_map: HashMap<String, Vec<OptionEntry>> = HashMap::new();
        let mut group_order: Vec<String> = Vec::new();

        for row in &rows {
            let entry = OptionEntry::from(row);
            if !group_map.contains_key(&row.group_name) {
                group_order.push(row.group_name.clone());
            }
            group_map
                .entry(row.group_name.clone())
                .or_default()
                .push(entry);
        }

        Ok(group_order
            .into_iter()
            .map(|key| OptionGroup {
                label: key.clone(),
                option_key: key.clone(),
                options: group_map.remove(&key).unwrap_or_default(),
            })
            .collect())
    }

    pub async fn get_public(&self, tenant_id: Option<&str>) -> HashMap<String, Value> {
        let rows = crate::models::options::find_all(&self.pool, tenant_id)
            .await
            .unwrap_or_default();
        rows.iter()
            .filter(|r| r.is_public)
            .map(|r| (r.option_key.clone(), parse_value(&r.value)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_global() {
        assert_eq!(cache_key(None, "foo"), "GLOBAL:foo");
    }

    #[test]
    fn cache_key_tenant() {
        assert_eq!(cache_key(Some("t1"), "foo"), "t1:foo");
    }

    #[test]
    fn parse_value_handles_json_string() {
        assert_eq!(
            parse_value(r#""hello""#),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn parse_value_handles_json_number() {
        assert_eq!(parse_value("42"), Value::Number(42.into()));
    }

    #[test]
    fn parse_value_handles_json_bool() {
        assert_eq!(parse_value("true"), Value::Bool(true));
    }

    #[test]
    fn parse_value_falls_back_to_string() {
        assert_eq!(
            parse_value("plain text"),
            Value::String("plain text".to_string())
        );
    }
}
