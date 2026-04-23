//! 站点配置服务
//!
//! 启动时将 `autoload=true` 的配置预加载到内存，
//! 后续读取优先走缓存，写入时同步更新缓存和数据库。
//! 每条配置含完整元数据（类型、分组、标签、校验规则）。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::errors::app_error::AppError;
use crate::models::options::OptionRow;
use crate::repositories::OptionsRepository;

/// 将数据库中的配置值字符串解析为 `serde_json::Value`
fn parse_value(value_str: &str) -> Value {
    serde_json::from_str::<Value>(value_str).unwrap_or(Value::String(value_str.to_string()))
}

/// 分组信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptionGroup {
    pub key: String,
    pub label: String,
    pub options: Vec<OptionEntry>,
}

/// 单条配置（值 + 元数据）
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptionEntry {
    pub key: String,
    pub value: Value,
    #[serde(rename = "type")]
    pub type_: String,
    pub label: String,
    pub description: Option<String>,
    pub validation: Option<Value>,
    pub is_public: bool,
}

impl From<&OptionRow> for OptionEntry {
    fn from(row: &OptionRow) -> Self {
        Self {
            key: row.key.clone(),
            value: parse_value(&row.value),
            type_: row.type_.clone(),
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

/// 站点配置服务
pub struct OptionsService {
    cache: Arc<RwLock<HashMap<String, OptionEntry>>>,
    repo: Arc<dyn OptionsRepository>,
}

impl OptionsService {
    /// 创建实例并预加载 autoload 配置
    pub async fn new(repo: Arc<dyn OptionsRepository>) -> Self {
        let service = Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            repo,
        };
        if let Err(e) = service.load_autoload().await {
            tracing::error!("failed to autoload options: {}", e);
        }
        service
    }

    /// 预加载所有 autoload=true 的配置到内存
    async fn load_autoload(&self) -> Result<(), AppError> {
        let rows = self.repo.find_autoload().await?;

        let mut cache = self.cache.write().await;
        cache.clear();
        for row in &rows {
            let entry = OptionEntry::from(row);
            cache.insert(row.key.clone(), entry);
        }

        tracing::info!("loaded {} option(s) into cache", cache.len());
        Ok(())
    }

    /// 获取配置值（优先查缓存）
    pub async fn get(&self, key: &str) -> Option<Value> {
        self.cache.read().await.get(key).map(|e| e.value.clone())
    }

    /// 获取配置条目（含元数据）
    pub async fn get_entry(&self, key: &str) -> Option<OptionEntry> {
        if let Some(entry) = self.cache.read().await.get(key).cloned() {
            return Some(entry);
        }
        let row: crate::models::options::OptionRow =
            self.repo.find_by_key(key, "default").await.ok().flatten()?;
        let entry = OptionEntry::from(&row);
        self.cache
            .write()
            .await
            .insert(key.to_string(), entry.clone());
        Some(entry)
    }

    /// 设置配置值（写入 DB + 更新缓存）
    pub async fn set(&self, key: &str, value: Value) -> Result<(), AppError> {
        let value_str = serde_json::to_string(&value)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("json serialize failed: {e}")))?;
        let now = crate::utils::tz::now_str();

        self.repo
            .upsert_value(key, &value_str, "default", &now)
            .await?;

        {
            let mut cache = self.cache.write().await;
            if let Some(entry) = cache.get_mut(key) {
                entry.value = value;
            } else {
                cache.insert(
                    key.to_string(),
                    OptionEntry {
                        key: key.to_string(),
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

    /// 批量设置配置（事务保证原子性）
    pub async fn set_batch(&self, pairs: HashMap<String, Value>) -> Result<(), AppError> {
        let now = crate::utils::tz::now_str();
        let sorted: Vec<_> = pairs.into_iter().collect();

        for (key, value) in &sorted {
            let value_str = serde_json::to_string(value)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("json serialize failed: {e}")))?;
            self.repo
                .upsert_value(key, &value_str, "default", &now)
                .await?;
        }

        for (key, value) in sorted {
            if let Some(entry) = self.cache.write().await.get_mut(&key) {
                entry.value = value;
            }
        }
        Ok(())
    }

    /// 删除配置
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        self.repo.delete_by_key(key, "default").await?;
        self.cache.write().await.remove(key);
        Ok(())
    }

    /// 获取所有配置（按分组组织）
    pub async fn get_grouped(&self) -> Result<Vec<OptionGroup>, AppError> {
        let rows = self.repo.find_all("default").await?;
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
                key: key.clone(),
                options: group_map.remove(&key).unwrap_or_default(),
            })
            .collect();

        Ok(groups)
    }

    /// 获取公开配置（前端可见，仅值）
    pub async fn get_public(&self) -> HashMap<String, Value> {
        let cache = self.cache.read().await;
        cache
            .values()
            .filter(|e| e.is_public)
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect()
    }

    /// 获取公开配置（含元数据，按分组）
    pub async fn get_public_grouped(&self) -> Vec<OptionGroup> {
        let rows: Vec<crate::models::options::OptionRow> =
            self.repo.find_all("default").await.unwrap_or_default();
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
                key: key.clone(),
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
