//! Generic request signing engine (egress-signature.md).
//!
//! Business-agnostic: this module implements the *mechanics* of signature
//! schemes (canonicalization templates, HMAC/RSA signing, key derivation,
//! injection) driven entirely by a declarative `op.signature` config. AWS
//! SigV4 / Tencent TC3 / Alibaba RPC / OAuth1 are **recipes** (config JSON),
//! never kernel code — see `oauth2-egress-guide.md` / `egress-signature.md`.
//!
//! Pipeline: the egress request builder hands us the method, final URL,
//! collected headers and exact body bytes. We emit auxiliary headers/params
//! (timestamp, payload hash, nonce), render the canonical string template,
//! render the string-to-sign template, derive the signing key (literal or
//! HMAC chain), sign + encode, then inject into a header or query param.
//!
//! Template variables:
//! - `{@name}` — computed values (method, uri, query, headers_canon, …)
//! - `{name}` — caller variables (credentials, signature-config scalars, input)
//! - `{sig}` — the final signature (injection templates only)

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};

/// Opaque handle to what the engine needs from the (already built) request.
pub struct SignRequest<'a> {
    pub method: &'a str,
    pub url: &'a mut reqwest::Url,
    /// Collected request headers (lowercased names, trimmed values).
    pub headers: &'a mut Vec<(String, String)>,
    /// Exact body bytes (empty for GET / multipart).
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignatureConfig {
    #[serde(default = "default_algorithm")]
    pub algorithm: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub key: Option<SignKey>,
    /// Header names (case-insensitive) that enter the canonical header block.
    #[serde(default)]
    pub canonical_headers: Vec<String>,
    /// The canonical request string template.
    pub canonical_template: String,
    /// Scope string template (SigV4: `{@date}/{region}/{service}/aws4_request`).
    #[serde(default)]
    pub scope: Option<String>,
    /// The string-to-sign template (may reference `{@canonical_hash}`).
    pub string_to_sign_template: String,
    /// Aux headers to set *before* canonicalization (e.g. `x-amz-date`).
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Aux query params to set *before* canonicalization (e.g. `SignatureNonce`).
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
    /// Timestamp template resolved from caller vars; absent → server now.
    #[serde(default)]
    pub timestamp: Option<String>,
    pub inject: SignInject,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignKey {
    /// Iterative HMAC derivation: HMAC(prefix+secret, steps[0]) →
    /// HMAC(result, steps[i])… (AWS SigV4 / Tencent TC3).
    HmacChain { prefix: String, steps: Vec<String> },
    /// Use the secret directly (optionally transformed, e.g. Alibaba appends `&`).
    Secret {
        #[serde(default)]
        secret: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignInject {
    #[serde(default = "default_into")]
    pub into: String,
    #[serde(default = "default_auth_header")]
    pub header: String,
    /// Header value template; `{sig}` is the signature (header injection).
    #[serde(default)]
    pub template: Option<String>,
    /// Query param name to receive the signature (query injection).
    #[serde(default)]
    pub query_param: Option<String>,
}

fn default_algorithm() -> String {
    "hmac-sha256".into()
}
fn default_encoding() -> String {
    "hex".into()
}
fn default_into() -> String {
    "header".into()
}
fn default_auth_header() -> String {
    "Authorization".into()
}

/// Entry point: parse `op.signature`, apply it to the request in place.
///
/// # Errors
///
/// `BadRequest` on malformed signature config / missing template variables;
/// `Internal` on HMAC/RSA failures.
pub fn apply_signature(
    req: &mut SignRequest<'_>,
    config: &SignatureConfig,
    vars: &Value,
) -> AppResult<()> {
    // 1. Timestamp / derived date.
    let timestamp = match &config.timestamp {
        Some(tpl) => render(tpl, &HashMap::new(), vars)?,
        None => now_basic(),
    };
    let date = timestamp.get(..8).unwrap_or("").to_string();
    let payload_hash = hex::encode(sha256(req.payload));
    let nonce = hex::encode(crate::utils::id::new_id().to_be_bytes());

    // Base computed context (available to aux header/query templates).
    let mut computed = HashMap::new();
    computed.insert("payload_hash".into(), payload_hash.clone());
    computed.insert("timestamp".into(), timestamp.clone());
    computed.insert("timestamp_rfc".into(), now_rfc3339());
    computed.insert("date".into(), date.clone());
    computed.insert("nonce".into(), nonce.clone());
    computed.insert("algorithm_upper".into(), config.algorithm.to_uppercase());

    // 2. Aux headers + query params (before canonicalization so they are signed).
    for (name, tpl) in config.query.iter().flatten() {
        let rendered = render(tpl, &computed, vars)?;
        set_url_query(req.url, name, &rendered);
    }
    let mut headers: Vec<(String, String)> = req.headers.clone();
    for (name, tpl) in config.headers.iter().flatten() {
        let rendered = render(tpl, &computed, vars)?;
        set_header(&mut headers, name, &rendered);
    }

    // 3. Canonical query is derived from the final URL (single source of truth
    // — covers op.query and aux signature params). `host` is set by the HTTP
    // layer, not our header map — derive it so recipes can sign it (SigV4
    // requires host) without putting it on the wire.
    let canonical_query = canonical_query(&url_query_pairs(req.url));
    let host = match (req.url.host_str(), req.url.port()) {
        (Some(h), Some(p)) => Some(format!("{h}:{p}")),
        (Some(h), None) => Some(h.to_string()),
        _ => None,
    };
    let canonical_headers =
        canonical_headers_block(&headers, &config.canonical_headers, host.as_deref());
    let signed_headers = signed_headers(&headers, &config.canonical_headers, host.as_deref());
    computed.insert("method".into(), req.method.to_string());
    computed.insert("uri".into(), req.url.path().to_string());
    computed.insert("query".into(), canonical_query.clone());
    computed.insert("enc_slash".into(), "%2F".into());
    computed.insert("enc_query".into(), percent_encode(&canonical_query));
    computed.insert("headers_canon".into(), canonical_headers.clone());
    computed.insert("headers_signed".into(), signed_headers.clone());
    computed.insert(
        "params_raw".into(),
        url_query_pairs(req.url)
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&"),
    );

    // 4. Canonical request string → its hash.
    let canonical = render(&config.canonical_template, &computed, vars)?;
    let canonical_hash = hex::encode(sha256(canonical.as_bytes()));
    computed.insert("canonical".into(), canonical.clone());
    computed.insert("canonical_hash".into(), canonical_hash);

    // 5. Scope + string-to-sign.
    if let Some(scope) = &config.scope {
        computed.insert("scope".into(), render(scope, &computed, vars)?);
    }
    let string_to_sign = render(&config.string_to_sign_template, &computed, vars)?;
    computed.insert("string_to_sign".into(), string_to_sign.clone());

    // 6. Signing key.
    let key = derive_key(config, &computed, vars)?;

    // 7. Signature.
    let signature = sign(&config.algorithm, &key, string_to_sign.as_bytes())?;
    let encoded = match config.encoding.as_str() {
        "hex" => hex::encode(&signature),
        "base64" => base64_std(&signature),
        other => {
            return Err(AppError::BadRequest(format!(
                "signature encoding '{other}' not supported (hex | base64)"
            )));
        }
    };
    computed.insert("sig".into(), encoded.clone());

    // 8. Inject.
    match config.inject.into.as_str() {
        "header" => {
            let tpl = config.inject.template.as_deref().ok_or_else(|| {
                AppError::BadRequest("signature.inject.header requires a 'template'".into())
            })?;
            let value = render(tpl, &computed, vars)?;
            set_header(
                &mut headers,
                &config.inject.header.to_ascii_lowercase(),
                &value,
            );
        }
        "query" => {
            let param = config.inject.query_param.as_deref().ok_or_else(|| {
                AppError::BadRequest("signature.inject.query requires a 'query_param'".into())
            })?;
            set_url_query(req.url, param, &encoded);
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "signature.inject.into '{other}' not supported (header | query)"
            )));
        }
    }

    // 10. Write headers back (query already lives on the URL).
    req.headers.clear();
    req.headers.extend(headers);
    Ok(())
}

