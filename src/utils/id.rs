//! ID 与时间戳生成工具

/// 生成 UUID v7 主键和当前 UTC 时间戳。
///
/// 用于 model 层的 `create` 函数，避免每次重复写
/// `let id = Uuid::now_v7().to_string(); let now = Utc::now().to_rfc3339();`。
#[must_use]
pub fn new_id_and_timestamp() -> (String, String) {
    let id = uuid::Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    (id, now)
}
