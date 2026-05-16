//! Cryptographic utility functions.

/// Signs using HMAC-SHA1 (Base64 encoded).
///
/// The key is automatically appended with a `&` suffix (OAuth 1.0 signing spec).
pub fn hmac_sha1_sign(key: &str, data: &str) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    type HmacSha1 = Hmac<Sha1>;

    let key_with_suffix = format!("{key}&");
    let mut mac = HmacSha1::new_from_slice(key_with_suffix.as_bytes())
        .unwrap_or_else(|_| panic!("HMAC-SHA1 accepts any key size"));
    mac.update(data.as_bytes());
    let result = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha1_deterministic() {
        let a = hmac_sha1_sign("key", "data");
        let b = hmac_sha1_sign("key", "data");
        assert_eq!(a, b);
    }

    #[test]
    fn hmac_sha1_different_keys() {
        let a = hmac_sha1_sign("key1", "data");
        let b = hmac_sha1_sign("key2", "data");
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_sha1_different_data() {
        let a = hmac_sha1_sign("key", "data1");
        let b = hmac_sha1_sign("key", "data2");
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_sha1_is_base64() {
        let result = hmac_sha1_sign("secret", "message");
        use base64::Engine;
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(&result)
                .is_ok()
        );
    }
}
