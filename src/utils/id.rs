//! ID 与时间戳生成工具

/// 生成 document_id（UUID v7）和当前站点时区时间戳。
///
/// 用于 model 层的 `create` 函数。`id` 由数据库自增生成，
/// `document_id` 在应用层生成用于对外暴露。
#[must_use]
pub fn new_document_id_and_timestamp() -> (String, String) {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = super::tz::now_str();
    (document_id, now)
}

/// 生成 document_id（UUID v7）。
#[must_use]
pub fn new_document_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Backwards-compatible alias for [`new_document_id_and_timestamp`].
#[must_use]
pub fn new_id_and_timestamp() -> (String, String) {
    new_document_id_and_timestamp()
}

/// 生成指定字节数的随机 hex 字符串
#[must_use]
pub fn random_hex(byte_count: usize) -> String {
    let mut buf = vec![0u8; byte_count];
    getrandom::getrandom(&mut buf).unwrap_or_else(|e| panic!("random_hex failed: {e}"));
    hex::encode(buf)
}
