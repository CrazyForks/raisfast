//! WASM Component Model 引擎封装（方案 B+C：Semaphore + Mutex）
//!
//! 基于 wasmtime Component Model，通过 `bindgen!` 生成的类型化绑定
//! 直接调用插件导出函数，消除手动内存管理。
//!
//! 实例池使用 Semaphore 控制总并发 + per-instance Mutex 获取实例，
//! Semaphore 异步等待不阻塞 tokio worker。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, Semaphore};

use crate::plugins::bindings::PluginWorld;
use crate::plugins::bindings::exports::raisfast::plugin_wit::plugin_hooks::CommentInput;
use crate::plugins::bindings::exports::raisfast::plugin_wit::plugin_hooks::ContentEvent;
use crate::plugins::bindings::exports::raisfast::plugin_wit::plugin_hooks::PostInput;
use crate::plugins::bindings::exports::raisfast::plugin_wit::plugin_hooks::PostOutput;
use crate::plugins::host_common::HostContext;

const DEFAULT_FUEL: u64 = 10_000_000;

/// 单个 WASM Component 实例
pub struct WasmComponentInstance {
    store: wasmtime::Store<Arc<HostContext>>,
    bindings: PluginWorld,
    timeout_ms: u64,
    fuel_limit: u64,
    #[allow(dead_code)]
    plugin_id: String,
}

/// 带 Mutex 和忙标记的 WASM 实例
struct PooledInstance {
    instance: Mutex<WasmComponentInstance>,
    busy: AtomicBool,
}

/// WASM Component 实例池（Semaphore + Mutex）
///
/// 用 Semaphore 控制最大并发数（异步等待，不阻塞线程），
/// AtomicBool 快速定位空闲实例，Mutex 提供 &mut 访问。
pub struct WasmInstancePool {
    instances: Vec<PooledInstance>,
    semaphore: Semaphore,
}

impl WasmInstancePool {
    /// 从 WASM 字节码创建实例池
    pub fn create_pool(
        engine: &wasmtime::Engine,
        wasm_bytes: &[u8],
        host_ctx: Arc<HostContext>,
        timeout_ms: u64,
        pool_size: usize,
    ) -> anyhow::Result<Self> {
        let component = wasmtime::component::Component::from_binary(engine, wasm_bytes)?;

        let mut linker = wasmtime::component::Linker::new(engine);
        PluginWorld::add_to_linker(
            &mut linker,
            |ctx: &mut Arc<HostContext>| -> &mut Arc<HostContext> { ctx },
        )?;

        let mut instances = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let ctx: Arc<HostContext> = Arc::new((*host_ctx).clone());
            let mut store = wasmtime::Store::new(engine, ctx);
            store.set_fuel(DEFAULT_FUEL)?;

            let bindings = PluginWorld::instantiate(&mut store, &component, &linker)?;

            instances.push(PooledInstance {
                instance: Mutex::new(WasmComponentInstance {
                    store,
                    bindings,
                    timeout_ms,
                    fuel_limit: DEFAULT_FUEL,
                    plugin_id: host_ctx.plugin_id().to_string(),
                }),
                busy: AtomicBool::new(false),
            });
        }
        Ok(Self {
            instances,
            semaphore: Semaphore::new(pool_size),
        })
    }

    /// 获取一个空闲实例
    ///
    /// Semaphore.acquire() 异步等待可用 permit（不阻塞线程），
    /// 然后遍历 AtomicBool 找到第一个空闲实例并锁定。
    /// 返回的 MutexGuard 在 Drop 时通过 WasmInstanceGuard 自动释放 busy 标记。
    pub async fn acquire(&self) -> WasmInstanceGuard<'_> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .expect("semaphore closed unexpectedly");
        for (i, pooled) in self.instances.iter().enumerate() {
            if pooled
                .busy
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                let guard = pooled.instance.lock().await;
                return WasmInstanceGuard {
                    _permit,
                    guard,
                    index: i,
                    pool: self,
                };
            }
        }
        unreachable!("semaphore guaranteed a free instance but none found");
    }
}

/// RAII 守卫：持有 MutexGuard + SemaphorePermit，Drop 时自动释放
pub struct WasmInstanceGuard<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
    guard: tokio::sync::MutexGuard<'a, WasmComponentInstance>,
    index: usize,
    pool: &'a WasmInstancePool,
}