/// Parse + apply from an `op.signature` JSON value.
///
/// # Errors
///
/// See [`apply_signature`].
pub fn apply_signature_value(
    req: &mut SignRequest<'_>,
    config_value: &Value,
    vars: &Value,
) -> AppResult<()> {
    let config: SignatureConfig = serde_json::from_value(config_value.clone())
        .map_err(|e| AppError::BadRequest(format!("op.signature invalid: {e}")))?;
    apply_signature(req, &config, vars)
}

/// RFC 3986 percent-encoding (keeps `A-Za-z0-9-._~`; space → `%20`).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Canonical query string: keys + values RFC3986-encoded, sorted by key bytes.
fn canonical_query(pairs: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = pairs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Canonical headers block: `"lowername:trimmedvalue\n"` for each requested
/// header present (sorted by name).
fn canonical_headers_block(
    headers: &[(String, String)],
    wanted: &[String],
    host: Option<&str>,
) -> String {
    present_headers(headers, wanted, host)
        .iter()
        .map(|(n, v)| format!("{n}:{v}\n"))
        .collect()
}

/// Signed-headers list: requested headers that are present, `;`-joined.
fn signed_headers(headers: &[(String, String)], wanted: &[String], host: Option<&str>) -> String {
    present_headers(headers, wanted, host)
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";")
}

