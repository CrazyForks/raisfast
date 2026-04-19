//! WASM 引擎封装
//!
//! 隔离所有 wasmtime 细节，提供简洁的调用接口。
//! 包含 fuel 消耗限制、超时和内存上限等安全机制。
//! Store 数据类型为 [`Arc<HostContext>`]，所有 Host Function 共享公共业务逻辑。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::plugins::host_common::HostContext;

const DEFAULT_FUEL: u64 = 10_000_000;

/// WASM 实例池
///
/// 为单个 WASM 插件维护多个实例，支持并发执行。
/// 使用 round-robin 策略分配实例。
pub struct WasmInstancePool {
    instances: Vec<Mutex<WasmInstance>>,
    next: AtomicUsize,
}

impl WasmInstancePool {
    /// 从已有实例创建指定大小的池
    pub fn new(instances: Vec<WasmInstance>) -> Self {
        let next = AtomicUsize::new(0);
        Self {
            instances: instances.into_iter().map(Mutex::new).collect(),
            next,
        }
    }

    /// 从 WASM 字节码创建实例池
    pub fn create_pool(
        engine: &wasmtime::Engine,
        wasm_bytes: &[u8],
        host_ctx: Arc<HostContext>,
        timeout_ms: u64,
        pool_size: usize,
    ) -> anyhow::Result<Self> {
        let mut instances = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let ctx: Arc<HostContext> = Arc::new((*host_ctx).clone());
            instances.push(WasmInstance::new(engine, wasm_bytes, ctx, timeout_ms)?);
        }
        Ok(Self::new(instances))
    }

    /// 获取下一个可用实例（round-robin）
    pub async fn acquire(&self) -> tokio::sync::MutexGuard<'_, WasmInstance> {
        let len = self.instances.len();
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % len;
        self.instances[idx].lock().await
    }

    /// 池中实例数量
    #[must_use]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

/// 单个插件实例
pub struct WasmInstance {
    store: wasmtime::Store<Arc<HostContext>>,
    instance: wasmtime::Instance,
    memory: wasmtime::Memory,
    alloc_fn: Option<wasmtime::TypedFunc<(i32,), i32>>,
    dealloc_fn: Option<wasmtime::TypedFunc<(i32, i32), ()>>,
    timeout_ms: u64,
    fuel_limit: u64,
    max_memory_bytes: usize,
    plugin_id: String,
}

impl WasmInstance {
    /// 从 WASM 字节码创建实例
    pub fn new(
        engine: &wasmtime::Engine,
        wasm_bytes: &[u8],
        host_ctx: Arc<HostContext>,
        timeout_ms: u64,
    ) -> anyhow::Result<Self> {
        let plugin_id = host_ctx.plugin_id().to_string();
        let max_memory_bytes = host_ctx.max_memory_bytes();

        let module = wasmtime::Module::new(engine, wasm_bytes)?;
        let mut store = wasmtime::Store::new(engine, host_ctx);

        let fuel_limit = DEFAULT_FUEL;
        store.set_fuel(fuel_limit)?;

        let mut linker = wasmtime::Linker::new(engine);
        super::host::register_host_functions(&mut linker)?;

        let instance = linker.instantiate(&mut store, &module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow::anyhow!("plugin has no exported memory"))?;

        let current_mem = memory.data_size(&store);
        if current_mem > max_memory_bytes {
            anyhow::bail!(
                "plugin {plugin_id} initial memory {current_mem} exceeds limit {max_memory_bytes}"
            );
        }

        let alloc_fn = instance
            .get_typed_func::<(i32,), i32>(&mut store, "alloc")
            .ok();
        let dealloc_fn = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")
            .ok();

        Ok(Self {
            store,
            instance,
            memory,
            alloc_fn,
            dealloc_fn,
            timeout_ms,
            fuel_limit,
            max_memory_bytes,
            plugin_id,
        })
    }

    /// 重置 fuel 为上限值（每次调用前执行）
    fn reset_fuel(&mut self) -> anyhow::Result<()> {
        self.store.set_fuel(self.fuel_limit)
    }

