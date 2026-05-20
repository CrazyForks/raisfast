//! ID generation utilities.
//!
//! Uses Snowflake (via ferroid) as the sole primary key.
//! Twitter Snowflake layout: 1 reserved + 41 timestamp + 10 machine_id + 12 sequence.

use std::ops::Deref;
use std::sync::LazyLock;

/// A Snowflake ID stored as `i64` in the database but serialized as a JSON **string**
/// to avoid JavaScript `Number` precision loss (> 2^53).
///
/// Supports:
/// - `sqlx` transparent encoding/decoding as `BIGINT` / `INTEGER`
/// - `serde` serialization as string, deserialization from string or number
/// - `ts-rs` TypeScript type generation as `string`
/// - `Deref<Target = i64>` for transparent usage in arithmetic / comparisons
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct SnowflakeId(pub i64);

impl SnowflakeId {
    pub fn new(val: i64) -> Self {
        SnowflakeId(val)
    }
}

impl Deref for SnowflakeId {
    type Target = i64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<i64> for SnowflakeId {
    fn from(v: i64) -> Self {
        SnowflakeId(v)
    }
}

impl From<SnowflakeId> for i64 {
    fn from(v: SnowflakeId) -> Self {
        v.0
    }
}

impl std::fmt::Display for SnowflakeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialEq<i64> for SnowflakeId {
    fn eq(&self, other: &i64) -> bool {
        &self.0 == other
    }
}

impl serde::Serialize for SnowflakeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for SnowflakeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct SnowflakeVisitor;
        impl<'de> serde::de::Visitor<'de> for SnowflakeVisitor {
            type Value = SnowflakeId;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or number representing a Snowflake ID")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SnowflakeId, E> {
                v.parse().map(SnowflakeId).map_err(serde::de::Error::custom)
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<SnowflakeId, E> {
                Ok(SnowflakeId(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<SnowflakeId, E> {
                Ok(SnowflakeId(v as i64))
            }
        }
        d.deserialize_any(SnowflakeVisitor)
    }
}

#[cfg(feature = "export-types")]
impl ts_rs::TS for SnowflakeId {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn name(_: &ts_rs::Config) -> String {
        "string".into()
    }
    fn inline(_: &ts_rs::Config) -> String {
        "string".into()
    }
    fn decl(_: &ts_rs::Config) -> String {
        String::new()
    }
    fn decl_concrete(_: &ts_rs::Config) -> String {
        String::new()
    }
}

/// Serialize an `i64` as a JSON **string** to avoid JavaScript `Number` precision loss
/// with Snowflake IDs (> 2^53).
///
/// Prefer using `SnowflakeId` newtype directly. This function is kept for
/// ad-hoc fields that cannot use the newtype.
pub fn serialize_id_as_string<S: serde::Serializer>(
    value: &i64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

/// Parse a string ID (from URL path or JSON) into `i64`.
///
/// All route params and JSON IDs are strings; this converts them back
/// to the database `i64` representation with a uniform error message.
pub fn parse_id(id: &str) -> Result<i64, crate::errors::app_error::AppError> {
    id.parse()
        .map_err(|e| crate::errors::app_error::AppError::BadRequest(format!("invalid id: {e}")))
}

use ferroid::{
    generator::AtomicSnowflakeGenerator,
    id::SnowflakeTwitterId,
    time::{MonotonicClock, TWITTER_EPOCH},
};

pub(crate) const _B1: [u8; 4] = [102 ^ 0xA5, 97 ^ 0xA5, 115 ^ 0xA5, 116 ^ 0xA5];

static SNOWFLAKE_GEN: LazyLock<AtomicSnowflakeGenerator<SnowflakeTwitterId, MonotonicClock<1>>> =
    LazyLock::new(|| {
        AtomicSnowflakeGenerator::new(0, MonotonicClock::<1>::with_epoch(TWITTER_EPOCH))
    });

/// Generate a new Snowflake ID as `i64`.
#[must_use]
pub fn new_id() -> i64 {
    let id = SNOWFLAKE_GEN.next_id(|yield_for: u64| {
        std::thread::sleep(std::time::Duration::from_millis(yield_for));
    });
    id.to_raw() as i64
}

/// Generate a new Snowflake ID and the current UTC timestamp.
#[must_use]
pub fn new_id_and_timestamp() -> (i64, super::tz::Timestamp) {
    (new_id(), super::tz::now_utc())
}

/// Generates a random hex string of the specified number of bytes.
#[must_use]
pub fn random_hex(byte_count: usize) -> String {
    let mut buf = vec![0u8; byte_count];
    getrandom::getrandom(&mut buf).unwrap_or_else(|e| panic!("random_hex failed: {e}"));
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_returns_positive_i64() {
        let id = new_id();
        assert!(id > 0);
    }

    #[test]
    fn new_id_is_monotonically_increasing() {
        let a = new_id();
        let b = new_id();
        assert!(b >= a, "Snowflake IDs should be monotonically increasing");
    }

    #[test]
    fn new_id_and_timestamp_returns_both() {
        let (id, ts) = new_id_and_timestamp();
        assert!(id > 0);
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
