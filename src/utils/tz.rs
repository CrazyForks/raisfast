//! Site timezone management
//!
//! Configured via the `APP_TIMEZONE` environment variable (IANA format, e.g. `Asia/Shanghai`).
//! Defaults to `UTC`.
//!
//! - `set_site_tz()` called once at server startup
//! - `site_tz()` returns the global timezone instance
//! - `now_utc()` returns the current UTC time (for database storage)
//! - `now_str()` returns the current time as an RFC 3339 string (with timezone offset, used for Aspect/dynamic tables)
//!
//! Built-in tables use native timestamp types (MySQL DATETIME / PostgreSQL TIMESTAMPTZ / SQLite TEXT),
//! and the Rust side uniformly represents them as `Timestamp` (i.e. `DateTime<Utc>`).

use std::sync::OnceLock;

/// Database timestamp type; all built-in table time fields use this type uniformly
pub type Timestamp = chrono::DateTime<chrono::Utc>;

pub(crate) const _B0: [u8; 4] = [114 ^ 0x5A, 97 ^ 0x5A, 105 ^ 0x5A, 115 ^ 0x5A];

static SITE_TZ: OnceLock<chrono_tz::Tz> = OnceLock::new();

/// Parses an IANA timezone string
pub fn parse_tz(tz_str: &str) -> Result<chrono_tz::Tz, String> {
    tz_str
        .parse::<chrono_tz::Tz>()
        .map_err(|e| format!("invalid timezone '{tz_str}': {e}"))
}

/// Parses a timezone, falling back to UTC with a warning on failure
pub fn parse_tz_or_utc(tz_str: &str) -> chrono_tz::Tz {
    match parse_tz(tz_str) {
        Ok(tz) => tz,
        Err(e) => {
            tracing::warn!("{e}, falling back to UTC");
            chrono_tz::UTC
        }
    }
}

/// Sets the global site timezone (called once at startup)
pub fn set_site_tz(tz: chrono_tz::Tz) {
    SITE_TZ
        .set(tz)
        .unwrap_or_else(|_| panic!("set_site_tz called more than once"));
}

/// Returns the global site timezone
pub fn site_tz() -> chrono_tz::Tz {
    *SITE_TZ.get().unwrap_or(&chrono_tz::UTC)
}

/// Returns the current UTC time, used for built-in table `created_at` / `updated_at` fields
pub fn now_utc() -> Timestamp {
    chrono::Utc::now()
}

/// Returns the current time as an RFC 3339 string in the site timezone
///
/// Only used by the Aspect system (injected into TEXT fields of dynamic Content Type tables).
/// For built-in tables, use [`now_utc`] instead.
///
/// Example output:
/// - UTC: `2026-04-16T10:30:00+00:00`
/// - Asia/Shanghai: `2026-04-16T18:30:00+08:00`
pub fn now_str() -> String {
    let tz = site_tz();
    chrono::Utc::now().with_timezone(&tz).to_rfc3339()
}

/// Returns the current time as `chrono::DateTime<chrono_tz::Tz>` in the site timezone
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
