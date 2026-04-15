//! WASM 宿主函数 — 引擎绑定层
//!
//! 通过 wasmtime Linker 的 `func_wrap` 注册到 WASM `env` 模块。
//! 所有函数通过 `Caller<'_, Arc<HostContext>>` 访问公共业务逻辑和 WASM 线性内存。
//!
//! ## ABI 协议
//!
//! 所有 Host 函数的参数和返回值均使用 `(ptr: i32, len: i32)` 传递字符串。
//! 返回值 0 表示空（None / error），非零为长度前缀编码的响应指针。

use std::sync::Arc;

use wasmtime::{Caller, Linker};

use crate::plugins::host_common::HostContext;

/// 注册所有 Host Functions 到 wasmtime Linker。
pub fn register_host_functions(linker: &mut Linker<Arc<HostContext>>) -> anyhow::Result<()> {
    linker.func_wrap("env", "host_log", host_log)?;
    linker.func_wrap("env", "host_get_config", host_get_config)?;
    linker.func_wrap("env", "host_http_get", host_http_get)?;
    linker.func_wrap("env", "host_http_post", host_http_post)?;
    linker.func_wrap("env", "host_get_data", host_get_data)?;
    linker.func_wrap("env", "host_set_data", host_set_data)?;
    linker.func_wrap("env", "host_get_post", host_get_post)?;
    linker.func_wrap("env", "host_db_query", host_db_query)?;
    linker.func_wrap("env", "host_fs_read", host_fs_read)?;
    linker.func_wrap("env", "host_fs_write", host_fs_write)?;
    linker.func_wrap("env", "host_fs_delete", host_fs_delete)?;
    linker.func_wrap("env", "host_fs_exists", host_fs_exists)?;
    linker.func_wrap("env", "host_fs_list", host_fs_list)?;
    linker.func_wrap("env", "host_fs_stat", host_fs_stat)?;
    Ok(())
}

/// 从 WASM 线性内存读取字符串
fn read_string(caller: &mut Caller<'_, Arc<HostContext>>, ptr: i32, len: i32) -> Option<String> {
    let mem = caller.get_export("memory")?.into_memory()?;
    let data = mem.data(&mut *caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[start..end].to_vec()).ok()
}

/// 将字符串以长度前缀格式写入 WASM 内存末尾，返回指针。
///
/// 布局：`[4 字节 LE 长度][数据]`
fn write_string(caller: &mut Caller<'_, Arc<HostContext>>, s: &str) -> i32 {
    let bytes = s.as_bytes();
    let total_len = 4 + bytes.len();
    let mem = match caller
        .get_export("memory")
        .and_then(wasmtime::Extern::into_memory)
    {
        Some(m) => m,
        None => return 0,
    };

    let current_size = mem.data_size(&mut *caller);
    let needed = total_len + 1024;
    if current_size < needed {
        let extra = needed - current_size;
        let pages = (extra / 65536) as u64 + 1;
        let _ = mem.grow(&mut *caller, pages);
    }

    let data = mem.data_mut(&mut *caller);
    let base = current_size;
    if base + total_len > data.len() {
        return 0;
    }

    let len_bytes = (bytes.len() as u32).to_le_bytes();
    data[base..base + 4].copy_from_slice(&len_bytes);
    data[base + 4..base + 4 + bytes.len()].copy_from_slice(bytes);
    base as i32
}

fn host_log(
    caller: Caller<'_, Arc<HostContext>>,
    level_ptr: i32,
    level_len: i32,
    msg_ptr: i32,
    msg_len: i32,
) {
    let mut caller = caller;
    let level = read_string(&mut caller, level_ptr, level_len).unwrap_or_default();
    let msg = read_string(&mut caller, msg_ptr, msg_len).unwrap_or_default();
    caller.data().log(&level, &msg);
}

fn host_get_config(caller: Caller<'_, Arc<HostContext>>, key_ptr: i32, key_len: i32) -> i32 {
    let mut caller = caller;
    let key = match read_string(&mut caller, key_ptr, key_len) {
        Some(k) => k,
        None => return 0,
    };
    let val = caller.data().get_config(&key);
    match val {
        Some(v) => write_string(&mut caller, &v),
        None => 0,
    }
}

