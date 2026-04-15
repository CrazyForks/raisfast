//! 站点配置服务
//!
//! 启动时将 `autoload=true` 的配置预加载到内存，
//! 后续读取优先走缓存，写入时同步更新缓存和数据库。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::errors::app_error::AppError;
use crate::repositories::OptionsRepository;

/// 公开配置 key 白名单（前端可通过 `/api/v1/options/public` 读取）
pub static PUBLIC_OPTIONS: &[&str] = &[
    "site_title",
    "site_description",
    "posts_per_page",
    "comment_order",
    "theme",
    "timezone",
    "date_format",
    "permalink_structure",
    "rss_items",
    "maintenance_mode",
];

/// 将数据库中的配置值字符串解析为 `serde_json::Value`。
///
/// 若字符串是合法 JSON 则解析为对应的值类型，否则作为纯字符串返回。
fn parse_value(value_str: String) -> Value {
    serde_json::from_str::<Value>(&value_str).unwrap_or(Value::String(value_str))
}

/// 站点配置服务
pub struct OptionsService {
    cache: Arc<RwLock<HashMap<String, Value>>>,
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
        for (key, value_str) in rows {
            let value = parse_value(value_str);
            cache.insert(key, value);
        }

        tracing::info!("loaded {} option(s) into cache", cache.len());
        Ok(())
    }

    /// 获取配置（优先查缓存，miss 时查 DB 并缓存）
    pub async fn get(&self, key: &str) -> Option<Value> {
        if let Some(v) = self.cache.read().await.get(key).cloned() {
            return Some(v);
        }

        match self.repo.find_by_key(key).await.ok().flatten() {
            Some(value_str) => {
                let value = parse_value(value_str);
                self.cache
                    .write()
                    .await
                    .insert(key.to_string(), value.clone());
                Some(value)
            }
            None => None,
        }
    }

    /// 设置配置（写入 DB + 更新缓存）
    pub async fn set(&self, key: &str, value: Value) -> Result<(), AppError> {
        let value_str = serde_json::to_string(&value)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("json serialize failed: {e}")))?;
        let now = chrono::Utc::now().to_rfc3339();

        self.repo.upsert(key, &value_str, &now).await?;
        self.cache.write().await.insert(key.to_string(), value);
        Ok(())
    }

    /// 批量设置配置
    pub async fn set_batch(&self, pairs: HashMap<String, Value>) -> Result<(), AppError> {
        for (key, value) in pairs {
            self.set(&key, value).await?;
        }
        Ok(())
    }

    /// 删除配置
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        self.repo.delete_by_key(key).await?;
        self.cache.write().await.remove(key);
        Ok(())
    }

    /// 获取所有配置（含非 autoload 的）
    pub async fn get_all(&self) -> Result<HashMap<String, Value>, AppError> {
        let rows = self.repo.find_all().await?;

        let mut map = HashMap::new();
        for (key, value_str) in rows {
            let value = parse_value(value_str);
            map.insert(key, value);
        }
        Ok(map)
    }

    /// 获取公开配置（前端可见）
    pub async fn get_public(&self) -> HashMap<String, Value> {
        let mut result = HashMap::new();
        for &key in PUBLIC_OPTIONS {
            if let Some(value) = self.get(key).await {
                result.insert(key.to_string(), value);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_options_whitelist() {
        assert!(PUBLIC_OPTIONS.contains(&"site_title"));
        assert!(PUBLIC_OPTIONS.contains(&"theme"));
        assert!(!PUBLIC_OPTIONS.contains(&"default_role"));
        assert!(!PUBLIC_OPTIONS.contains(&"comment_moderation"));
    }
}
