//! ChannelCache — the in-memory routing table (design §7.1).
//!
//! `routes` maps `RouteKey { tenant, group, model }` to candidate channels
//! (enabled only, pre-sorted by priority DESC — disabled channels are evicted
//! from the routing table immediately on write).
//! `models` caches the directory (status=active rows) including pricing.
//! Write-path invalidation lives in `service::LlmRouter`.

use std::collections::HashMap;
use std::str::FromStr as _;
use std::sync::Arc;

use crate::llm::models::channel::{
    LlmChannel, LlmChannelStatus, LlmKeyEntry, LlmKeyMode, LlmKeyStatus, parse_keys,
};
use crate::llm::models::model::{LlmModel, LlmModelStatus, LlmModelType, LlmPriceMode};
use crate::types::snowflake_id::SnowflakeId;

/// Routing key (design §7.1; `group` stays `"default"` until groups ship).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub tenant: String,
    pub group: String,
    pub model: String,
}

/// Pricing snapshot carried on `ModelInfo` (design §9.3: same cache, same
/// invalidation — the relay hot path never queries the DB for prices).
/// USD per 1M tokens (pricing.md §2); cache prices fall back to `input_price`.
#[derive(Debug, Clone)]
pub struct Pricing {
    pub price_mode: LlmPriceMode,
    pub input_price: f64,
    pub output_price: f64,
    pub cache_read_price: Option<f64>,
    pub cache_write_price: Option<f64>,
    pub call_price: Option<f64>,
}

/// Model metadata + pricing resolved with a request (design §10.1).
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub model_type: LlmModelType,
    pub pricing: Pricing,
    pub params: Option<serde_json::Value>,
}

/// One decrypted key inside the cached channel. `plain` is `None` when
/// decryption failed — the key is marked disabled and skipped (lenient, §11.3).
#[derive(Debug, Clone)]
pub struct CachedKey {
    pub plain: Option<String>,
    pub status: LlmKeyStatus,
    pub max_concurrency: Option<i32>,
}

/// Immutable cached channel shared via `Arc`; state changes clone-and-replace.
#[derive(Debug, Clone)]
pub struct CachedChannel {
    pub id: SnowflakeId,
    pub tenant: String,
    pub provider: String,
    pub base_url: String,
    pub keys: Vec<CachedKey>,
    pub key_mode: LlmKeyMode,
    pub status: LlmChannelStatus,
    pub models: Vec<String>,
    pub model_mapping: Option<serde_json::Value>,
    pub priority: i64,
    pub weight: i32,
    pub groups: Vec<String>,
    pub auto_ban: bool,
    /// Fallback concurrency cap for keys without their own
    /// `max_concurrency` (`channel.config.max_concurrency`, design §7.6).
    pub default_max_concurrency: Option<i32>,
    /// Probe model for the channel test endpoint / health cron (§7.5).
    pub test_model: Option<String>,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
    pub config: Option<serde_json::Value>,
    /// Upstream cost model (pricing.md §7): per-token discount for `usage`,
    /// undefined per-token for `fixed` (cost lives in monthly_cost).
    pub cost_mode: crate::llm::models::channel::LlmCostMode,
    pub cost_discount: f64,
}

