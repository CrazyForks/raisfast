//! Upstream key encryption at rest — AES-256-GCM with `enc:v1:` prefix
//! (design §11).
//!
//! - Key = `APP_KEY` (base64, exactly 32 bytes) — the same at-rest
//!   credential key already used by api_token and payment credentials
//!   ([自造-偏离 WeKnora：WeKnora ships a dedicated env key; raisfast
//!   converges all third-party-credential-at-rest encryption on APP_KEY —
//!   one secret to govern, auto-generated and persisted by `AppConfig`]).
//! - Install once at startup via [`install_from_app_key`]; encryption
//!   refuses until installed (design §11.2 tightening: no plaintext keys
//!   at rest).
//! - Decryption is lenient: unprefixed values pass through (legacy/dev
//!   rows); an authenticated-decryption failure yields `None` — the key
//!   is marked disabled (§11.3).

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use crate::errors::app_error::{AppError, AppResult};

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Process-wide at-rest key, installed once from `APP_KEY` at startup.
static KEY: std::sync::OnceLock<[u8; KEY_LEN]> = std::sync::OnceLock::new();

/// Install the at-rest key from the app key (base64, 32 bytes). Idempotent
/// for tests: the first successful install wins; later calls with the same
/// key no-op silently.
///
/// # Errors
///
/// `AppError::Internal` when the app key is not valid base64 or not
/// exactly 32 bytes.
pub fn install_from_app_key(app_key: &str) -> AppResult<()> {
    let decoded = B64
        .decode(app_key)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("APP_KEY base64 decode: {e}")))?;
    if decoded.len() != KEY_LEN {
        return Err(AppError::Internal(anyhow::anyhow!(
            "APP_KEY must decode to 32 bytes, got {}",
            decoded.len()
        )));
    }
    let mut arr = [0u8; KEY_LEN];
    arr.copy_from_slice(&decoded);
    let _ = KEY.set(arr);
    Ok(())
}

/// Whether at-rest encryption is configured (i.e. key writes are allowed).
pub fn is_enabled() -> bool {
    KEY.get().is_some()
}

/// Encrypt one upstream key with the installed app key.
pub fn encrypt(plain: &str) -> AppResult<String> {
    let key = KEY.get().ok_or_else(|| {
        AppError::BadRequest(
            "refusing to store plaintext upstream keys: APP_KEY not configured".to_owned(),
        )
    })?;
    seal(key, plain)
}

/// Decrypt a stored value. Returns `None` for undecryptable input —
/// callers decide lenient handling.
pub fn decrypt(stored: &str) -> Option<String> {
    if !stored.starts_with(PREFIX) {
        return Some(stored.to_owned());
    }
    open(KEY.get()?, stored)
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

/// Environment-free open (testable core): authenticated decryption — a
/// wrong key always fails.
fn open(key: &[u8; KEY_LEN], stored: &str) -> Option<String> {
    let payload = B64.decode(stored.trim_start_matches(PREFIX)).ok()?;
    if payload.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ct) = payload.split_at(NONCE_LEN);
    let nonce = Nonce::<U12>::try_from(nonce).ok()?;
    let k = Key::<Aes256Gcm>::try_from(key.as_slice()).ok()?;
    let cipher = Aes256Gcm::new(&k);
    let plain = cipher.decrypt(&nonce, ct).ok()?;
    String::from_utf8(plain).ok()
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

    fn b64_key(seed: u8) -> String {
        B64.encode(key(seed))
    }

    #[test]
    fn seal_open_roundtrip() {
        let k = key(1);
        let sealed = seal(&k, "sk-upstream-secret").expect("seal");
        assert!(sealed.starts_with(PREFIX));
        assert_ne!(sealed, "sk-upstream-secret");
        assert_eq!(open(&k, &sealed).as_deref(), Some("sk-upstream-secret"));
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&key(2), "rotate-me").expect("seal");
        assert_eq!(open(&key(9), &sealed), None);
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
        assert_eq!(open(&key(1), "enc:v1:!!!notbase64"), None);
        assert_eq!(open(&key(1), &format!("{PREFIX}AAAA")), None);
    }

    #[test]
    fn decrypt_passthrough_for_unprefixed_without_install() {
        // Lenient: legacy plaintext values pass through even when no key
        // is installed yet.
        assert_eq!(decrypt("legacy-plain"), Some("legacy-plain".to_owned()));
    }

    #[test]
    fn install_rejects_bad_app_key() {
        assert!(install_from_app_key("not-base64!!!").is_err());
        assert!(install_from_app_key(&B64.encode([0u8; 16])).is_err());
    }

    #[test]
    fn install_then_encrypt_decrypt_roundtrip() {
        // First-wins OnceLock: whatever key is installed (possibly by an
        // earlier test in this process), encrypt/decrypt stay consistent.
        let _ = install_from_app_key(&b64_key(7));
        if is_enabled() {
            let sealed = encrypt("sk-e2e").expect("encrypt");
            assert_eq!(decrypt(&sealed).as_deref(), Some("sk-e2e"));
        }
    }

    #[test]
    fn mask_hides_middle() {
        let masked = super::mask_key("sk-abcdefghij");
        assert_eq!(masked, "sk-abc…ghij");
    }
}