impl<'a> Drop for WasmInstanceGuard<'a> {
    fn drop(&mut self) {
        self.pool.instances[self.index]
            .busy
            .store(false, Ordering::Release);
    }
}

impl<'a> std::ops::Deref for WasmInstanceGuard<'a> {
    type Target = WasmComponentInstance;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<'a> std::ops::DerefMut for WasmInstanceGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl WasmComponentInstance {
    /// 返回插件的超时时间（毫秒）
    #[must_use]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// 调用返回 JSON 的 Filter Hook（泛型封装）
    ///
    /// 将 `serde_json::Value` 转换为 WIT 类型，调用对应的组件方法，
    /// 再将结果转换回 `serde_json::Value`。
    pub fn call_json_filter<T: Clone + Serialize + DeserializeOwned>(
        &mut self,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        self.store.set_fuel(self.fuel_limit)?;
        let input_val = serde_json::to_value(input)?;

        match func_name {
            "on_post_creating" | "on_post_updating" => {
                let wit = json_to_post_input(&input_val)?;
                let hooks = self.bindings.raisfast_plugin_wit_plugin_hooks();
                let result = match func_name {
                    "on_post_creating" => hooks.call_on_post_creating(&mut self.store, &wit)?,
                    "on_post_updating" => hooks.call_on_post_updating(&mut self.store, &wit)?,
                    _ => None,
                };
                Ok(result
                    .map(|r| post_input_to_json(&r))
                    .and_then(|v| serde_json::from_value(v).ok()))
            }
            "on_comment_creating" => {
                let wit = json_to_comment_input(&input_val)?;
                let hooks = self.bindings.raisfast_plugin_wit_plugin_hooks();
                let result = hooks.call_on_comment_creating(&mut self.store, &wit)?;
                Ok(result
                    .map(|r| comment_input_to_json(&r))
                    .and_then(|v| serde_json::from_value(v).ok()))
            }
            "on_content_creating" | "on_content_updating" => {
                let wit = json_to_content_event(&input_val)?;
                let hooks = self.bindings.raisfast_plugin_wit_plugin_hooks();
                let result = match func_name {
                    "on_content_creating" => {
                        hooks.call_on_content_creating(&mut self.store, &wit)?
                    }
                    "on_content_updating" => {
                        hooks.call_on_content_updating(&mut self.store, &wit)?
                    }
                    _ => None,
                };
                Ok(result
                    .map(|r| content_event_to_json(&r))
                    .and_then(|v| serde_json::from_value(v).ok()))
            }
            "render_markdown" | "filter_html" => {
                let s = input_val.as_str().unwrap_or("");
                let hooks = self.bindings.raisfast_plugin_wit_plugin_hooks();
                let result = match func_name {
                    "render_markdown" => hooks.call_render_markdown(&mut self.store, s)?,
                    "filter_html" => hooks.call_filter_html(&mut self.store, s)?,
                    _ => None,
                };
                Ok(result.and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()))
            }
            _ => Ok(None),
        }
    }

    /// 调用 Action Hook（无返回值）
    pub fn call_json_action<T: Serialize>(
        &mut self,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<()> {
        self.store.set_fuel(self.fuel_limit)?;
        let input_val = serde_json::to_value(input)?;
        let hooks = self.bindings.raisfast_plugin_wit_plugin_hooks();

        match func_name {
            "on_post_created" | "on_post_updated" => {
                let wit = json_to_post_output(&input_val)?;
                match func_name {
                    "on_post_created" => hooks.call_on_post_created(&mut self.store, &wit)?,
                    "on_post_updated" => hooks.call_on_post_updated(&mut self.store, &wit)?,
                    _ => {}
                }
            }
            "on_post_deleted" => {
                if let Some(id) = input_val.as_str() {
                    hooks.call_on_post_deleted(&mut self.store, id)?;
                }
            }
            "on_comment_created" => {
                let wit = json_to_comment_input(&input_val)?;
                hooks.call_on_comment_created(&mut self.store, &wit)?;
            }
            "on_content_created" | "on_content_updated" => {
                let wit = json_to_content_event(&input_val)?;
                match func_name {
                    "on_content_created" => hooks.call_on_content_created(&mut self.store, &wit)?,
                    "on_content_updated" => hooks.call_on_content_updated(&mut self.store, &wit)?,
                    _ => {}
                }
            }
            "on_content_deleted" => {
                let ct = input_val
                    .get("content_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = input_val.get("id").and_then(|v| v.as_str()).unwrap_or("");
                hooks.call_on_content_deleted(&mut self.store, ct, id)?;
            }
            "on_login" => {
                if let Some(uid) = input_val.as_str() {
                    hooks.call_on_login(&mut self.store, uid)?;
                }
            }
            "on_cron_tick" => {
                let payload = input_val.as_str().map(|s| s.to_string());
                hooks.call_on_cron_tick(&mut self.store, payload.as_deref())?;
            }
            _ => {}
        }
        Ok(())
    }
}

// ── JSON ↔ WIT 类型转换 ──────────────────────────────────────────

fn json_to_post_input(v: &serde_json::Value) -> anyhow::Result<PostInput> {
    Ok(PostInput {
        title: v["title"].as_str().unwrap_or("").to_string(),
        content: v["content"].as_str().unwrap_or("").to_string(),
        slug: v["slug"].as_str().map(|s| s.to_string()),
        excerpt: v["excerpt"].as_str().map(|s| s.to_string()),
        category_id: v["category_id"].as_str().map(|s| s.to_string()),
        tag_ids: v["tag_ids"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        }),
        status: v["status"].as_str().map(|s| s.to_string()),
        cover_image: v["cover_image"].as_str().map(|s| s.to_string()),
    })
}