fn tenant_of(row: &LlmChannel) -> String {
    row.tenant_id
        .clone()
        .unwrap_or_else(|| "default".to_owned())
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Built-in common-model seed (design §5.4: directory lookup = DB row →
/// built-in → 400; both count as "registered"). Final pair = USD / 1M tokens
/// (input, output) — vendor list prices (pricing.md §2).
const BUILTIN_MODELS: &[(&str, &str, f64, f64)] = &[
    ("gpt-4o", "chat", 2.5, 10.0),
    ("gpt-4o-mini", "chat", 0.15, 0.6),
    ("o3-mini", "chat", 1.1, 4.4),
    ("claude-sonnet-4-5", "chat", 3.0, 15.0),
    ("claude-haiku-4-5", "chat", 1.0, 5.0),
    ("deepseek-chat", "chat", 0.27, 1.1),
    ("deepseek-reasoner", "chat", 0.55, 2.19),
    ("gemini-2.0-flash", "chat", 0.15, 0.6),
    ("qwen-plus", "chat", 0.4, 1.2),
    ("text-embedding-3-small", "embedding", 0.02, 0.02),
    ("text-embedding-3-large", "embedding", 0.13, 0.13),
];

fn builtin_model(name: &str) -> Option<Arc<ModelInfo>> {
    let (_, _, input, output) = BUILTIN_MODELS.iter().find(|(n, ..)| *n == name)?;
    let model_type = LlmModelType::from_str(
        BUILTIN_MODELS
            .iter()
            .find(|(n, ..)| *n == name)
            .map(|(_, t, ..)| *t)
            .unwrap_or("chat"),
    )
    .ok()?;
    Some(Arc::new(ModelInfo {
        name: name.to_owned(),
        model_type,
        pricing: Pricing {
            price_mode: LlmPriceMode::Token,
            input_price: *input,
            output_price: *output,
            cache_read_price: None,
            cache_write_price: None,
            call_price: None,
        },
        params: None,
    }))
}

/// The in-memory routing table snapshot.
#[derive(Debug, Default, Clone)]
pub struct ChannelCache {
    pub routes: HashMap<RouteKey, Vec<Arc<CachedChannel>>>,
    pub channels: HashMap<SnowflakeId, Arc<CachedChannel>>,
    pub models: HashMap<(String, String), Arc<ModelInfo>>,
}

impl ChannelCache {
    /// Build a cache snapshot from channel + directory rows.
    pub fn build(channels: Vec<LlmChannel>, models: Vec<LlmModel>) -> Self {
        let mut cache = ChannelCache::default();
        for row in channels {
            let cached = cached_from_row(&row);
            cache.channels.insert(cached.id, Arc::new(cached));
        }
        for row in models {
            if row.status != LlmModelStatus::Active {
                continue;
            }
            let tenant = row
                .tenant_id
                .clone()
                .unwrap_or_else(|| "default".to_owned());
            cache.models.insert(
                (tenant, row.name.clone()),
                Arc::new(ModelInfo {
                    name: row.name.clone(),
                    model_type: row.model_type,
                    pricing: Pricing {
                        price_mode: row.price_mode,
                        input_price: row.input_price,
                        output_price: row.output_price,
                        cache_read_price: row.cache_read_price,
                        cache_write_price: row.cache_write_price,
                        call_price: row.call_price,
                    },
                    params: row.params.clone(),
                }),
            );
        }
        cache.rebuild_routes();
        cache
    }

    /// Directory lookup: DB row → built-in seed → `None` (design §5.4).
    pub fn model_info(&self, tenant: &str, name: &str) -> Option<Arc<ModelInfo>> {
        self.models
            .get(&(tenant.to_owned(), name.to_owned()))
            .cloned()
            .or_else(|| builtin_model(name))
    }

    /// Candidates for a route key (already priority-DESC sorted).
    pub fn candidates(&self, key: &RouteKey) -> Option<&Vec<Arc<CachedChannel>>> {
        self.routes.get(key)
    }

    /// Rebuild every route entry from the channel map (O(channels × models));
    /// invoked on write-path invalidation — channel counts are small.
    pub fn rebuild_routes(&mut self) {
        let mut routes: HashMap<RouteKey, Vec<Arc<CachedChannel>>> = HashMap::new();
        for channel in self.channels.values() {
            if channel.status != LlmChannelStatus::Enabled {
                continue;
            }
            for group in &channel.groups {
                for model in &channel.models {
                    routes
                        .entry(RouteKey {
                            tenant: channel.tenant.clone(),
                            group: group.clone(),
                            model: model.clone(),
                        })
                        .or_default()
                        .push(channel.clone());
                }
            }
        }
        for candidates in routes.values_mut() {
            candidates.sort_by_key(|c| std::cmp::Reverse(c.priority));
        }
        self.routes = routes;
    }
}

impl ChannelCache {
    /// Convert a DB row into a cached channel (public for incremental
    /// reloads).
    pub fn from_row(row: &LlmChannel) -> CachedChannel {
        cached_from_row(row)
    }
}

/// Convert a DB row into a cached channel, decrypting keys leniently (§11.3:
/// undecryptable key → disabled + warn, never fails the row).
fn cached_from_row(row: &LlmChannel) -> CachedChannel {
    let entries: Vec<LlmKeyEntry> = parse_keys(row);
    let keys = entries
        .iter()
        .map(|e| {
            let plain = crate::llm::crypto::decrypt(&e.key);
            let undecryptable = plain.is_none();
            if undecryptable {
                tracing::warn!(
                    channel = %row.id,
                    "llm key undecryptable (aes key rotated/lost?), marked disabled"
                );
            }
            CachedKey {
                plain,
                status: if undecryptable {
                    LlmKeyStatus::Disabled
                } else {
                    e.status
                },
                max_concurrency: e.max_concurrency,
            }
        })
        .collect();
    CachedChannel {
        id: row.id,
        tenant: tenant_of(row),
        provider: row.provider.clone(),
        base_url: row.base_url.clone(),
        keys,
        key_mode: row.key_mode,
        status: row.status,
        models: split_csv(&row.models),
        model_mapping: row.model_mapping.clone(),
        priority: row.priority,
        weight: row.weight,
        groups: {
            let groups = split_csv(&row.channel_groups);
            if groups.is_empty() {
                vec!["default".to_owned()]
            } else {
                groups
            }
        },
        auto_ban: row.auto_ban,
        default_max_concurrency: row
            .config
            .as_ref()
            .and_then(|c| c.get("max_concurrency"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        test_model: row.test_model.clone(),
        param_override: row.param_override.clone(),
        header_override: row.header_override.clone(),
        config: row.config.clone(),
        cost_mode: row.cost_mode,
        cost_discount: row.cost_discount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::models::channel::{
        LlmChannel, LlmChannelStatus, LlmKeyEntry, LlmKeyMode, LlmKeyStatus,
    };
    use crate::llm::models::model::{LlmModel, LlmModelStatus, LlmModelType, LlmPriceMode};

    fn row(id: i64, models: &str, groups: &str, priority: i64) -> LlmChannel {
        LlmChannel {
            id: SnowflakeId(id),
            tenant_id: Some("default".to_owned()),
            name: format!("ch{id}"),
            provider: "openai".to_owned(),
            base_url: "https://x.test/v1".to_owned(),
            api_keys: serde_json::to_value(vec![LlmKeyEntry {
                key: "plain-key".to_owned(),
                status: LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: None,
            }])
            .unwrap(),
            key_mode: LlmKeyMode::Polling,
            status: LlmChannelStatus::Enabled,
            models: models.to_owned(),
            model_mapping: None,
            priority,
            weight: 0,
            channel_groups: groups.to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            used_quota: 0,
            cost_mode: crate::llm::models::channel::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
            test_time: None,
            response_time: None,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn model_row(name: &str, model_type: LlmModelType, status: LlmModelStatus) -> LlmModel {
        LlmModel {
            id: SnowflakeId(new_id_for_test()),
            tenant_id: Some("default".to_owned()),
            name: name.to_owned(),
            model_type,
            price_mode: LlmPriceMode::Token,
            input_price: 2.0,
            output_price: 4.0,
            cache_read_price: None,
            cache_write_price: None,
            call_price: None,
            params: None,
            status,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn new_id_for_test() -> i64 {
        static C: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(9000);
        C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn route(tenant: &str, group: &str, model: &str) -> RouteKey {
        RouteKey {
            tenant: tenant.to_owned(),
            group: group.to_owned(),
            model: model.to_owned(),
        }
    }

    #[test]
    fn routes_fan_out_over_groups_and_models() {
        let cache = ChannelCache::build(vec![row(1, "m1,m2", "default,vip", 0)], vec![]);
        let ch = cache.channels.get(&SnowflakeId(1)).cloned().unwrap();
        assert_eq!(ch.groups, vec!["default", "vip"]);
        assert_eq!(ch.models.len(), 2);
        for (g, m) in [
            ("default", "m1"),
            ("default", "m2"),
            ("vip", "m1"),
            ("vip", "m2"),
        ] {
            assert!(
                cache.routes.contains_key(&route("default", g, m)),
                "missing route {g}/{m}"
            );
        }
    }

    #[test]
    fn disabled_channel_evicted_from_routes_kept_in_map() {
        let mut r = row(2, "m1", "default", 0);
        r.status = LlmChannelStatus::ManualDisabled;
        let cache = ChannelCache::build(vec![r], vec![]);
        assert!(cache.channels.contains_key(&SnowflakeId(2)));
        assert!(
            !cache
                .routes
                .contains_key(&route("default", "default", "m1"))
        );
    }

    #[test]
    fn candidates_sorted_by_priority_desc() {
        let cache = ChannelCache::build(
            vec![row(1, "m1", "default", 1), row(2, "m1", "default", 9)],
            vec![],
        );
        let cands = cache
            .candidates(&route("default", "default", "m1"))
            .expect("route exists");
        assert_eq!(cands[0].id, SnowflakeId(2));
        assert_eq!(cands[1].id, SnowflakeId(1));
    }

    #[test]
    fn directory_lookup_order_db_then_builtin_then_none() {
        let cache = ChannelCache::build(
            vec![],
            vec![
                model_row("db-registered", LlmModelType::Chat, LlmModelStatus::Active),
                model_row("db-disabled", LlmModelType::Chat, LlmModelStatus::Disabled),
            ],
        );
        assert!(cache.model_info("default", "db-registered").is_some());
        // Disabled DB rows do not resolve (§5.4 global delisting)…
        assert!(cache.model_info("default", "db-disabled").is_none());
        // …and never shadow the builtin seed for the same name.
        let cache2 = ChannelCache::build(
            vec![],
            vec![model_row(
                "gpt-4o",
                LlmModelType::Chat,
                LlmModelStatus::Disabled,
            )],
        );
        assert!(
            cache2.model_info("default", "gpt-4o").is_some(),
            "builtin fallback"
        );
        assert!(
            cache
                .model_info("default", "no-such-model-anywhere")
                .is_none()
        );
        // Tenant isolation on directory rows.
        assert!(cache.model_info("other-tenant", "db-registered").is_none());
    }

    #[test]
    fn default_concurrency_from_channel_config() {
        let mut r = row(3, "m1", "default", 0);
        r.config = Some(serde_json::json!({ "max_concurrency": 7 }));
        let ch = ChannelCache::from_row(&r);
        assert_eq!(ch.default_max_concurrency, Some(7));
        let ch2 = ChannelCache::from_row(&row(4, "m1", "default", 0));
        assert_eq!(ch2.default_max_concurrency, None);
    }

    #[test]
    fn mapped_model_applies_after_selection() {
        let mut r = row(5, "gpt-4", "default", 0);
        r.model_mapping = Some(serde_json::json!({ "gpt-4": "gpt-4-0613" }));
        let ch = ChannelCache::from_row(&r);
        assert_eq!(
            crate::llm::service::mapped_model(&ch, "gpt-4"),
            "gpt-4-0613"
        );
        assert_eq!(crate::llm::service::mapped_model(&ch, "other"), "other");
        let plain = ChannelCache::from_row(&row(6, "m", "default", 0));
        assert_eq!(crate::llm::service::mapped_model(&plain, "m"), "m");
    }

    #[test]
    fn empty_groups_defaults_to_default_group() {
        let mut r = row(7, "m1", "", 0);
        r.channel_groups = "".to_owned();
        let ch = ChannelCache::from_row(&r);
        assert_eq!(ch.groups, vec!["default".to_owned()]);
    }
}
