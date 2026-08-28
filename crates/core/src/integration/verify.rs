//! L0 verify — inbound trust checks (integration.md §6.1).
//!
//! Verifiers are pure functions over the decoded request; secrets come from
//! the channel's sealed credentials (vault-unsealed here, never logged).
//! All comparisons are constant-time.

use serde_json::Value;

use crate::integration::channel::ItgChannel;
use crate::integration::vault::Vault;

/// The decoded inbound HTTP request (transport-agnostic so tests don't need axum).
#[derive(Debug, Clone)]
pub struct InboundHttpRequest {
    pub method: String,
    /// Raw query string (no leading `?`), possibly empty.
    pub query: String,
    /// Header name/value pairs (names lowercased).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl InboundHttpRequest {
    /// First value of a header (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == lower)
            .map(|(_, v)| v.as_str())
    }

    /// First value of a query parameter.
    #[must_use]
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            if k == name {
                Some(v)
            } else {
                None
            }
        })
    }
}

/// Verifier verdict.
#[derive(Debug, Clone)]
pub enum VerifyOutcome {
    /// Trust established.
    Ok,
    /// Challenge verifier: respond with this echo body and 200 (GET verify flows).
    ChallengeEcho(String),
    /// Trust rejected — respond with status + reason, never 2xx.
    Reject { status: u16, reason: String },
}

fn reject(status: u16, reason: &str) -> VerifyOutcome {
    VerifyOutcome::Reject {
        status,
        reason: reason.to_string(),
    }
}

/// Constant-time byte-slice equality (no `unsafe`).
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Unseal the channel credentials JSON ({} when none stored).
fn credentials_json(channel: &ItgChannel, vault: Option<&Vault>) -> Result<Value, VerifyOutcome> {
    let Some(sealed) = channel.credentials.as_deref() else {
        return Ok(Value::Object(serde_json::Map::new()));
    };
    let Some(vault) = vault else {
        return Err(reject(
            503,
            "credentials present but vault sealed (set INTEGRATION_VAULT_KEY)",
        ));
    };
    match vault.unseal(sealed) {
        Ok(json) => serde_json::from_str(&json).map_err(|_| reject(401, "corrupted credentials")),
        Err(_) => Err(reject(401, "credentials unseal failed (rotated key?)")),
    }
}

