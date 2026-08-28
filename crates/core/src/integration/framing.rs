//! L2 framing/codec — bytes → structured value (M1: `raw` framing + `json` codec).
//!
//! Additional framings (json-rpc, grpc, soap, mime, csv) arrive in later
//! phases per the roadmap; unknown combinations fail fast with guidance.

use serde_json::Value;

use crate::errors::app_error::AppError;

/// Decode a request body into the structured value handed to the mapper.
///
/// # Errors
///
/// - `BadRequest` for unsupported framing/codec combinations (with guidance)
/// - `BadRequest` for malformed JSON bodies
pub fn decode(framing: &str, codec: &str, body: &[u8]) -> Result<Value, AppError> {
    match (framing, codec) {
        ("raw", "json") => {
            if body.is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_slice(body).map_err(|e| {
                AppError::BadRequest(format!("malformed JSON body: {e}"))
            })
        }
        ("raw", _) => Err(AppError::BadRequest(format!(
            "codec '{codec}' not supported in this phase — use 'json' or a normalizer plugin"
        ))),
        (other, _) => Err(AppError::BadRequest(format!(
            "framing '{other}' not supported in this phase — use 'raw' or a normalizer plugin"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_json_object() {
        let v = decode("raw", "json", br#"{"id":42}"#).expect("decode");
        assert_eq!(v["id"], 42);
    }

    #[test]
    fn empty_body_is_null() {
        let v = decode("raw", "json", b"").expect("decode");
        assert!(v.is_null());
    }

    #[test]
    fn malformed_json_rejected() {
        assert!(decode("raw", "json", b"{oops").is_err());
    }

    #[test]
    fn unsupported_framing_guides_to_plugin() {
        let err = decode("json-rpc", "json", b"{}").expect_err("unsupported");
        let msg = err.to_string();
        assert!(msg.contains("normalizer plugin"), "guidance present: {msg}");
    }
}
