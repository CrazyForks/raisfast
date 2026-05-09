//! ID 与时间戳生成工具

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_id_is_valid_uuid_v7() {
        let id = new_document_id();
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn new_document_id_and_timestamp_returns_both() {
        let (id, ts) = new_document_id_and_timestamp();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        assert!(!ts.to_rfc3339().is_empty());
    }

    #[test]
    fn new_id_and_timestamp_is_alias() {
        let (id, ts) = new_id_and_timestamp();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        assert!(!ts.to_rfc3339().is_empty());
    }

    #[test]
    fn random_hex_length() {
        let hex = random_hex(16);
        assert_eq!(hex.len(), 32);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn random_hex_uniqueness() {
        let a = random_hex(32);
        let b = random_hex(32);
        assert_ne!(a, b);
    }

    #[test]
    fn random_hex_empty() {
        let hex = random_hex(0);
        assert_eq!(hex, "");
    }
}

/// 生成 document_id（UUID v7）和当前 UTC 时间戳。
///
/// 用于 model 层的 `create` 函数。`id` 由数据库自增生成，
/// `document_id` 在应用层生成用于对外暴露。
#[must_use]
pub fn new_document_id_and_timestamp() -> (String, super::tz::Timestamp) {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = super::tz::now_utc();
    (document_id, now)
}

/// 生成 document_id（UUID v7）。
#[must_use]
pub fn new_document_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Backwards-compatible alias for [`new_document_id_and_timestamp`].
#[must_use]
pub fn new_id_and_timestamp() -> (String, super::tz::Timestamp) {
    new_document_id_and_timestamp()
}

/// 生成指定字节数的随机 hex 字符串
#[must_use]
pub fn random_hex(byte_count: usize) -> String {
    let mut buf = vec![0u8; byte_count];
    getrandom::getrandom(&mut buf).unwrap_or_else(|e| panic!("random_hex failed: {e}"));
    hex::encode(buf)
}