fn cred_str<'a>(creds: &'a Value, key: &str) -> Option<&'a str> {
    creds.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn cfg_str(config: &Value, key: &str, default: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn cfg_u64(config: &Value, key: &str, default: u64) -> u64 {
    config.get(key).and_then(Value::as_u64).unwrap_or(default)
}

/// Dispatch by `verify_kind`.
pub fn verify(channel: &ItgChannel, vault: Option<&Vault>, req: &InboundHttpRequest) -> VerifyOutcome {
    let config = channel.verify_config.clone().unwrap_or(Value::Null);
    match channel.verify_kind.as_str() {
        "hmac-sha256" => verify_hmac(&config, channel, vault, req),
        "token" => verify_token(&config, channel, vault, req),
        "challenge" => verify_challenge(&config, req),
        "none" => VerifyOutcome::Ok,
        other => reject(500, &format!("unsupported verify_kind '{other}'")),
    }
}

/// `hmac-sha256`: HMAC-SHA256 over the raw body, compared constant-time.
///
/// Config: `header` (default `x-signature`), `scheme` (default `sha256=`,
/// empty = raw hex), optional `timestamp_header` + `window_secs` (default 300)
/// for replay protection.
fn verify_hmac(
    config: &Value,
    channel: &ItgChannel,
    vault: Option<&Vault>,
    req: &InboundHttpRequest,
) -> VerifyOutcome {
    use hmac::{Hmac, KeyInit, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;

    let creds = match credentials_json(channel, vault) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let Some(secret) = cred_str(&creds, "secret") else {
        return reject(500, "hmac-sha256 requires credentials {\"secret\": ...}");
    };

    // Optional replay window check.
    if let Some(ts_header) = config.get("timestamp_header").and_then(Value::as_str) {
        let Some(ts_str) = req.header(ts_header) else {
            return reject(401, "missing timestamp header");
        };
        let Ok(ts) = ts_str.parse::<i64>() else {
            return reject(401, "malformed timestamp");
        };
        let window = cfg_u64(config, "window_secs", 300) as i64;
        let now = crate::utils::tz::now_utc().timestamp();
        if (now - ts).abs() > window {
            return reject(401, "timestamp outside allowed window");
        }
    }

    let Some(sig) = req.header(&cfg_str(config, "header", "x-signature")) else {
        return reject(401, "missing signature header");
    };
    let scheme = cfg_str(config, "scheme", "sha256=");
    let sig_hex = if scheme.is_empty() {
        sig
    } else if let Some(stripped) = sig.strip_prefix(&scheme) {
        stripped
    } else {
        return reject(401, "signature scheme mismatch");
    };

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| reject(500, "hmac init"))
        .unwrap_or_else(|_| unreachable!("HMAC accepts any key size"));
    mac.update(&req.body);
    match hex::decode(sig_hex) {
        Ok(expected) => {
            // `verify_slice` is constant-time.
            if mac.verify_slice(&expected).is_ok() {
                VerifyOutcome::Ok
            } else {
                reject(401, "signature mismatch")
            }
        }
        Err(_) => reject(401, "signature not valid hex"),
    }
}

/// `token`: static bearer compare (constant-time) from header or query param.
///
/// Config: `header` (default `x-ingress-token`), `query_param` (optional
/// fallback, e.g. `token`).
fn verify_token(
    config: &Value,
    channel: &ItgChannel,
    vault: Option<&Vault>,
    req: &InboundHttpRequest,
) -> VerifyOutcome {
    let creds = match credentials_json(channel, vault) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let Some(expected) = cred_str(&creds, "token") else {
        return reject(500, "token verify requires credentials {\"token\": ...}");
    };
    let header = cfg_str(config, "header", "x-ingress-token");
    let provided = req
        .header(&header)
        .or_else(|| {
            config
                .get("query_param")
                .and_then(Value::as_str)
                .and_then(|p| req.query_param(p))
        })
        .unwrap_or("");
    if !expected.is_empty() && constant_eq(provided.as_bytes(), expected.as_bytes()) {
        VerifyOutcome::Ok
    } else {
        reject(401, "invalid token")
    }
}

/// `challenge`: GET echo verification (WeChat server check style).
///
/// Config: `echo_param` (default `echostr`). POST requests pass through as Ok
/// (the challenge only guards the GET handshake).
fn verify_challenge(config: &Value, req: &InboundHttpRequest) -> VerifyOutcome {
    if req.method != "GET" {
        return VerifyOutcome::Ok;
    }
    let echo_param = cfg_str(config, "echo_param", "echostr");
    match req.query_param(&echo_param) {
        Some(echo) => VerifyOutcome::ChallengeEcho(echo.to_string()),
        None => reject(400, "missing echo parameter"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration::channel::ItgChannel;
    use crate::utils::id::new_snowflake_id;

    fn channel(verify_kind: &str, config: Value, credentials: Option<String>) -> ItgChannel {
        ItgChannel {
            id: new_snowflake_id(),
            tenant_id: "default".into(),
            channel_key: "test".into(),
            provider: "generic-hmac".into(),
            display_name: "t".into(),
            mode: "push".into(),
            transport: "http1".into(),
            framing: "raw".into(),
            codec: "json".into(),
            endpoint: None,
            verify_kind: verify_kind.into(),
            verify_config: Some(config),
            credentials,
            mapping: None,
            normalizer_plugin: None,
            pull_semantics: None,
            pull_config: None,
            ack_kind: "http-200".into(),
            redelivery_max: 5,
            backpressure: None,
            target_type: "t".into(),
            route_extra: None,
            status: "idle".into(),
            last_error: None,
            lease_owner: None,
            enabled: true,
            version: 1,
            shadow: false,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn request(body: &[u8], headers: &[(&str, &str)]) -> InboundHttpRequest {
        InboundHttpRequest {
            method: "POST".into(),
            query: String::new(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                .collect(),
            body: body.to_vec(),
        }
    }

    fn sign(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, KeyInit, Mac};
        type HmacSha256 = Hmac<sha2::Sha256>;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .unwrap_or_else(|_| panic!("hmac init"));
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn hmac_accepts_valid_signature() {
        let vault = Vault::from_secret("test-key").expect("vault");
        let creds = vault.seal(r#"{"secret":"s3cret"}"#).expect("seal");
        let ch = channel("hmac-sha256", serde_json::json!({}), Some(creds));
        let body = br#"{"id":1}"#;
        let sig = sign("s3cret", body);
        let req = request(body, &[("x-signature", &sig)]);
        assert!(matches!(verify(&ch, Some(&vault), &req), VerifyOutcome::Ok));
    }

    #[test]
    fn hmac_rejects_wrong_secret_and_body_tamper() {
        let vault = Vault::from_secret("test-key").expect("vault");
        let creds = vault.seal(r#"{"secret":"s3cret"}"#).expect("seal");
        let ch = channel("hmac-sha256", serde_json::json!({}), Some(creds));
        let body = br#"{"id":1}"#;

        let sig = sign("wrong", body);
        let req = request(body, &[("x-signature", &sig)]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req),
            VerifyOutcome::Reject { .. }
        ));

        let sig = sign("s3cret", br#"{"id":2}"#);
        let req = request(body, &[("x-signature", &sig)]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req),
            VerifyOutcome::Reject { .. }
        ));
    }

    #[test]
    fn hmac_rejects_stale_timestamp() {
        let vault = Vault::from_secret("test-key").expect("vault");
        let creds = vault.seal(r#"{"secret":"s"}"#).expect("seal");
        let ch = channel(
            "hmac-sha256",
            serde_json::json!({"timestamp_header": "x-ts"}),
            Some(creds),
        );
        let stale = (crate::utils::tz::now_utc().timestamp() - 3600).to_string();
        let sig = sign("s", b"{}");
        let req = request(b"{}", &[("x-signature", &sig), ("x-ts", &stale)]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req),
            VerifyOutcome::Reject { status: 401, .. }
        ));
    }

    #[test]
    fn sealed_vault_rejects_with_503() {
        let vault = Vault::from_secret("test-key").expect("vault");
        let creds = vault.seal(r#"{"secret":"s"}"#).expect("seal");
        let ch = channel("hmac-sha256", Value::Null, Some(creds));
        let req = request(b"{}", &[]);
        assert!(matches!(
            verify(&ch, None, &req),
            VerifyOutcome::Reject { status: 503, .. }
        ));
    }

    #[test]
    fn token_via_header_or_query() {
        let vault = Vault::from_secret("k").expect("vault");
        let creds = vault.seal(r#"{"token":"tok1"}"#).expect("seal");
        let ch = channel("token", serde_json::json!({"query_param": "token"}), Some(creds));

        let req = request(b"{}", &[("x-ingress-token", "tok1")]);
        assert!(matches!(verify(&ch, Some(&vault), &req), VerifyOutcome::Ok));

        let mut req = request(b"{}", &[]);
        req.query = "token=tok1".into();
        assert!(matches!(verify(&ch, Some(&vault), &req), VerifyOutcome::Ok));

        let req = request(b"{}", &[("x-ingress-token", "tok2")]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req),
            VerifyOutcome::Reject { .. }
        ));
    }

    #[test]
    fn challenge_echoes_get_param() {
        let ch = channel("challenge", Value::Null, None);
        let mut req = request(b"", &[]);
        req.method = "GET".into();
        req.query = "echostr=abc123".into();
        match verify(&ch, None, &req) {
            VerifyOutcome::ChallengeEcho(echo) => assert_eq!(echo, "abc123"),
            other => panic!("expected echo, got {other:?}"),
        }
    }
}