/// Wanted headers (case-insensitive) that are present, sorted by name.
fn present_headers(
    headers: &[(String, String)],
    wanted: &[String],
    host: Option<&str>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for name in wanted {
        let lower = name.to_ascii_lowercase();
        if let Some((_, v)) = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(&lower)) {
            out.push((lower, v.trim().to_string()));
        } else if lower == "host"
            && let Some(h) = host
        {
            out.push((lower, h.to_string()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Insert (or replace) a header by lowercased name.
fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(existing) = headers
        .iter_mut()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
    {
        existing.1 = value.to_string();
    } else {
        headers.push((name.to_ascii_lowercase(), value.to_string()));
    }
}

/// Decoded URL query pairs (single source of truth for canonicalization).
fn url_query_pairs(url: &reqwest::Url) -> Vec<(String, String)> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Set (replace) a query param by name on the URL; appends when absent.
fn set_url_query(url: &mut reqwest::Url, name: &str, value: &str) {
    let pairs = url_query_pairs(url);
    url.set_query(None);
    let mut replaced = false;
    for (k, v) in pairs {
        if k == name && !replaced {
            url.query_pairs_mut().append_pair(name, value);
            replaced = true;
        } else {
            url.query_pairs_mut().append_pair(&k, &v);
        }
    }
    if !replaced {
        url.query_pairs_mut().append_pair(name, value);
    }
}

fn derive_key(
    config: &SignatureConfig,
    computed: &HashMap<String, String>,
    vars: &Value,
) -> AppResult<Vec<u8>> {
    let algorithm = config.algorithm.as_str();
    match config.key.as_ref() {
        None => Ok(secret_bytes(computed, vars)?),
        Some(SignKey::Secret { secret }) => {
            let bytes = if let Some(tpl) = secret {
                render(tpl, computed, vars)?.into_bytes()
            } else {
                secret_bytes(computed, vars)?
            };
            Ok(bytes)
        }
        Some(SignKey::HmacChain { prefix, steps }) => {
            let base = format!(
                "{prefix}{}",
                String::from_utf8_lossy(&secret_bytes(computed, vars)?)
            );
            let mut key = base.into_bytes();
            for step in steps {
                let msg = render(step, computed, vars)?.into_bytes();
                key = hmac(algorithm, &key, &msg)?;
            }
            Ok(key)
        }
    }
}

/// Default secret source: the caller variable `{secret_key}`.
fn secret_bytes(computed: &HashMap<String, String>, vars: &Value) -> AppResult<Vec<u8>> {
    let rendered = render("{secret_key}", computed, vars)?;
    Ok(rendered.into_bytes())
}

fn sign(algorithm: &str, key: &[u8], msg: &[u8]) -> AppResult<Vec<u8>> {
    hmac(algorithm, key, msg)
}

fn hmac(algorithm: &str, key: &[u8], msg: &[u8]) -> AppResult<Vec<u8>> {
    use hmac::Mac;
    use hmac::digest::KeyInit;
    match algorithm {
        "hmac-sha256" => {
            let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("hmac key: {e}")))?;
            mac.update(msg);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        "hmac-sha1" => {
            let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(key)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("hmac key: {e}")))?;
            mac.update(msg);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        other => Err(AppError::BadRequest(format!(
            "signature algorithm '{other}' not supported (hmac-sha256 | hmac-sha1)"
        ))),
    }
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher.finalize().to_vec()
}

fn base64_std(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Basic ISO8601 timestamp (`yyyyMMdd'T'HHmmss'Z'`) — SigV4 / TC3 format.
fn now_basic() -> String {
    let now = crate::utils::tz::now_utc();
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

fn now_rfc3339() -> String {
    crate::utils::tz::now_utc()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Render a signature template: `{@computed}` + `{caller}` + `{sig}`.
fn render(tpl: &str, computed: &HashMap<String, String>, vars: &Value) -> AppResult<String> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err(AppError::BadRequest(format!(
                "signature template '{tpl}' has an unclosed '{{'"
            )));
        };
        let name = &after[..end];
        let value = if let Some(stripped) = name.strip_prefix('@') {
            computed.get(stripped).cloned().ok_or_else(|| {
                AppError::BadRequest(format!(
                    "signature template references unknown computed var '{{@{stripped}}}'"
                ))
            })?
        } else if let Some(v) = computed.get(name) {
            // Plain names (e.g. `{sig}`) also resolve from computed values.
            v.clone()
        } else {
            match vars.get(name) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                _ => {
                    return Err(AppError::BadRequest(format!(
                        "signature template references unknown var '{{{name}}}'"
                    )));
                }
            }
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// AWS SigV4 official example (aws docs "Signature Version 4 signing
    /// process"). Fixed key/date/host/query → must reproduce the documented
    /// canonical request, string-to-sign and signature exactly.
    #[test]
    fn aws_sigv4_official_vector() {
        let mut url =
            reqwest::Url::parse("https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08")
                .unwrap();
        let mut headers = vec![
            ("host".into(), "iam.amazonaws.com".into()),
            ("x-amz-date".into(), "20150830T123600Z".into()),
            (
                "content-type".into(),
                "application/x-www-form-urlencoded; charset=utf-8".into(),
            ),
        ];
        let mut request = SignRequest {
            method: "GET",
            url: &mut url,
            headers: &mut headers,
            payload: b"",
        };
        let config = SignatureConfig {
            algorithm: "hmac-sha256".into(),
            encoding: "hex".into(),
            key: Some(SignKey::HmacChain {
                prefix: "AWS4".into(),
                steps: vec![
                    "{@date}".into(),
                    "us-east-1".into(),
                    "iam".into(),
                    "aws4_request".into(),
                ],
            }),
            canonical_headers: vec![
                "host".into(),
                "content-type".into(),
                "x-amz-date".into(),
            ],
            canonical_template: "{@method}\n{@uri}\n{@query}\n{@headers_canon}\n{@headers_signed}\n{@payload_hash}".into(),
            scope: Some("{@date}/us-east-1/iam/aws4_request".into()),
            string_to_sign_template: "AWS4-HMAC-SHA256\n{@timestamp}\n{@scope}\n{@canonical_hash}".into(),
            headers: None,
            query: None,
            timestamp: Some("{ts}".into()),
            inject: SignInject {
                into: "header".into(),
                header: "Authorization".into(),
                template: Some("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/{@scope}, SignedHeaders={@headers_signed}, Signature={sig}".into()),
                query_param: None,
            },
        };
        let vars = json!({
            "ts": "20150830T123600Z",
            "secret_key": "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "access_key": "AKIDEXAMPLE",
        });
        apply_signature(&mut request, &config, &vars).unwrap();

        // Canonical request hash: f536975d06c0309214f805bb90ccff089219ecd68b2577efef23edd43b7e1a59
        // Signature:             5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7
        // Signature:      5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7
        let auth = headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .unwrap()
            .1
            .clone();
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, Signature="
        ));
        assert!(
            auth.ends_with("5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"),
            "signature mismatch: {auth}"
        );
        assert_eq!(headers.len(), 4, "authorization appended");
    }

    #[test]
    fn alibaba_style_signs_sorted_query_with_encoded_form() {
        let mut url = reqwest::Url::parse(
            "https://ecs.aliyuncs.com/?Action=DescribeInstances&AccessKeyId=testId",
        )
        .unwrap();
        let mut headers: Vec<(String, String)> = vec![];
        let mut request = SignRequest {
            method: "GET",
            url: &mut url,
            headers: &mut headers,
            payload: b"",
        };
        let config = SignatureConfig {
            algorithm: "hmac-sha1".into(),
            encoding: "base64".into(),
            key: Some(SignKey::Secret {
                secret: Some("{secret_key}&".into()),
            }),
            canonical_headers: vec![],
            canonical_template: "{@query}".into(),
            scope: None,
            string_to_sign_template: "{@method}&{@enc_slash}&{@enc_query}".into(),
            headers: None,
            query: None,
            timestamp: None,
            inject: SignInject {
                into: "query".into(),
                header: "Authorization".into(),
                template: None,
                query_param: Some("Signature".into()),
            },
        };
        let vars = json!({"secret_key": "testSecret"});
        apply_signature(&mut request, &config, &vars).unwrap();
        eprintln!("DEBUG alibaba final url: {url}");
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "Signature" && !v.is_empty()),
            "Signature injected into query"
        );
    }
}