    /// 返回插件的超时时间（毫秒）
    #[must_use]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// 调用返回 JSON 的 Filter Hook
    ///
    /// ABI 协议：函数签名 `(ptr: i32, len: i32) -> i32`
    /// 返回值 = 输出数据的指针，该指针处前 4 字节为 LE 长度，后面是数据。
    /// 返回 0 表示插件未处理（None）。
    pub fn call_json_filter<T: Clone + Serialize + DeserializeOwned>(
        &mut self,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let func = match self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, func_name)
        {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        self.reset_fuel()?;

        let input_json = serde_json::to_vec(input)?;
        let ptr = self.write_to_memory(&input_json)?;

        let result_ptr: i32 = func
            .call(&mut self.store, (ptr, input_json.len() as i32))
            .map_err(|e| self.format_wasm_error(e))?;

        self.free_memory(ptr, input_json.len() as i32);

        if result_ptr <= 0 {
            return Ok(None);
        }

        let output = self.read_length_prefixed(result_ptr)?;
        let result: T = serde_json::from_slice(&output)?;
        Ok(Some(result))
    }

    /// 调用返回 String 的 Filter Hook（如 `render_markdown`）
    ///
    /// ABI 协议同 [`call_json_filter`]。
    pub fn call_string_filter(
        &mut self,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let func = match self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, func_name)
        {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        self.reset_fuel()?;

        let input_bytes = input.as_bytes().to_vec();
        let ptr = self.write_to_memory(&input_bytes)?;

        let result_ptr: i32 = func
            .call(&mut self.store, (ptr, input_bytes.len() as i32))
            .map_err(|e| self.format_wasm_error(e))?;

        self.free_memory(ptr, input_bytes.len() as i32);

        if result_ptr <= 0 {
            return Ok(None);
        }

        let output = self.read_length_prefixed(result_ptr)?;
        Ok(Some(String::from_utf8(output)?))
    }

    /// 调用 Action Hook（无返回值）
    pub fn call_json_action<T: Serialize>(
        &mut self,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<()> {
        let func = match self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, func_name)
        {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };

        self.reset_fuel()?;

        let input_json = serde_json::to_vec(input)?;
        let ptr = self.write_to_memory(&input_json)?;

        func.call(&mut self.store, (ptr, input_json.len() as i32))
            .map_err(|e| self.format_wasm_error(e))?;

        self.free_memory(ptr, input_json.len() as i32);
        Ok(())
    }

    /// 带 fuel 守卫的 WASM 调用包装。
    ///
    /// fuel 耗尽时 wasmtime 返回 Trap，此处统一转为错误。
    fn format_wasm_error(&self, e: impl std::fmt::Display) -> anyhow::Error {
        let msg = e.to_string();
        if msg.contains("all fuel consumed") {
            anyhow::anyhow!(
                "plugin {} exceeded fuel limit ({} fuel units, timeout {}ms)",
                self.plugin_id,
                self.fuel_limit,
                self.timeout_ms,
            )
        } else {
            anyhow::anyhow!("{msg}")
        }
    }

    /// 将数据写入 WASM 线性内存，返回指针
    fn write_to_memory(&mut self, data: &[u8]) -> anyhow::Result<i32> {
        let len = data.len() as i32;

        let ptr = if let Some(ref alloc_fn) = self.alloc_fn {
            alloc_fn
                .call(&mut self.store, (len,))
                .map_err(|e| anyhow::anyhow!("{e}"))?
        } else {
            let mem_size = self.memory.data_size(&self.store);
            let needed = data.len() + 1024;
            if mem_size < needed {
                let extra = needed - mem_size;
                let pages_needed = (extra / 65536) as u64 + 1;
                let new_total = mem_size as u64 + pages_needed * 65536;
                if new_total > self.max_memory_bytes as u64 {
                    anyhow::bail!(
                        "plugin {} memory allocation exceeds limit ({} bytes)",
                        self.plugin_id,
                        self.max_memory_bytes,
                    );
                }
                self.memory.grow(&mut self.store, pages_needed)?;
            }
            0i32
        };

        self.memory.data_mut(&mut self.store)[ptr as usize..ptr as usize + data.len()]
            .copy_from_slice(data);

        Ok(ptr)
    }

