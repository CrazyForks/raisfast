//! ID generation utilities.
//!
//! Uses Snowflake (via ferroid) as the sole primary key.
//! Twitter Snowflake layout: 1 reserved + 41 timestamp + 10 machine_id + 12 sequence.

use std::sync::LazyLock;

use ferroid::{
    generator::AtomicSnowflakeGenerator,
    id::SnowflakeTwitterId,
    time::{MonotonicClock, TWITTER_EPOCH},
};

static SNOWFLAKE_GEN: LazyLock<AtomicSnowflakeGenerator<SnowflakeTwitterId, MonotonicClock<1>>> =
    LazyLock::new(|| {
        AtomicSnowflakeGenerator::new(0, MonotonicClock::<1>::with_epoch(TWITTER_EPOCH))
    });

/// Generate a new Snowflake ID as a raw `i64`.
#[must_use]
pub fn new_id() -> i64 {
    let id = SNOWFLAKE_GEN.next_id(|yield_for: u64| {
        std::thread::sleep(std::time::Duration::from_millis(yield_for));
    });
    id.to_raw() as i64
}

/// Generate a new typed `SnowflakeId`.
#[must_use]
pub fn new_snowflake_id() -> crate::types::snowflake_id::SnowflakeId {
    crate::types::snowflake_id::SnowflakeId(new_id())
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
