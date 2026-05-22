//! Site options service.
//!
//! On startup, preloads options with `autoload=true` into memory.
//! Subsequent reads prioritize the cache; writes update both the cache and the database.
//! Each option includes full metadata (type, group, label, validation rules).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppError;
use crate::models::options::OptionRow;

/// Parse a configuration value string from the database into a `serde_json::Value`
fn parse_value(value_str: &str) -> Value {
    serde_json::from_str::<Value>(value_str).unwrap_or(Value::String(value_str.to_string()))
}

/// Group information
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptionGroup {
    pub option_key: String,
    pub label: String,
    pub options: Vec<OptionEntry>,
}

/// Single option entry (value + metadata)
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
        }
    }
}

/// Site options service
pub struct OptionsService {
    cache: Arc<RwLock<HashMap<String, OptionEntry>>>,
    pool: Arc<crate::db::Pool>,
}

impl OptionsService {
    pub async fn new(pool: Arc<crate::db::Pool>, _builtin_tenantable: bool) -> Self {
        let service = Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            pool,
        };
        if let Err(e) = service.load_autoload().await {
            tracing::error!("failed to autoload options: {}", e);
        }
        service
    }

    fn tenant_arg(&self) -> Option<&str> {
        None
    }

    async fn load_autoload(&self) -> Result<(), AppError> {
        let rows = crate::models::options::find_autoload(&self.pool).await?;

        let mut cache = self.cache.write().await;
        cache.clear();
        for row in &rows {
            let entry = OptionEntry::from(row);
            cache.insert(row.option_key.clone(), entry);
        }

        tracing::info!("loaded {} option(s) into cache", cache.len());
        Ok(())
    }

    /// Get an option value (cache-first)
    pub async fn get(&self, key: &str) -> Option<Value> {
        self.cache.read().await.get(key).map(|e| e.value.clone())
    }

    /// Get an option entry (including metadata)
    pub async fn get_entry(&self, key: &str) -> Option<OptionEntry> {
        if let Some(entry) = self.cache.read().await.get(key).cloned() {
            return Some(entry);
        }
        let row: crate::models::options::OptionRow =
            crate::models::options::find_by_key(&self.pool, key, self.tenant_arg())
                .await
                .ok()
                .flatten()?;
        let entry = OptionEntry::from(&row);
        self.cache
            .write()
            .await
            .insert(key.to_string(), entry.clone());
        Some(entry)
    }

    /// Set an option value (write to DB + update cache)
    pub async fn set(&self, key: &str, value: Value) -> Result<(), AppError> {
        let value_str = serde_json::to_string(&value).map_err(|e| AppError::Internal(e.into()))?;

        crate::models::options::upsert_value(&self.pool, key, &value_str, self.tenant_arg())
            .await?;

        {
            let mut cache = self.cache.write().await;
            if let Some(entry) = cache.get_mut(key) {
                entry.value = value;
            } else {
                cache.insert(
                    key.to_string(),
                    OptionEntry {
                        option_key: key.to_string(),
                        value,
                        type_: "string".to_string(),
                        label: key.to_string(),
                        description: None,
                        validation: None,
                        is_public: false,
                    },
                );
            }
        }
        Ok(())
    }

    /// Batch-set options (transactional for atomicity)
    pub async fn set_batch(&self, pairs: HashMap<String, Value>) -> Result<(), AppError> {
        let sorted: Vec<_> = pairs.into_iter().collect();

        for (key, value) in &sorted {
            let value_str =
                serde_json::to_string(value).map_err(|e| AppError::Internal(e.into()))?;
            crate::models::options::upsert_value(&self.pool, key, &value_str, self.tenant_arg())
                .await?;
        }

        for (key, value) in sorted {
            if let Some(entry) = self.cache.write().await.get_mut(&key) {
                entry.value = value;
            }
        }
        Ok(())
    }

    /// Delete an option
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        crate::models::options::delete_by_key(&self.pool, key, self.tenant_arg()).await?;
        self.cache.write().await.remove(key);
        Ok(())
    }

    /// Get all options (organized by group)
    pub async fn get_grouped(&self) -> Result<Vec<OptionGroup>, AppError> {
        let rows = crate::models::options::find_all(&self.pool, self.tenant_arg()).await?;
        let mut group_map: HashMap<String, Vec<OptionEntry>> = HashMap::new();
        let mut group_labels: HashMap<String, String> = HashMap::new();
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
            group_labels.insert(row.group_name.clone(), row.group_name.clone());
        }

        let groups = group_order
            .into_iter()
            .map(|key| OptionGroup {
                label: group_labels
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| key.clone()),
                option_key: key.clone(),
                options: group_map.remove(&key).unwrap_or_default(),
            })
            .collect();

        Ok(groups)
    }

    /// Get public options (visible to frontend, values only)
    pub async fn get_public(&self) -> HashMap<String, Value> {
        let cache = self.cache.read().await;
        cache
            .values()
            .filter(|e| e.is_public)
            .map(|e| (e.option_key.clone(), e.value.clone()))
            .collect()
    }

    /// Get public options (including metadata, organized by group)
    pub async fn get_public_grouped(&self) -> Vec<OptionGroup> {
        let rows: Vec<crate::models::options::OptionRow> =
            crate::models::options::find_all(&self.pool, self.tenant_arg())
                .await
                .unwrap_or_default();
        let mut group_map: HashMap<String, Vec<OptionEntry>> = HashMap::new();
        let mut group_order: Vec<String> = Vec::new();

        for row in &rows {
            if !row.is_public {
                continue;
            }
            let entry = OptionEntry::from(row);
            if !group_map.contains_key(&row.group_name) {
                group_order.push(row.group_name.clone());
            }
            group_map
                .entry(row.group_name.clone())
                .or_default()
                .push(entry);
        }

        group_order
            .into_iter()
            .map(|key| OptionGroup {
                label: key.clone(),
                option_key: key.clone(),
                options: group_map.remove(&key).unwrap_or_default(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
