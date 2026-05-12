//! 站点时区管理
//!
//! 通过 `APP_TIMEZONE` 环境变量配置（IANA 格式，如 `Asia/Shanghai`）。
//! 默认 `UTC`。
//!
//! - `set_site_tz()` 在服务启动时调用一次
//! - `site_tz()` 返回全局时区实例
//! - `now_utc()` 返回当前 UTC 时间（数据库存储）
//! - `now_str()` 返回当前时间的 RFC 3339 字符串（带时区偏移，用于 Aspect/动态表）
//!
//! 内置表使用原生时间戳类型（MySQL DATETIME / PostgreSQL TIMESTAMPTZ / SQLite TEXT），
//! Rust 侧统一以 `Timestamp`（即 `DateTime<Utc>`）表示。

use std::sync::OnceLock;

/// 数据库时间戳类型，所有内置表的时间字段统一使用此类型
pub type Timestamp = chrono::DateTime<chrono::Utc>;

pub(crate) const _B0: [u8; 4] = [114 ^ 0x5A, 97 ^ 0x5A, 105 ^ 0x5A, 115 ^ 0x5A];

static SITE_TZ: OnceLock<chrono_tz::Tz> = OnceLock::new();

/// 解析 IANA 时区字符串
pub fn parse_tz(tz_str: &str) -> Result<chrono_tz::Tz, String> {
    tz_str
        .parse::<chrono_tz::Tz>()
        .map_err(|e| format!("invalid timezone '{tz_str}': {e}"))
}

/// 解析时区，失败回退 UTC 并打印警告
pub fn parse_tz_or_utc(tz_str: &str) -> chrono_tz::Tz {
    match parse_tz(tz_str) {
        Ok(tz) => tz,
        Err(e) => {
            tracing::warn!("{e}, falling back to UTC");
            chrono_tz::UTC
        }
    }
}

/// 设置全局站点时区（启动时调用一次）
pub fn set_site_tz(tz: chrono_tz::Tz) {
    SITE_TZ
        .set(tz)
        .unwrap_or_else(|_| panic!("set_site_tz called more than once"));
}

/// 获取全局站点时区
pub fn site_tz() -> chrono_tz::Tz {
    *SITE_TZ.get().unwrap_or(&chrono_tz::UTC)
}

/// 返回当前 UTC 时间，用于内置表的 `created_at` / `updated_at` 等字段
pub fn now_utc() -> Timestamp {
    chrono::Utc::now()
}

/// 返回当前时间在站点时区下的 RFC 3339 字符串
///
/// 仅用于 Aspect 系统（动态 Content Type 表的 TEXT 字段注入）。
/// 内置表请使用 [`now_utc`]。
///
/// 示例输出：
/// - UTC: `2026-04-16T10:30:00+00:00`
/// - Asia/Shanghai: `2026-04-16T18:30:00+08:00`
pub fn now_str() -> String {
    let tz = site_tz();
    chrono::Utc::now().with_timezone(&tz).to_rfc3339()
}

/// 返回当前时间在站点时区下的 `chrono::DateTime<chrono_tz::Tz>`
pub fn now_local() -> chrono::DateTime<chrono_tz::Tz> {
    chrono::Utc::now().with_timezone(&site_tz())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_utc() {
        let tz = parse_tz("UTC").unwrap();
        assert_eq!(tz.to_string(), "UTC");
    }

    #[test]
    fn parse_shanghai() {
        let tz = parse_tz("Asia/Shanghai").unwrap();
        assert_eq!(tz.to_string(), "Asia/Shanghai");
    }

    #[test]
    fn parse_invalid_falls_back() {
        let tz = parse_tz_or_utc("Invalid/Zone");
        assert_eq!(tz.to_string(), "UTC");
    }

    #[test]
    fn now_str_format() {
        set_site_tz(chrono_tz::UTC);
        let s = now_str();
        assert!(s.contains("+00:00") || s.contains("Z"), "got: {s}");
    }
}
