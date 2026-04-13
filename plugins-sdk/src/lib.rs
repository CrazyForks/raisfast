//! rust-blog 插件开发 SDK
//!
//! 提供编写 WASM 插件所需的类型定义和辅助宏。
//!
//! # 快速开始
//!
//! ```rust,ignore
//! use rust_blog_plugin_sdk::*;
//!
//! // 导出插件必须实现的 Hook 函数
//! // 每个函数都是可选的，只需导出你需要的 Hook
//!
//! /// 创建文章前的过滤器
//! /// 接收 JSON 格式的 CreatePostRequest，返回修改后的版本
//! #[no_mangle]
//! pub extern "C" fn on_post_creating(ptr: i32, len: i32) -> i32 {
//!     let input = read_input::<CreatePostInput>(ptr, len);
//!     // ... 修改 input ...
//!     write_output(&input)
//! }
//! ``@

use serde::{Deserialize, Serialize};

/// 分配 WASM 线性内存
///
/// 宿主调用此函数为输入数据分配内存。
/// 插件必须导出此函数。
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    let mut buf = Vec::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr() as i32;
    std::mem::forget(buf);
    ptr
}

/// 释放 WASM 线性内存
///
/// 宿主调用此函数释放之前分配的内存。
/// 插件必须导出此函数。
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: i32, size: i32) {
    let _ = Vec::from_raw_parts(ptr as *mut u8, size as usize, size as usize);
}

/// 文章输入数据（对应 CreatePostRequest）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePostInput {
    pub title: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_image: Option<String>,
}

/// 文章输出数据（对应 PostResponse）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostOutput {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub status: String,
    pub author_id: String,
    pub category_id: Option<String>,
    pub view_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub published_at: Option<String>,
}

/// 评论输入数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentInput {
    pub content: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub parent_id: Option<String>,
}

/// 从 WASM 内存读取并反序列化 JSON 输入
///
/// `ptr` 和 `len` 是宿主写入 WASM 内存的数据位置和长度。
pub fn read_input<T: for<'de> Deserialize<'de>>(ptr: i32, len: i32) -> T {
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    serde_json::from_slice(bytes).expect("failed to deserialize plugin input")
}

/// 将数据序列化为 JSON 并写入 WASM 内存
///
/// 返回写入的字节数。宿主从同一 `ptr` 位置读取结果。
pub fn write_output<T: Serialize>(data: &T) -> i32 {
    let json = serde_json::to_vec(data).expect("failed to serialize plugin output");
    let len = json.len() as i32;
    let ptr = alloc(len);
    unsafe {
        std::ptr::copy_nonoverlapping(json.as_ptr(), ptr as *mut u8, json.len());
    }
    len
}

/// 将字符串写入 WASM 内存，返回写入长度
pub fn write_string_output(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let len = bytes.len() as i32;
    let ptr = alloc(len);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    }
    len
}

/// 从 WASM 内存读取字符串
pub fn read_string_input(ptr: i32, len: i32) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8(bytes.to_vec()).expect("invalid utf8 in plugin input")
}

/// 从内存读取原始字节
pub fn read_bytes(ptr: i32, len: i32) -> Vec<u8> {
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec() }
}

/// 写入原始字节到内存
pub fn write_bytes(bytes: &[u8]) -> i32 {
    let len = bytes.len() as i32;
    let ptr = alloc(len);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    }
    len
}