fn post_input_to_json(p: &PostInput) -> serde_json::Value {
    serde_json::json!({
        "title": p.title,
        "content": p.content,
        "slug": p.slug,
        "excerpt": p.excerpt,
        "category_id": p.category_id,
        "tag_ids": p.tag_ids,
        "status": p.status,
        "cover_image": p.cover_image,
    })
}

fn json_to_post_output(v: &serde_json::Value) -> anyhow::Result<PostOutput> {
    Ok(PostOutput {
        id: v["id"].as_str().unwrap_or("").to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
        slug: v["slug"].as_str().unwrap_or("").to_string(),
        content: v["content"].as_str().unwrap_or("").to_string(),
        excerpt: v["excerpt"].as_str().map(|s| s.to_string()),
        status: v["status"].as_str().unwrap_or("").to_string(),
        created_by: v["created_by"].as_str().unwrap_or("").to_string(),
        updated_by: v["updated_by"].as_str().map(|s| s.to_string()),
        category_id: v["category_id"].as_str().map(|s| s.to_string()),
        view_count: v["view_count"].as_i64().unwrap_or(0),
        created_at: v["created_at"].as_str().unwrap_or("").to_string(),
        updated_at: v["updated_at"].as_str().unwrap_or("").to_string(),
        published_at: v["published_at"].as_str().map(|s| s.to_string()),
    })
}

fn json_to_comment_input(v: &serde_json::Value) -> anyhow::Result<CommentInput> {
    Ok(CommentInput {
        content: v["content"].as_str().unwrap_or("").to_string(),
        nickname: v["nickname"].as_str().map(|s| s.to_string()),
        email: v["email"].as_str().map(|s| s.to_string()),
        parent_id: v["parent_id"].as_str().map(|s| s.to_string()),
    })
}

fn comment_input_to_json(c: &CommentInput) -> serde_json::Value {
    serde_json::json!({
        "content": c.content,
        "nickname": c.nickname,
        "email": c.email,
        "parent_id": c.parent_id,
    })
}

fn json_to_content_event(v: &serde_json::Value) -> anyhow::Result<ContentEvent> {
    Ok(ContentEvent {
        content_type: v["content_type"].as_str().unwrap_or("").to_string(),
        data: v["data"].as_str().unwrap_or("").to_string(),
        id: v["id"].as_str().map(|s| s.to_string()),
    })
}