fn host_http_get(caller: Caller<'_, Arc<HostContext>>, url_ptr: i32, url_len: i32) -> i32 {
    let mut caller = caller;
    let url = match read_string(&mut caller, url_ptr, url_len) {
        Some(u) => u,
        None => return 0,
    };
    let result = caller.data().http_get(&url);
    write_string(&mut caller, &result)
}

fn host_http_post(
    caller: Caller<'_, Arc<HostContext>>,
    url_ptr: i32,
    url_len: i32,
    body_ptr: i32,
    body_len: i32,
) -> i32 {
    let mut caller = caller;
    let url = match read_string(&mut caller, url_ptr, url_len) {
        Some(u) => u,
        None => return 0,
    };
    let body = match read_string(&mut caller, body_ptr, body_len) {
        Some(b) => b,
        None => return 0,
    };
    let result = caller.data().http_post(&url, &body);
    write_string(&mut caller, &result)
}

fn host_get_data(caller: Caller<'_, Arc<HostContext>>, key_ptr: i32, key_len: i32) -> i32 {
    let mut caller = caller;
    let key = match read_string(&mut caller, key_ptr, key_len) {
        Some(k) => k,
        None => return 0,
    };
    match caller.data().get_data(&key) {
        Some(val) => write_string(&mut caller, &val),
        None => 0,
    }
}

fn host_set_data(
    caller: Caller<'_, Arc<HostContext>>,
    key_ptr: i32,
    key_len: i32,
    val_ptr: i32,
    val_len: i32,
) -> i32 {
    let mut caller = caller;
    let key = match read_string(&mut caller, key_ptr, key_len) {
        Some(k) => k,
        None => return 0,
    };
    let value = match read_string(&mut caller, val_ptr, val_len) {
        Some(v) => v,
        None => return 0,
    };
    i32::from(caller.data().set_data(&key, &value))
}

fn host_get_post(caller: Caller<'_, Arc<HostContext>>, slug_ptr: i32, slug_len: i32) -> i32 {
    let mut caller = caller;
    let slug = match read_string(&mut caller, slug_ptr, slug_len) {
        Some(s) => s,
        None => return 0,
    };
    match caller.data().get_post(&slug) {
        Some(json) => write_string(&mut caller, &json),
        None => 0,
    }
}

fn host_db_query(caller: Caller<'_, Arc<HostContext>>, sql_ptr: i32, sql_len: i32) -> i32 {
    let mut caller = caller;
    let sql = match read_string(&mut caller, sql_ptr, sql_len) {
        Some(s) => s,
        None => return 0,
    };
    let result = caller.data().db_query(&sql);
    write_string(&mut caller, &result)
}

fn host_fs_read(caller: Caller<'_, Arc<HostContext>>, path_ptr: i32, path_len: i32) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    match caller.data().fs_read(&path) {
        Ok(content) => write_string(&mut caller, &content),
        Err(_) => 0,
    }
}

fn host_fs_write(
    caller: Caller<'_, Arc<HostContext>>,
    path_ptr: i32,
    path_len: i32,
    content_ptr: i32,
    content_len: i32,
) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    let content = match read_string(&mut caller, content_ptr, content_len) {
        Some(s) => s,
        None => return 0,
    };
    i32::from(caller.data().fs_write(&path, &content).is_ok())
}

fn host_fs_delete(caller: Caller<'_, Arc<HostContext>>, path_ptr: i32, path_len: i32) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    i32::from(caller.data().fs_delete(&path).is_ok())
}

fn host_fs_exists(caller: Caller<'_, Arc<HostContext>>, path_ptr: i32, path_len: i32) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    match caller.data().fs_exists(&path) {
        Ok(true) => 1,
        _ => 0,
    }
}

fn host_fs_list(caller: Caller<'_, Arc<HostContext>>, path_ptr: i32, path_len: i32) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    match caller.data().fs_list(&path) {
        Ok(entries) => {
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            write_string(&mut caller, &json)
        }
        Err(_) => 0,
    }
}

fn host_fs_stat(caller: Caller<'_, Arc<HostContext>>, path_ptr: i32, path_len: i32) -> i32 {
    let mut caller = caller;
    let path = match read_string(&mut caller, path_ptr, path_len) {
        Some(s) => s,
        None => return 0,
    };
    match caller.data().fs_stat(&path) {
        Ok(json) => write_string(&mut caller, &json),
        Err(_) => 0,
    }
}
