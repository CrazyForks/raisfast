//! Upstream key encryption at rest — AES-256-GCM with `enc:v1:` prefix
//! (design §11).
//!
//! - The prefix is a *format* version, not a key generation marker.
//! - Key ring: decryption tries `RAISFAST_AES_KEY` first, then
//!   `RAISFAST_AES_KEY_OLD` (rotation window, §11.6). AES-GCM is
//!   authenticated, so a wrong key always fails — try-both is unambiguous.
//! - Writing keys without `RAISFAST_AES_KEY` set is rejected (design §11.2
//!   tightening); reading stays lenient (`decrypt` returns `None`).

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use crate::errors::app_error::{AppError, AppResult};

const PREFIX: &str = "enc:v1:";
const KEY_ENV: &str = "RAISFAST_AES_KEY";
const KEY_OLD_ENV: &str = "RAISFAST_AES_KEY_OLD";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

fn key_from_env(env: &str) -> Option<[u8; KEY_LEN]> {
    let raw = std::env::var(env).ok()?;
    let bytes = raw.as_bytes();
    if bytes.len() != KEY_LEN {
        tracing::warn!(
            env,
            len = bytes.len(),
            "{env} must be exactly 32 bytes, ignored"
        );
        return None;
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(bytes);
    Some(key)
}

fn current_key() -> Option<[u8; KEY_LEN]> {
    key_from_env(KEY_ENV)
}

fn old_key() -> Option<[u8; KEY_LEN]> {
    key_from_env(KEY_OLD_ENV)
}

/// Whether at-rest encryption is configured (i.e. key writes are allowed).
pub fn is_enabled() -> bool {
    current_key().is_some()
}

/// Encrypt one upstream key with the current AES key.
pub fn encrypt(plain: &str) -> AppResult<String> {
    let key = current_key().ok_or_else(|| {
        AppError::BadRequest(
            "refusing to store plaintext upstream keys: set RAISFAST_AES_KEY (32 bytes)".to_owned(),
        )
    })?;
    seal(&key, plain)
}

/// Environment-free seal (testable core).
fn seal(key: &[u8; KEY_LEN], plain: &str) -> AppResult<String> {
    let k = Key::<Aes256Gcm>::try_from(key.as_slice())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid aes key length")))?;
    let cipher = Aes256Gcm::new(&k);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("generate nonce: {e}")))?;
    let nonce = Nonce::<U12>::try_from(nonce_bytes.as_slice())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid nonce length")))?;
    let ct = cipher
        .encrypt(&nonce, plain.as_bytes())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("aes-gcm encrypt failed")))?;
    let mut buf = Vec::with_capacity(NONCE_LEN + ct.len());
    buf.extend_from_slice(&nonce_bytes);
    buf.extend_from_slice(&ct);
    Ok(format!("{PREFIX}{}", B64.encode(buf)))
}

/// Decrypt a stored value via the key ring (current key first, then OLD).
/// Returns `None` for undecryptable input — callers decide lenient handling.
pub fn decrypt(stored: &str) -> Option<String> {
    if !stored.starts_with(PREFIX) {
        return Some(stored.to_owned());
    }
    open_ring(stored, current_key().as_ref(), old_key().as_ref())
}

/// Environment-free open over a key ring (testable core): try each key in
/// order; authenticated encryption guarantees a wrong key always fails.
fn open_ring(
    stored: &str,
    current: Option<&[u8; KEY_LEN]>,
    old: Option<&[u8; KEY_LEN]>,
) -> Option<String> {
    let payload = B64.decode(stored.trim_start_matches(PREFIX)).ok()?;
    if payload.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ct) = payload.split_at(NONCE_LEN);
    let nonce = Nonce::<U12>::try_from(nonce).ok()?;
    for key in [current, old].into_iter().flatten() {
        let k = Key::<Aes256Gcm>::try_from(key.as_slice()).ok()?;
        let cipher = Aes256Gcm::new(&k);
        if let Ok(plain) = cipher.decrypt(&nonce, ct) {
            return String::from_utf8(plain).ok();
        }
    }
    None
}

/// Mask a stored (encrypted) key for admin DTOs: first 6 + last 4 of the
/// plaintext, or a placeholder when it cannot be decrypted (design §11.4).
pub fn mask_key(stored: &str) -> String {
    match decrypt(stored) {
        Some(plain) if plain.len() > 12 => {
            format!("{}…{}", &plain[..6], &plain[plain.len() - 4..])
        }
        Some(_) => "configured".to_owned(),
        None => "not-configured".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; KEY_LEN] {
        [seed; KEY_LEN]
    }

    #[test]
    fn seal_open_roundtrip() {
        let k = key(1);
        let sealed = seal(&k, "sk-upstream-secret").expect("seal");
        assert!(sealed.starts_with(PREFIX));
        assert_ne!(sealed, "sk-upstream-secret");
        let opened = open_ring(&sealed, Some(&k), None).expect("open");
        assert_eq!(opened, "sk-upstream-secret");
    }

    #[test]
    fn wrong_key_fails_then_old_key_recovers() {
        let old = key(2);
        let sealed = seal(&old, "rotate-me").expect("seal");
        // Current key wrong → None; key ring falls back to OLD (§11.6).
        assert_eq!(open_ring(&sealed, Some(&key(9)), None), None);
        assert_eq!(
            open_ring(&sealed, Some(&key(9)), Some(&old)).as_deref(),
            Some("rotate-me")
        );
    }

    #[test]
    fn nonce_uniqueness_yields_distinct_ciphertexts() {
        let k = key(3);
        let a = seal(&k, "same").expect("seal");
        let b = seal(&k, "same").expect("seal");
        assert_ne!(a, b, "random nonce must vary ciphertexts");
    }

    #[test]
    fn garbage_input_returns_none() {
        assert_eq!(open_ring("enc:v1:!!!notbase64", Some(&key(1)), None), None);
        assert_eq!(
            open_ring(&format!("{PREFIX}AAAA"), Some(&key(1)), None),
            None
        );
    }

    #[test]
    fn decrypt_passthrough_for_unprefixed() {
        // Lenient: legacy plaintext values pass through unchanged.
        assert_eq!(decrypt("legacy-plain"), Some("legacy-plain".to_owned()));
    }

    #[test]
    fn mask_hides_middle() {
        let k = key(4);
        let _sealed = seal(&k, "sk-abcdefghij").expect("seal");
        // mask_key reads env keys; without RAISFAST_AES_KEY set it reports
        // not-configured for prefixed values — assert the non-prefixed shape.
        let masked = super::mask_key("sk-abcdefghij");
        assert_eq!(masked, "sk-abc…ghij");
    }
}
