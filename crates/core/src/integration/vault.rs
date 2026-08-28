//! Credential vault — L0 trust.
//!
//! Channel credentials are sealed with AES-256-GCM (reusing
//! `payment::crypto`) under a master key from `INTEGRATION_VAULT_KEY`.
//! Plaintext never leaves this module: admin APIs return only a
//! "has credentials" flag, plugins never see it at all.

use crate::errors::app_error::{AppError, AppResult};
use sha2::{Digest, Sha256};

/// Master key stretched to AES-256 size from the configured secret.
#[derive(Clone)]
pub struct Vault {
    key: [u8; 32],
}

impl Vault {
    /// Build from a configured master secret (any non-empty string).
    ///
    /// # Errors
    ///
    /// `AppError::BadRequest` when the secret is empty.
    pub fn from_secret(secret: &str) -> AppResult<Self> {
        if secret.is_empty() {
            return Err(AppError::BadRequest(
                "INTEGRATION_VAULT_KEY must not be empty".into(),
            ));
        }
        let key: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        Ok(Self { key })
    }

    /// Seal a credentials JSON blob → storable base64 string.
    ///
    /// # Errors
    ///
    /// Returns `AppError` on encryption failure.
    pub fn seal(&self, credentials_json: &str) -> AppResult<String> {
        crate::payment::crypto::aes256gcm_encrypt(credentials_json, &self.key)
    }

    /// Unseal a stored blob → credentials JSON.
    ///
    /// # Errors
    ///
    /// Returns `AppError` on decryption failure (wrong key or corrupted data).
    pub fn unseal(&self, sealed: &str) -> AppResult<String> {
        crate::payment::crypto::aes256gcm_decrypt(sealed, &self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let vault = Vault::from_secret("unit-test-secret").expect("vault");
        let sealed = vault.seal(r#"{"token":"abc"}"#).expect("seal");
        assert_ne!(sealed, r#"{"token":"abc"}"#);
        let opened = vault.unseal(&sealed).expect("unseal");
        assert_eq!(opened, r#"{"token":"abc"}"#);
    }

    #[test]
    fn wrong_key_fails() {
        let a = Vault::from_secret("key-a").expect("vault");
        let b = Vault::from_secret("key-b").expect("vault");
        let sealed = a.seal("secret-data").expect("seal");
        assert!(b.unseal(&sealed).is_err());
    }

    #[test]
    fn empty_secret_rejected() {
        assert!(Vault::from_secret("").is_err());
    }
}
