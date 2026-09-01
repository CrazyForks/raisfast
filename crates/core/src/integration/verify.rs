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
            if k == name { Some(v) } else { None }
        })
    }
}

/// Verifier verdict.
#[derive(Debug, Clone)]
pub enum VerifyOutcome {
    /// Trust established.
    Ok,
    /// Trust established AND the body was transformed by the verifier
    /// (e.g. wechat-aes decryption) — the pipeline must use this body.
    OkDecrypted(Vec<u8>),
    /// Widget session token verified: the caller is `contact_id` on
    /// `channel_key`. The pipeline injects `sender` + `_session` so mappings
    /// (and downstream chat.ingress) can attribute the message.
    WidgetSession {
        contact_id: String,
        channel_key: String,
    },
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
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
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
    creds
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
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
///
/// `jwt_secret` is the platform secret used to sign short-session widget
/// tokens (verify `jwt-widget`); it is `None` for channels that don't need it.
pub fn verify(
    channel: &ItgChannel,
    vault: Option<&Vault>,
    req: &InboundHttpRequest,
    jwt_secret: Option<&str>,
) -> VerifyOutcome {
    let config = channel.verify_config.clone().unwrap_or(Value::Null);
    match channel.verify_kind.as_str() {
        "hmac-sha256" => verify_hmac(&config, channel, vault, req),
        "token" => verify_token(&config, channel, vault, req),
        "challenge" => verify_challenge(&config, req),
        "wechat-aes" => verify_wechat_aes(&config, channel, vault, req),
        "jwt-widget" => verify_jwt_widget(&config, channel, req, jwt_secret),
        "none" => VerifyOutcome::Ok,
        other => reject(500, &format!("unsupported verify_kind '{other}'")),
    }
}

/// `jwt-widget`: platform-issued short-session JWT (`Bearer <token>`).
///
/// Verifies signature (platform `jwt_secret`), `exp`, `typ == "widget"` and
/// that `claims.ch` equals the channel key — so a token minted for one widget
/// channel can never submit to another. On success the claims (contact_id,
/// channel_key) are returned for `sender`/`_session` injection.
fn verify_jwt_widget(
    _config: &Value,
    channel: &ItgChannel,
    req: &InboundHttpRequest,
    jwt_secret: Option<&str>,
) -> VerifyOutcome {
    let Some(secret) = jwt_secret else {
        return reject(500, "jwt-widget verify requires platform JWT_SECRET");
    };
    let Some(auth) = req.header("authorization") else {
        return reject(401, "missing Authorization header");
    };
    let Some(token) = crate::utils::widget_token::bearer_token(auth) else {
        return reject(401, "expected 'Bearer <token>'");
    };
    match crate::utils::widget_token::verify_widget_token(secret, token) {
        Some(claims) if claims.ch == channel.channel_key => VerifyOutcome::WidgetSession {
            contact_id: claims.sub,
            channel_key: claims.ch,
        },
        Some(_) => reject(403, "widget token channel mismatch"),
        None => reject(401, "invalid or expired widget token"),
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
    use base64::Engine;
    let expected: Result<Vec<u8>, String> = match cfg_str(config, "encoding", "hex").as_str() {
        "hex" => hex::decode(sig_hex).map_err(|e| e.to_string()),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(sig_hex)
            .map_err(|e| e.to_string()),
        "base64url" => base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig_hex)
            .map_err(|e| e.to_string()),
        other => return reject(500, &format!("unsupported encoding '{other}'")),
    };
    match expected {
        Ok(expected) => {
            // `verify_slice` is constant-time.
            if mac.verify_slice(&expected).is_ok() {
                VerifyOutcome::Ok
            } else {
                reject(401, "signature mismatch")
            }
        }
        Err(_) => reject(401, "signature not valid for the configured encoding"),
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

/// `wechat-aes`: WeChat Work / Official Account encrypted callbacks
/// (WXBizMsgCrypt). Signature = SHA1 over the sorted
/// (token, timestamp, nonce, encrypt) tuple; the payload is AES-256-CBC
/// sealed with the EncodingAESKey. Decrypted plaintext replaces the body.
///
/// Config: none needed beyond `query_param` defaults; credentials carry
/// `{"token": "...", "encoding_aes_key": "..."}`.
///
/// GET = URL verification (echo the decrypted `echostr`); POST = event.
fn verify_wechat_aes(
    _config: &Value,
    channel: &ItgChannel,
    vault: Option<&Vault>,
    req: &InboundHttpRequest,
) -> VerifyOutcome {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

    let creds = match credentials_json(channel, vault) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let (Some(token), Some(aes_key_b64)) = (
        cred_str(&creds, "token"),
        cred_str(&creds, "encoding_aes_key"),
    ) else {
        return reject(
            500,
            "wechat-aes requires credentials {\"token\": ..., \"encoding_aes_key\": ...}",
        );
    };

    // Decode the EncodingAESKey (43 chars + "=" → 32 bytes).
    let aes_key = {
        use base64::Engine;
        match base64::engine::general_purpose::STANDARD.decode(format!("{aes_key_b64}=")) {
            Ok(k) if k.len() == 32 => k,
            _ => return reject(500, "invalid encoding_aes_key"),
        }
    };
    let iv = &aes_key[..16];

    let decrypt = |cipher_text_b64: &str| -> Result<Vec<u8>, String> {
        use base64::Engine;
        let ct = base64::engine::general_purpose::STANDARD
            .decode(cipher_text_b64)
            .map_err(|e| format!("encrypt field not base64: {e}"))?;
        if ct.is_empty() || ct.len() % 16 != 0 {
            return Err("cipher length not AES block aligned".into());
        }
        let key_arr: &[u8; 32] = aes_key
            .as_slice()
            .try_into()
            .map_err(|_| "key".to_string())?;
        let iv_arr: &[u8; 16] = iv.try_into().map_err(|_| "iv".to_string())?;
        let plain = Aes256CbcDec::new(key_arr.into(), iv_arr.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&ct)
            .map_err(|e| format!("aes decrypt: {e}"))?;
        // 16 random bytes | 4-byte BE msg_len | msg | receive_id
        if plain.len() < 20 {
            return Err("plaintext too short".into());
        }
        let msg_len = u32::from_be_bytes([plain[16], plain[17], plain[18], plain[19]]) as usize;
        if plain.len() < 20 + msg_len {
            return Err("plaintext length mismatch (wrong key?)".into());
        }
        Ok(plain[20..20 + msg_len].to_vec())
    };
    let check_signature = |encrypt: &str, sig: &str, ts: &str, nonce: &str| -> bool {
        let mut parts = [
            token.to_string(),
            ts.to_string(),
            nonce.to_string(),
            encrypt.to_string(),
        ];
        parts.sort();
        let digest = {
            use sha1::Digest;
            let mut h = sha1::Sha1::new();
            h.update(parts.join("").as_bytes());
            hex::encode(h.finalize())
        };
        constant_eq(digest.as_bytes(), sig.as_bytes())
    };

    let sig = req.query_param("msg_signature").unwrap_or_default();
    let ts = req.query_param("timestamp").unwrap_or_default();
    let nonce = req.query_param("nonce").unwrap_or_default();

    if req.method == "GET" {
        // URL verification: decrypt the echostr and echo the plaintext.
        let Some(echo_cipher) = req.query_param("echostr") else {
            return reject(400, "missing echostr");
        };
        if !check_signature(echo_cipher, sig, ts, nonce) {
            return reject(401, "msg_signature mismatch");
        }
        return match decrypt(echo_cipher) {
            Ok(plain) => VerifyOutcome::ChallengeEcho(String::from_utf8_lossy(&plain).to_string()),
            Err(e) => reject(400, &format!("echostr decrypt: {e}")),
        };
    }

    // POST: body is XML (or JSON) carrying {"Encrypt": ...} — extract without
    // a full XML dependency.
    let body = String::from_utf8_lossy(&req.body);
    let encrypt = extract_field(&body, "Encrypt")
        .or_else(|| extract_field(&body, "encrypt"))
        .unwrap_or_default();
    if encrypt.is_empty() {
        return reject(400, "body has no Encrypt field");
    }
    if !check_signature(&encrypt, sig, ts, nonce) {
        return reject(401, "msg_signature mismatch");
    }
    match decrypt(&encrypt) {
        Ok(plain) => VerifyOutcome::OkDecrypted(plain),
        Err(e) => reject(400, &format!("decrypt: {e}")),
    }
}

/// Pull a `<Field>value</Field>`-style element out of an XML-ish body
/// (good enough for the single-field envelopes these gateways send).
fn extract_field(body: &str, field: &str) -> Option<String> {
    let open = format!("<{field}>");
    let close = format!("</{field}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    Some(body[start..end].to_string())
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
            app_id: None,
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
            stream_config: None,
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
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).unwrap_or_else(|_| panic!("hmac init"));
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
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Ok
        ));
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
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Reject { .. }
        ));

        let sig = sign("s3cret", br#"{"id":2}"#);
        let req = request(body, &[("x-signature", &sig)]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
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
            verify(&ch, Some(&vault), &req, None),
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
            verify(&ch, None, &req, None),
            VerifyOutcome::Reject { status: 503, .. }
        ));
    }

    #[test]
    fn token_via_header_or_query() {
        let vault = Vault::from_secret("k").expect("vault");
        let creds = vault.seal(r#"{"token":"tok1"}"#).expect("seal");
        let ch = channel(
            "token",
            serde_json::json!({"query_param": "token"}),
            Some(creds),
        );

        let req = request(b"{}", &[("x-ingress-token", "tok1")]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Ok
        ));

        let mut req = request(b"{}", &[]);
        req.query = "token=tok1".into();
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Ok
        ));

        let req = request(b"{}", &[("x-ingress-token", "tok2")]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Reject { .. }
        ));
    }

    /// Test-side WXBizMsgCrypt encryptor: 16 random | BE len | msg | id.
    fn wechat_encrypt(aes_key: &[u8; 32], msg: &str) -> String {
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        use base64::Engine;
        type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
        let mut plain = vec![0u8; 16];
        plain.extend_from_slice(&(msg.len() as u32).to_be_bytes());
        plain.extend_from_slice(msg.as_bytes());
        plain.extend_from_slice(b"corpid_test");
        let ct = Aes256CbcEnc::new(aes_key.into(), aes_key[..16].into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        base64::engine::general_purpose::STANDARD.encode(ct)
    }

    fn wechat_sign(token: &str, ts: &str, nonce: &str, encrypt: &str) -> String {
        use sha1::Digest;
        let mut parts = [token, ts, nonce, encrypt];
        parts.sort();
        let mut h = sha1::Sha1::new();
        h.update(parts.join("").as_bytes());
        hex::encode(h.finalize())
    }

    // Base64("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=") → bytes 0..32.
    const WECHAT_AES_KEY_43: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    const WECHAT_KEY: &[u8; 32] = &[
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];

    fn wechat_channel(vault: &Vault) -> ItgChannel {
        let creds = vault
            .seal(&format!(
                r#"{{"token":"tok123","encoding_aes_key":"{WECHAT_AES_KEY_43}"}}"#
            ))
            .expect("seal");
        channel("wechat-aes", Value::Null, Some(creds))
    }

    #[test]
    fn hmac_base64_encoding_ok() {
        let vault = Vault::from_secret("test-key").expect("vault");
        let creds = vault.seal(r#"{"secret":"s3cret"}"#).expect("seal");
        let ch = channel(
            "hmac-sha256",
            serde_json::json!({"encoding": "base64", "scheme": ""}),
            Some(creds),
        );
        use base64::Engine;
        use hmac::{Hmac, KeyInit, Mac};
        type HmacSha256 = Hmac<sha2::Sha256>;
        let mut mac = HmacSha256::new_from_slice(b"s3cret").unwrap();
        mac.update(b"{\"shopify\":true}");
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let req = request(b"{\"shopify\":true}", &[("x-signature", sig.as_str())]);
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Ok
        ));
    }

    #[test]
    fn wechat_aes_post_decrypts_and_get_echoes() {
        let vault = Vault::from_secret("k").expect("vault");
        let ch = wechat_channel(&vault);

        // POST: encrypted event → OkDecrypted(plaintext XML/JSON).
        let plain_msg = r#"{"event":"message","content":"你好"}"#;
        let encrypt = wechat_encrypt(WECHAT_KEY, plain_msg);
        let sig = wechat_sign("tok123", "1700000000", "n1", &encrypt);
        let body = format!(r#"<xml><Encrypt>{encrypt}</Encrypt></xml>"#);
        let mut req = request(body.as_bytes(), &[]);
        req.query = format!("msg_signature={sig}&timestamp=1700000000&nonce=n1");
        match verify(&ch, Some(&vault), &req, None) {
            VerifyOutcome::OkDecrypted(plain) => {
                assert_eq!(String::from_utf8_lossy(&plain), plain_msg);
            }
            other => panic!("expected OkDecrypted, got {other:?}"),
        }

        // Tampered signature → reject.
        let mut req = request(body.as_bytes(), &[]);
        req.query = "msg_signature=deadbeef&timestamp=1700000000&nonce=n1".to_string();
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Reject { status: 401, .. }
        ));

        // GET: echostr decrypt-then-echo.
        let echo_plain = "RANDOM-ECHO-8421";
        let echo_cipher = wechat_encrypt(WECHAT_KEY, echo_plain);
        let sig = wechat_sign("tok123", "1700000000", "n1", &echo_cipher);
        let mut req = request(b"", &[]);
        req.method = "GET".into();
        req.query =
            format!("msg_signature={sig}&timestamp=1700000000&nonce=n1&echostr={echo_cipher}");
        match verify(&ch, Some(&vault), &req, None) {
            VerifyOutcome::ChallengeEcho(echo) => assert_eq!(echo, echo_plain),
            other => panic!("expected echo, got {other:?}"),
        }

        // Wrong key → length mismatch error (no panic, clean 400).
        let bad_cipher = wechat_encrypt(b"00000000000000000000000000000000", plain_msg);
        let sig = wechat_sign("tok123", "1700000000", "n1", &bad_cipher);
        let body = format!(r#"<xml><Encrypt>{bad_cipher}</Encrypt></xml>"#);
        let mut req = request(body.as_bytes(), &[]);
        req.query = format!("msg_signature={sig}&timestamp=1700000000&nonce=n1");
        assert!(matches!(
            verify(&ch, Some(&vault), &req, None),
            VerifyOutcome::Reject { status: 400, .. }
        ));
    }

    #[test]
    fn challenge_echoes_get_param() {
        let ch = channel("challenge", Value::Null, None);
        let mut req = request(b"", &[]);
        req.method = "GET".into();
        req.query = "echostr=abc123".into();
        match verify(&ch, None, &req, None) {
            VerifyOutcome::ChallengeEcho(echo) => assert_eq!(echo, "abc123"),
            other => panic!("expected echo, got {other:?}"),
        }
    }

    #[test]
    fn jwt_widget_accepts_valid_channel_scoped_token() {
        let secret = "platform-secret-32bytes-at-least";
        let ch = channel("jwt-widget", Value::Null, None);
        let tok = crate::utils::widget_token::issue_widget_token(secret, "test", "12345", 3600)
            .expect("issue");
        let req = request(b"{}", &[("authorization", &format!("Bearer {tok}"))]);
        match verify(&ch, None, &req, Some(secret)) {
            VerifyOutcome::WidgetSession {
                contact_id,
                channel_key,
            } => {
                assert_eq!(contact_id, "12345");
                assert_eq!(channel_key, "test");
            }
            other => panic!("expected WidgetSession, got {other:?}"),
        }
    }

    #[test]
    fn jwt_widget_rejects_wrong_channel_and_bad_secret() {
        let secret = "platform-secret-32bytes-at-least";
        let ch = channel("jwt-widget", Value::Null, None);
        let tok =
            crate::utils::widget_token::issue_widget_token(secret, "chat-other", "12345", 3600)
                .expect("issue");
        let req = request(b"{}", &[("authorization", &format!("Bearer {tok}"))]);
        assert!(matches!(
            verify(&ch, None, &req, Some(secret)),
            VerifyOutcome::Reject { status: 403, .. }
        ));

        let tok2 =
            crate::utils::widget_token::issue_widget_token(secret, "chat-widget", "12345", 3600)
                .expect("issue");
        let req2 = request(b"{}", &[("authorization", &format!("Bearer {tok2}"))]);
        assert!(matches!(
            verify(&ch, None, &req2, Some("different-secret")),
            VerifyOutcome::Reject { status: 401, .. }
        ));
    }
}
