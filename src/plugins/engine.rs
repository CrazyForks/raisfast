//! WASM 引擎封装
//!
//! 隔离所有 wasmtime 细节，提供简洁的调用接口。

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 单个插件实例
pub struct WasmInstance {
    store: wasmtime::Store<InstanceState>,
    instance: wasmtime::Instance,
    memory: wasmtime::Memory,
    alloc_fn: Option<wasmtime::TypedFunc<(i32,), i32>>,
    dealloc_fn: Option<wasmtime::TypedFunc<(i32, i32), ()>>,
    #[allow(dead_code)]
    timeout_ms: u64,
    plugin_id: String,
}

struct InstanceState {
    #[allow(dead_code)]
    plugin_id: String,
}

impl WasmInstance {
    /// 从 WASM 字节码创建实例
    pub fn new(
        engine: &wasmtime::Engine,
        wasm_bytes: &[u8],
        plugin_id: String,
        timeout_ms: u64,
    ) -> anyhow::Result<Self> {
        let module = wasmtime::Module::new(engine, wasm_bytes)?;
        let mut store = wasmtime::Store::new(
            engine,
            InstanceState {
                plugin_id: plugin_id.clone(),
            },
        );

        store.set_fuel(u64::MAX)?;

        let mut linker = wasmtime::Linker::new(engine);

        linker.func_wrap("env", "host_log", {
            let pid = plugin_id.clone();
            move |level_ptr: i32, level_len: i32, msg_ptr: i32, msg_len: i32| {
                // 占位：实际日志在 call 时通过 store data 访问
                let _ = (level_ptr, level_len, msg_ptr, msg_len, &pid);
            }
        })?;

        let instance = linker.instantiate(&mut store, &module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow::anyhow!("plugin has no exported memory"))?;

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
            plugin_id,
        })
    }

    /// 调用返回 JSON 的 Filter Hook
    ///
    /// 返回 `Ok(Some(result))` 表示插件修改了数据，
    /// `Ok(None)` 表示插件未修改（返回 0 或函数不存在）。
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

        let input_json = serde_json::to_vec(input)?;
        let ptr = self.write_to_memory(&input_json)?;

        let result_len: i32 = func
            .call(&mut self.store, (ptr, input_json.len() as i32))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if result_len <= 0 {
            self.free_memory(ptr, input_json.len() as i32);
            return Ok(None);
        }

        let result_len = result_len as usize;
        let output = self.read_from_memory(ptr, result_len)?;
        self.free_memory(ptr, std::cmp::max(input_json.len(), result_len) as i32);

        let result: T = serde_json::from_slice(&output)?;
        Ok(Some(result))
    }

    /// 调用返回 String 的 Filter Hook（如 render_markdown）
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

        let input_bytes = input.as_bytes().to_vec();
        let ptr = self.write_to_memory(&input_bytes)?;

        let result_len: i32 = func
            .call(&mut self.store, (ptr, input_bytes.len() as i32))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if result_len <= 0 {
            self.free_memory(ptr, input_bytes.len() as i32);
            return Ok(None);
        }

        let result_len = result_len as usize;
        let output = self.read_from_memory(ptr, result_len)?;
        self.free_memory(ptr, std::cmp::max(input_bytes.len(), result_len) as i32);

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

        let input_json = serde_json::to_vec(input)?;
        let ptr = self.write_to_memory(&input_json)?;

        func.call(&mut self.store, (ptr, input_json.len() as i32))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        self.free_memory(ptr, input_json.len() as i32);
        Ok(())
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
            if mem_size < data.len() + 1024 {
                let pages_needed = ((data.len() + 1024) / 65536) as u64 + 1;
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

    /// 释放 WASM 内存
    fn free_memory(&mut self, ptr: i32, len: i32) {
        if let Some(dealloc_fn) = &self.dealloc_fn {
            let _ = dealloc_fn.call(&mut self.store, (ptr, len));
        }
    }
}
