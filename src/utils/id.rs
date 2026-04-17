//! ID 与时间戳生成工具

/// 生成 UUID v7 主键和当前站点时区时间戳。
///
/// 用于 model 层的 `create` 函数，避免每次重复写
/// `let id = Uuid::now_v7().to_string(); let now = super::tz::now_str();`。
#[must_use]
pub fn new_id_and_timestamp() -> (String, String) {
    let id = uuid::Uuid::now_v7().to_string();
    let now = super::tz::now_str();
    (id, now)
}

/// 生成指定字节数的随机 hex 字符串
#[must_use]
pub fn random_hex(byte_count: usize) -> String {
    let mut buf = vec![0u8; byte_count];
    getrandom::getrandom(&mut buf).unwrap_or_else(|e| panic!("random_hex failed: {e}"));
    hex::encode(buf)
}