fn content_event_to_json(e: &ContentEvent) -> serde_json::Value {
    serde_json::json!({
        "content_type": e.content_type,
        "data": e.data,
        "id": e.id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_post_input_roundtrip() {
        let original = serde_json::json!({
            "title": "Hello",
            "content": "World",
            "slug": "hello",
            "tag_ids": ["tag1", "tag2"],
        });
        let wit = json_to_post_input(&original).unwrap();
        let back = post_input_to_json(&wit);
        assert_eq!(back["title"], "Hello");
        assert_eq!(back["slug"], "hello");
        assert_eq!(back["tag_ids"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn json_comment_input_roundtrip() {
        let original = serde_json::json!({
            "content": "nice post",
            "nickname": "alice",
        });
        let wit = json_to_comment_input(&original).unwrap();
        let back = comment_input_to_json(&wit);
        assert_eq!(back["content"], "nice post");
        assert_eq!(back["nickname"], "alice");
    }

    #[test]
    fn json_content_event_roundtrip() {
        let original = serde_json::json!({
            "content_type": "articles",
            "data": "{\"title\":\"test\"}",
            "id": "abc-123",
        });
        let wit = json_to_content_event(&original).unwrap();
        let back = content_event_to_json(&wit);
        assert_eq!(back["content_type"], "articles");
        assert_eq!(back["id"], "abc-123");
    }

    #[tokio::test]
    async fn wasm_pool_acquire_and_release() {
        let Some(pool) = try_create_test_pool(2).await else {
            return;
        };
        let g1 = pool.acquire().await;
        assert_eq!(g1.timeout_ms(), 5000);
        drop(g1);
    }

    #[tokio::test]
    async fn wasm_pool_concurrent_within_pool_size() {
        let Some(pool) = try_create_test_pool(3).await else {
            return;
        };
        let g1 = pool.acquire().await;
        let g2 = pool.acquire().await;
        let g3 = pool.acquire().await;

        assert_eq!(g1.timeout_ms(), 5000);
        assert_eq!(g2.timeout_ms(), 5000);
        assert_eq!(g3.timeout_ms(), 5000);

        drop(g1);
        drop(g2);
        drop(g3);
    }

    #[tokio::test]
    async fn wasm_pool_excess_waits_for_release() {
        let Some(pool) = try_create_test_pool(2).await else {
            return;
        };
        let pool = Arc::new(pool);
        let g1 = pool.acquire().await;
        let g2 = pool.acquire().await;

        let pool_clone = Arc::clone(&pool);
        let acquire_task = tokio::spawn(async move {
            let _g = pool_clone.acquire().await;
            true
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !acquire_task.is_finished(),
            "3rd acquire should be waiting (pool_size=2)"
        );

        drop(g1);
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), acquire_task)
            .await
            .unwrap()
            .unwrap();
        assert!(result, "3rd acquire should succeed after g1 dropped");

        drop(g2);
    }

    #[tokio::test]
    async fn wasm_pool_guard_drop_releases_permit() {
        let Some(pool) = try_create_test_pool(1).await else {
            return;
        };
        let pool = Arc::new(pool);
        {
            let _g = pool.acquire().await;
        }

        let pool_clone = Arc::clone(&pool);
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async move {
            let _g = pool_clone.acquire().await;
            true
        })
        .await
        .unwrap();
        assert!(result, "should acquire after guard dropped");
    }

    async fn try_create_test_pool(pool_size: usize) -> Option<WasmInstancePool> {
        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true);
        engine_config.wasm_component_model(true);
        let engine = wasmtime::Engine::new(&engine_config).ok()?;

        let wasm_bytes = find_wasm_fixture(&engine)?;

        let host_ctx = Arc::new(HostContext::new(
            "wasm",
            Arc::new(crate::config::app::AppConfig::test_defaults()),
            "test-plugin".to_string(),
            crate::plugins::Permissions::default(),
            None,
        ));

        WasmInstancePool::create_pool(&engine, &wasm_bytes, host_ctx, 5000, pool_size).ok()
    }

    fn find_wasm_fixture(engine: &wasmtime::Engine) -> Option<Vec<u8>> {
        let candidates = [
            "plugin-wit/test_plugin.wasm",
            "tests/fixtures/test_plugin.wasm",
            "plugins/seo-optimizer/seo_optimizer.wasm",
            "plugins/content-filter/content_filter.wasm",
        ];
        for path in &candidates {
            if let Ok(bytes) = std::fs::read(path) {
                if wasmtime::component::Component::from_binary(engine, &bytes).is_ok() {
                    return Some(bytes);
                }
            }
        }
        tracing::warn!(
            "skipping WASM pool tests: no valid WASM Component fixture found (build with `just plugins-build`)"
        );
        None
    }
}
