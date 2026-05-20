use std::ops::Deref;
use std::sync::LazyLock;

use base64::Engine;

const B62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

static ID_CIPHER_KEY: LazyLock<[u8; 16]> = LazyLock::new(|| {
    let app_key = std::env::var("APP_KEY").unwrap_or_default();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&app_key)
        .unwrap_or_default();
    if bytes.len() >= 16 {
        let mut key = [0u8; 16];
        key.copy_from_slice(&bytes[..16]);
        key
    } else {
        [0u8; 16]
    }
});

static ID_ENCODING_ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var("ID_ENCODING").unwrap_or_default() == "true");

/// Encrypt a Snowflake i64 into a base62 string.
///
/// Uses 3-round XOR-rotate for confusion-diffusion. Output is 1-11 URL-safe
/// alphanumeric characters. Fully deterministic and invertible via `decode_id`.
pub fn encode_id(val: i64) -> String {
    if !*ID_ENCODING_ENABLED {
        return val.to_string();
    }
    let key = &*ID_CIPHER_KEY;
    let k0 = u64::from_be_bytes(key[0..8].try_into().unwrap());
    let k1 = u64::from_be_bytes(key[8..16].try_into().unwrap());
    let rot1 = (key[8] & 0x3F) as u32;
    let rot2 = (key[9] & 0x3F) as u32;

    let mut v = val as u64;
    v ^= k0;
    v = v.rotate_left(rot1);
    v ^= k1;
    v = v.rotate_left(rot2);
    v ^= k0.wrapping_add(k1);

    if v == 0 {
        return "0".into();
    }
    let mut buf = [0u8; 11];
    let mut pos = 11usize;
    while v > 0 {
        pos -= 1;
        buf[pos] = B62[(v % 62) as usize];
        v /= 62;
    }
    std::str::from_utf8(&buf[pos..]).unwrap().into()
}

/// Decrypt a base62-encoded ID string back to the original Snowflake i64.
///
/// Returns `None` if the string contains invalid base62 characters or
/// the decoded value overflows u64.
pub fn decode_id(s: &str) -> Option<i64> {
    if !*ID_ENCODING_ENABLED {
        return s.parse().ok();
    }
    let key = &*ID_CIPHER_KEY;
    let k0 = u64::from_be_bytes(key[0..8].try_into().unwrap());
    let k1 = u64::from_be_bytes(key[8..16].try_into().unwrap());
    let rot1 = (key[8] & 0x3F) as u32;
    let rot2 = (key[9] & 0x3F) as u32;

    let mut v = 0u64;
    for c in s.bytes() {
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'Z' => c - b'A' + 10,
            b'a'..=b'z' => c - b'a' + 36,
            _ => return None,
        } as u64;
        v = v.checked_mul(62)?.checked_add(digit)?;
    }

    v ^= k0.wrapping_add(k1);
    v = v.rotate_right(rot2);
    v ^= k1;
    v = v.rotate_right(rot1);
    v ^= k0;

    Some(v as i64)
}

/// Returns true if ID encoding is enabled via the `ID_ENCODING=true` env var.
pub fn is_id_encoding_enabled() -> bool {
    *ID_ENCODING_ENABLED
}

/// A Snowflake ID stored as `i64` in the database.
///
/// When `ID_ENCODING=true`:
/// - Serializes to an encrypted base62 string (e.g. `"aB3xKp9"`)
/// - Deserializes from the same encrypted string
///
/// When `ID_ENCODING` is unset or not `"true"`:
/// - Serializes to a plain numeric string (e.g. `"1895784563210240001"`)
/// - Deserializes from string or number
///
/// sqlx always reads/writes the raw `i64` regardless of encoding mode.
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

impl std::str::FromStr for SnowflakeId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(SnowflakeId)
    }
}

impl PartialEq<i64> for SnowflakeId {
    fn eq(&self, other: &i64) -> bool {
        &self.0 == other
    }
}

impl serde::Serialize for SnowflakeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode_id(self.0))
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
                if *ID_ENCODING_ENABLED {
                    decode_id(v)
                        .map(SnowflakeId)
                        .ok_or_else(|| serde::de::Error::custom("invalid encoded id"))
                } else {
                    v.parse().map(SnowflakeId).map_err(serde::de::Error::custom)
                }
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

/// Serialize an `i64` as a JSON **string** (or encrypted base62 when encoding is on).
///
/// Prefer using `SnowflakeId` newtype directly. This function is kept for
/// ad-hoc fields that cannot use the newtype.
pub fn serialize_id_as_string<S: serde::Serializer>(
    value: &i64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&encode_id(*value))
}

/// Parse/decode a string ID (from URL path or JSON) into a `SnowflakeId`.
///
/// When `ID_ENCODING=true`, decrypts the base62 string.
/// Otherwise parses a plain numeric string.
pub fn parse_id(id: &str) -> Result<SnowflakeId, crate::errors::app_error::AppError> {
    if *ID_ENCODING_ENABLED {
        decode_id(id)
            .map(SnowflakeId)
            .ok_or_else(|| crate::errors::app_error::AppError::BadRequest("invalid id".into()))
    } else {
        id.parse::<i64>()
            .map(SnowflakeId)
            .map_err(|e| crate::errors::app_error::AppError::BadRequest(format!("invalid id: {e}")))
    }
}

/// Parse/decode a string ID into a `SnowflakeId`.
///
/// Alias for `parse_id` (which now returns `SnowflakeId` directly).
pub fn parse_snowflake_id(id: &str) -> Result<SnowflakeId, crate::errors::app_error::AppError> {
    parse_id(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let ids = [1i64, 42, 999, 1895784563210240001, i64::MAX, i64::MIN + 1];
        for id in ids {
            let encoded = encode_id(id);
            let decoded = decode_id(&encoded);
            assert_eq!(
                decoded,
                Some(id),
                "roundtrip failed for {id}: encoded={encoded}"
            );
        }
    }

    #[test]
    fn encode_produces_base62() {
        let encoded = encode_id(12345);
        assert!(
            encoded.chars().all(|c| c.is_ascii_alphanumeric()),
            "encoded should be base62: {encoded}"
        );
    }

    #[test]
    fn encode_deterministic() {
        let a = encode_id(42);
        let b = encode_id(42);
        assert_eq!(a, b);
    }

    #[test]
    fn encode_different_ids_differ() {
        let a = encode_id(1);
        let b = encode_id(2);
        assert_ne!(a, b);
    }

    #[test]
    fn decode_invalid_returns_none() {
        assert!(decode_id("!!!invalid!!!").is_none());
    }
}
