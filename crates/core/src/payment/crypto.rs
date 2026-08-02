use crate::errors::app_error::{AppError, AppResult};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

pub fn aes256gcm_encrypt(plaintext: &str, key: &[u8; 32]) -> AppResult<String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes init: {e}")))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes256gcm encrypt: {e}")))?;
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

pub fn aes256gcm_decrypt(ciphertext_b64: &str, key: &[u8; 32]) -> AppResult<String> {
    let combined = BASE64
        .decode(ciphertext_b64)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes256gcm base64 decode: {e}")))?;
    if combined.len() < 12 {
        return Err(AppError::Internal(anyhow::anyhow!(
            "aes256gcm: ciphertext too short"
        )));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes init: {e}")))?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes256gcm decrypt: {e}")))?;
    String::from_utf8(plaintext)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("aes256gcm utf8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn roundtrip() {
        let key = test_key();
        let encrypted = aes256gcm_encrypt("hello world", &key).unwrap();
        let decrypted = aes256gcm_decrypt(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "hello world");
    }

    #[test]
    fn roundtrip_empty_string() {
        let key = test_key();
        let encrypted = aes256gcm_encrypt("", &key).unwrap();
        let decrypted = aes256gcm_decrypt(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = test_key();
        let key2 = [99u8; 32];
        let encrypted = aes256gcm_encrypt("secret", &key1).unwrap();
        assert!(aes256gcm_decrypt(&encrypted, &key2).is_err());
    }

    #[test]
    fn corrupted_data_fails() {
        let key = test_key();
        assert!(aes256gcm_decrypt("aGVsbG8=", &key).is_err());
    }
}