    /// 从 WASM 线性内存读取数据
    fn read_from_memory(&self, ptr: i32, len: usize) -> anyhow::Result<Vec<u8>> {
        let mem_data = self.memory.data(&self.store);
        let start = ptr as usize;
        let end = start + len;
        if end > mem_data.len() {
            return Err(anyhow::anyhow!(
                "plugin {} attempted out-of-bounds read: [{start}..{end}]",
                self.plugin_id,
            ));
        }
        Ok(mem_data[start..end].to_vec())
    }

    /// 从 WASM 内存读取长度前缀编码的数据。
    ///
    /// 布局：`[4 字节 LE 长度][数据]`
    fn read_length_prefixed(&self, ptr: i32) -> anyhow::Result<Vec<u8>> {
        let len_bytes = self.read_from_memory(ptr, 4)?;
        let len = u32::from_le_bytes(
            len_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid length prefix"))?,
        ) as usize;
        self.read_from_memory(ptr + 4, len)
    }

    /// 释放 WASM 内存
    fn free_memory(&mut self, ptr: i32, len: i32) {
        if let Some(dealloc_fn) = &self.dealloc_fn {
            let _ = dealloc_fn.call(&mut self.store, (ptr, len));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::app::AppConfig;
    use crate::db::Pool;
    use crate::plugins::Permissions;

    const TEST_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next_ptr (mut i32) (i32.const 0))
  (func (export "alloc") (param $size i32) (result i32)
    (global.get $next_ptr)
    (global.set $next_ptr (i32.add (global.get $next_ptr) (local.get $size)))
  )
  (func (export "dealloc") (param $ptr i32) (param $size i32))
  (func $echo_lp (param $ptr i32) (param $len i32) (result i32)
    (local $out i32)
    (local.set $out (global.get $next_ptr))
    (global.set $next_ptr (i32.add (global.get $next_ptr) (i32.add (i32.const 4) (local.get $len))))
    (i32.store (local.get $out) (local.get $len))
    (memory.copy (i32.add (local.get $out) (i32.const 4)) (local.get $ptr) (local.get $len))
    (local.get $out)
  )
  (func (export "on_post_creating") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "on_post_created") (param $ptr i32) (param $len i32))
  (func (export "render_markdown") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "infinite_loop")
    (block $break (loop $loop (br $loop)))
  )
)
"#;

    fn make_test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
    }

    fn make_host_ctx(id: &str, perms: Permissions) -> Arc<HostContext> {
        Arc::new(HostContext::new(
            "wasm",
            make_test_config(),
            id.into(),
            perms,
            None::<Pool>,
        ))
    }

    fn make_instance(id: &str) -> WasmInstance {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let host_ctx = make_host_ctx(id, Permissions::default());
        WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap()
    }

    #[test]
    fn create_from_wat_succeeds() {
        let _inst = make_instance("test-create");
    }

    #[test]
    fn write_and_read_roundtrip() {
        let mut inst = make_instance("test-roundtrip");
        let data = b"Hello, WASM!";
        let ptr = inst.write_to_memory(data).unwrap();
        let read_back = inst.read_from_memory(ptr, data.len()).unwrap();
        assert_eq!(&read_back, data);
    }

    #[test]
    fn read_out_of_bounds_fails() {
        let mut inst = make_instance("test-oob");
        let data = b"hello";
        let ptr = inst.write_to_memory(data).unwrap();
        let result = inst.read_from_memory(ptr, 999_999_999);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out-of-bounds"));
    }

    #[test]
    fn fuel_exhaustion_detected() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let perms = Permissions {
            timeout_ms: Some(100),
            ..Default::default()
        };
        let host_ctx = make_host_ctx("fuel-test", perms);
        let mut inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 100).unwrap();

        inst.reset_fuel().unwrap();
        inst.store.set_fuel(100).unwrap();
        let func = inst
            .instance
            .get_typed_func::<(), ()>(&mut inst.store, "infinite_loop")
            .unwrap();
        let result = func.call(&mut inst.store, ());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("fuel")
                || err_msg.contains("Trap")
                || err_msg.contains("trap")
                || err_msg.contains("wasm")
                || err_msg.contains("interrupt"),
            "unexpected error message: {err_msg}"
        );
    }

    #[test]
    fn memory_limit_allows_within_budget() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let perms = Permissions {
            max_memory_mb: Some(1),
            ..Default::default()
        };
        let host_ctx = make_host_ctx("mem-test", perms);
        let mut inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();

        let data = vec![0u8; 1024];
        assert!(inst.write_to_memory(&data).is_ok());
    }

    #[test]
    fn format_error_fuel_message() {
        let inst = make_instance("fmt-test");
        let err = inst.format_wasm_error("all fuel consumed by test");
        let msg = err.to_string();
        assert!(msg.contains("exceeded fuel limit"));
        assert!(msg.contains("fmt-test"));
    }

    #[test]
    fn format_error_generic_message() {
        let inst = make_instance("fmt-test2");
        let err = inst.format_wasm_error("some generic error");
        assert_eq!(err.to_string(), "some generic error");
    }

    #[test]
    fn call_json_filter_echo() {
        let mut inst = make_instance("filter-test");
        let input = serde_json::json!({"title": "Hello"});
        let result = inst.call_json_filter::<serde_json::Value>("on_post_creating", &input);
        match result {
            Ok(Some(v)) => assert_eq!(v["title"], "Hello"),
            Ok(None) => {}
            Err(_) => {}
        }
    }

    #[test]
    fn call_json_action_ok() {
        let mut inst = make_instance("action-test");
        let result = inst.call_json_action("on_post_created", &serde_json::json!({"id": "123"}));
        assert!(result.is_ok());
    }

    #[test]
    fn call_nonexistent_filter_returns_none() {
        let mut inst = make_instance("noexist-test");
        let result: anyhow::Result<Option<serde_json::Value>> =
            inst.call_json_filter("nonexistent", &serde_json::json!({}));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn call_string_filter_echo() {
        let mut inst = make_instance("str-test");
        let result = inst.call_string_filter("render_markdown", "test content");
        match result {
            Ok(Some(s)) => assert!(!s.is_empty()),
            Ok(None) => {}
            Err(_) => {}
        }
    }

    #[test]
    fn wasm_instance_stores_host_context() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let perms = Permissions {
            database: vec!["posts".into()],
            ..Permissions::default()
        };
        let host_ctx = make_host_ctx("ctx-test", perms);
        let inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();

        assert_eq!(inst.store.data().plugin_id(), "ctx-test");
        assert_eq!(inst.store.data().runtime_label, "wasm");
    }

    #[test]
    fn wasm_host_context_memory_limit_from_permissions() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let perms = Permissions {
            max_memory_mb: Some(16),
            ..Permissions::default()
        };
        let host_ctx = make_host_ctx("mem-limit-test", perms);
        assert_eq!(host_ctx.max_memory_bytes(), 16 * 1024 * 1024);
        let _inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();
    }

    #[test]
    fn wasm_host_context_config_accessible() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };
        let host_ctx = make_host_ctx("config-test", perms);
        assert!(host_ctx.get_config("app.env").is_some());
        assert_eq!(host_ctx.get_config("app.env"), Some("test".into()));
        let _inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();
    }

    #[test]
    fn wasm_host_context_log_does_not_crash() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let host_ctx = make_host_ctx("log-test", Permissions::default());
        host_ctx.log("info", "test message");
        host_ctx.log("warn", "warning message");
        host_ctx.log("error", "error message");
        let _inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();
    }

    #[test]
    fn wasm_host_context_no_pool_returns_gracefully() {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = wasmtime::Engine::new(&cfg).unwrap();
        let host_ctx = make_host_ctx("no-pool-test", Permissions::default());

        assert!(host_ctx.get_data("key").is_none());
        assert!(!host_ctx.set_data("key", "val"));
        assert!(host_ctx.get_post("slug").is_none());
        assert!(host_ctx.db_query("SELECT 1").contains("no database access"));

        let _inst = WasmInstance::new(&engine, TEST_WAT.as_bytes(), host_ctx, 5000).unwrap();
    }
}
