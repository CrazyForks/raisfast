//! 站点时区管理
//!
//! 通过 `APP_TIMEZONE` 环境变量配置（IANA 格式，如 `Asia/Shanghai`）。
//! 默认 `UTC`。
//!
//! - `set_site_tz()` 在服务启动时调用一次
//! - `site_tz()` 返回全局时区实例
//! - `now_str()` 返回当前时间的 RFC 3339 字符串（带时区偏移）
//!
//! 数据库存储统一使用站点时区，API 原样返回。

use std::sync::OnceLock;

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

/// 返回当前时间在站点时区下的 RFC 3339 字符串
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
