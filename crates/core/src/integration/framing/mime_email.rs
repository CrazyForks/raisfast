//! L2 `mime` framing + `email` codec — RFC5322/MIME message → JSON tree
//! (integration.md §2 "邮箱收信" row). Parsing is delegated to `mail-parser`
//! (Stalwart): charsets, encoded words (RFC2047), multipart and nested
//! messages are handled there; the wire envelope is a flat JSON object so
//! mappings stay plain JSONPath.
//!
//! Output shape:
//! ```json
//! {
//!   "message_id": "<abc@example.com>",     // receipts idempotency key
//!   "in_reply_to": "<def@example.com>",
//!   "subject": "…",
//!   "from": { "name": "…", "address": "a@b.c" },
//!   "to":   [ { "name": "…", "address": "…" } ],
//!   "date": "2026-08-30T12:00:00+00:00",
//!   "text_body": "…",
//!   "html_body": "…",
//!   "attachments": [ { "filename": "…", "content_type": "…", "size": 123 } ]
//! }
//! ```
//!
//! `message_id` fallback: messages without a Message-ID header get a
//! deterministic synthesized id (body hash) so receipts dedup still applies.

use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};

/// Decode a raw RFC5322 message into the JSON envelope.
///
/// # Errors
///
/// `AppError::BadRequest` when the message cannot be parsed as MIME.
pub fn decode(body: &[u8]) -> AppResult<Value> {
    let msg = mail_parser::MessageParser::default()
        .parse(body)
        .ok_or_else(|| AppError::BadRequest("malformed MIME message".into()))?;

    let message_id = match msg.message_id() {
        Some(id) => id.to_string(),
        None => synthesized_id(body),
    };

    Ok(json!({
        "message_id": message_id,
        "in_reply_to": header_str(msg.in_reply_to().as_text()),
        "subject": msg.subject().unwrap_or_default(),
        "from": addr_json(msg.from()),
        "to": addr_list_json(msg.to()),
        "cc": addr_list_json(msg.cc()),
        "bcc": addr_list_json(msg.bcc()),
        "reply_to": addr_json(msg.reply_to()),
        "date": date_json(msg.date()),
        "text_body": msg.body_text(0).map(|c| c.into_owned()).unwrap_or_default(),
        "html_body": msg.body_html(0).map(|c| c.into_owned()),
        "attachments": attachments_json(&msg),
        // Escape hatch: EVERY header, lowercased — same convention as the
        // push pipeline's `_headers` injection. Mapping can pick any of
        // them (`$._headers.list-unsubscribe`, `$._headers.references`, …).
        "_headers": headers_json(&msg),
    }))
}

/// All message headers as a lowercase-keyed object (decoded where possible,
/// raw text otherwise; duplicated keys keep the first occurrence).
fn headers_json(msg: &mail_parser::Message<'_>) -> Value {
    let mut map = serde_json::Map::new();
    for header in msg.headers() {
        let key = header.name().to_ascii_lowercase();
        if map.contains_key(&key) {
            continue;
        }
        let text = header
            .value()
            .as_text()
            .map(str::to_string)
            .unwrap_or_else(|| {
                // Raw slice fallback — strip the leading "Name:" prefix.
                let raw = String::from_utf8_lossy(
                    &msg.raw_message()[header.offset_start as usize..header.offset_end as usize],
                )
                .trim()
                .to_string();
                raw.strip_prefix(&format!("{}:", header.name()))
                    .unwrap_or(&raw)
                    .trim()
                    .to_string()
            });
        map.insert(key, Value::String(text));
    }
    Value::Object(map)
}

fn header_str(v: Option<&str>) -> Option<String> {
    v.map(str::to_string)
}

fn addr_json(addr: Option<&mail_parser::Address<'_>>) -> Value {
    let Some(addr) = addr else {
        return Value::Null;
    };
    let first = match addr {
        mail_parser::Address::List(list) => list.first(),
        mail_parser::Address::Group(groups) => groups.first().and_then(|g| g.addresses.first()),
    };
    match first {
        Some(a) => json!({
            "name": a.name.as_deref().unwrap_or_default(),
            "address": a.address.as_deref().unwrap_or_default(),
        }),
        None => Value::Null,
    }
}

fn addr_list_json(addr: Option<&mail_parser::Address<'_>>) -> Value {
    let Some(addr) = addr else {
        return json!([]);
    };
    let list = match addr {
        mail_parser::Address::List(list) => list,
        mail_parser::Address::Group(groups) => {
            return Value::Array(
                groups
                    .iter()
                    .flat_map(|g| g.addresses.iter())
                    .map(|a| {
                        json!({
                            "name": a.name.as_deref().unwrap_or_default(),
                            "address": a.address.as_deref().unwrap_or_default(),
                        })
                    })
                    .collect(),
            );
        }
    };
    Value::Array(
        list.iter()
            .map(|a| {
                json!({
                    "name": a.name.as_deref().unwrap_or_default(),
                    "address": a.address.as_deref().unwrap_or_default(),
                })
            })
            .collect(),
    )
}

fn date_json(date: Option<&mail_parser::DateTime>) -> Value {
    let Some(d) = date else {
        return Value::Null;
    };
    let ts = d.to_timestamp();
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => json!(dt.to_rfc3339()),
        None => Value::Null,
    }
}

fn attachments_json(msg: &mail_parser::Message<'_>) -> Value {
    use mail_parser::MimeHeaders;
    Value::Array(
        msg.attachments()
            .map(|part| {
                let content_type = part
                    .content_type()
                    .map(|ct| match &ct.c_subtype {
                        Some(sub) => format!("{}/{}", ct.c_type, sub),
                        None => ct.c_type.to_string(),
                    })
                    .unwrap_or_else(|| "application/octet-stream".into());
                let size = match &part.body {
                    mail_parser::PartType::Text(t) => t.len(),
                    mail_parser::PartType::Html(h) => h.len(),
                    mail_parser::PartType::Binary(b) | mail_parser::PartType::InlineBinary(b) => {
                        b.len()
                    }
                    _ => 0,
                };
                json!({
                    "filename": part.attachment_name().unwrap_or_default(),
                    "content_type": content_type,
                    "size": size,
                })
            })
            .collect(),
    )
}

/// Deterministic id for messages lacking a Message-ID header (hash of the
/// raw bytes — same message re-fetched dedups, different messages differ).
/// Bracket-free to match mail-parser's normalized header values.
fn synthesized_id(body: &[u8]) -> String {
    use sha2::Digest;
    format!("{}@synthesized", hex::encode(sha2::Sha256::digest(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "Message-ID: <abc-123@example.com>\r\n\
        From: Alice <alice@example.com>\r\n\
        To: Bob <bob@example.com>, Carol <carol@example.com>\r\n\
        Subject: Hello\r\n\
        Date: Mon, 30 Aug 2026 12:00:00 +0000\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        \r\n\
        Hi Bob!";

    #[test]
    fn decodes_plain_message() {
        let v = decode(PLAIN.as_bytes()).expect("decode");
        // mail-parser normalizes Message-ID without angle brackets.
        assert_eq!(v["message_id"], "abc-123@example.com");
        assert_eq!(v["subject"], "Hello");
        assert_eq!(v["from"]["address"], "alice@example.com");
        assert_eq!(v["from"]["name"], "Alice");
        assert_eq!(v["to"].as_array().map(Vec::len), Some(2));
        assert_eq!(v["to"][1]["address"], "carol@example.com");
        assert_eq!(v["text_body"], "Hi Bob!");
        assert_eq!(v["attachments"].as_array().map(Vec::len), Some(0));
        assert!(
            v["date"]
                .as_str()
                .unwrap_or_default()
                .starts_with("2026-08-30")
        );
    }

    #[test]
    fn decodes_utf8_encoded_subject() {
        // RFC2047 encoded-word for a UTF-8 subject.
        let raw = "Message-ID: <u1@example.com>\r\n\
            From: =?UTF-8?B?5byg5LiJ?= <zhang@example.com>\r\n\
            Subject: =?UTF-8?B?5rWL6K+V?= report\r\n\
            \r\n\
            body";
        let v = decode(raw.as_bytes()).expect("decode");
        assert_eq!(v["subject"], "测试 report");
        assert_eq!(v["from"]["name"], "张三");
    }

    #[test]
    fn decodes_multipart_with_attachment() {
        let raw = "Message-ID: <m1@example.com>\r\n\
            From: s@example.com\r\n\
            Subject: with attachment\r\n\
            MIME-Version: 1.0\r\n\
            Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
            \r\n\
            --BOUND\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            see attached\r\n\
            --BOUND\r\n\
            Content-Type: application/pdf; name=\"doc.pdf\"\r\n\
            Content-Disposition: attachment; filename=\"doc.pdf\"\r\n\
            Content-Transfer-Encoding: base64\r\n\
            \r\n\
            aGVsbG8gcGRmIQ==\r\n\
            --BOUND--\r\n";
        let v = decode(raw.as_bytes()).expect("decode");
        assert_eq!(v["text_body"], "see attached");
        let atts = v["attachments"].as_array().expect("attachments");
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["filename"], "doc.pdf");
        assert_eq!(atts[0]["content_type"], "application/pdf");
        assert_eq!(atts[0]["size"], 10); // "hello pdf!" decoded
    }

    #[test]
    fn missing_message_id_gets_stable_synthesized_id() {
        let raw = "From: x@example.com\r\nSubject: no id\r\n\r\nbody";
        let v1 = decode(raw.as_bytes()).expect("decode");
        let v2 = decode(raw.as_bytes()).expect("decode");
        let id = v1["message_id"].as_str().expect("synthesized").to_string();
        assert!(id.ends_with("@synthesized"), "id: {id}");
        assert_eq!(id, v2["message_id"], "deterministic across parses");
        let other = decode(format!("{raw}2").as_bytes()).expect("decode");
        assert_ne!(id, other["message_id"], "different bodies differ");
    }

    #[test]
    fn empty_body_rejected_and_parser_is_lenient_by_design() {
        // Empty input fails parsing outright…
        assert!(decode(b"").is_err());
        // …while random bytes are treated as a header-less message —
        // mail-parser never rejects, dedup falls back to the synthesized id.
        let v = decode(&[0xff, 0xfe, 0x00, 0x01, 0x02]).expect("lenient parse");
        assert!(
            v["message_id"]
                .as_str()
                .unwrap_or_default()
                .ends_with("@synthesized")
        );
    }

    #[test]
    fn headers_escape_hatch_contains_everything() {
        let raw = "Message-ID: <h9@example.com>\r\n\
            From: a@b.c\r\n\
            List-Unsubscribe: <https://x.y/unsub>\r\n\
            X-Mailer: NetEase WebMail\r\n\
            References: <ref1@x> <ref2@x>\r\n\
            \r\n\
            body";
        let v = decode(raw.as_bytes()).expect("decode");
        assert_eq!(v["_headers"]["message-id"], "h9@example.com");
        assert_eq!(v["_headers"]["list-unsubscribe"], "<https://x.y/unsub>");
        assert_eq!(v["_headers"]["x-mailer"], "NetEase WebMail");
        assert!(
            v["_headers"]["references"]
                .as_str()
                .unwrap_or_default()
                .contains("ref2@x")
        );
        // Address-type headers fall back to decoded text via as_text.
        assert!(
            v["_headers"]["from"]
                .as_str()
                .unwrap_or_default()
                .contains("a@b.c")
        );
    }

    #[test]
    fn cc_reply_to_extracted() {
        let raw = "Message-ID: <cc1@example.com>\r\n\
            From: a@b.c\r\n\
            To: x@y.z\r\n\
            Cc: One <one@x.y>, Two <two@x.y>\r\n\
            Reply-To: Support <support@b.c>\r\n\
            Date: Mon, 30 Aug 2026 12:00:00 +0000\r\n\
            \r\n\
            body";
        let v = decode(raw.as_bytes()).expect("decode");
        let cc = v["cc"].as_array().expect("cc array");
        assert_eq!(cc.len(), 2);
        assert_eq!(cc[0]["address"], "one@x.y");
        assert_eq!(cc[1]["name"], "Two");
        assert_eq!(v["reply_to"]["address"], "support@b.c");
        assert_eq!(v["reply_to"]["name"], "Support");
        assert_eq!(v["bcc"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn in_reply_to_extracted() {
        let raw = "Message-ID: <r2@example.com>\r\n\
            In-Reply-To: <r1@example.com>\r\n\
            From: a@b.c\r\n\r\nreply";
        let v = decode(raw.as_bytes()).expect("decode");
        assert_eq!(v["in_reply_to"], "r1@example.com");
    }
}

#[cfg(test)]
mod diag_tests {
    #[test]
    #[ignore = "local raw eml diagnostic"]
    fn diag_html_vs_text() {
        let path = "/Users/chriszhong/work/www/Rust/raisfast/storage/integration/raw/2093862484409581568/2093890939540996096.bin";
        let Ok(raw) = std::fs::read(path) else {
            eprintln!("no file");
            return;
        };
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        let html = msg.body_html(0).unwrap_or_default().into_owned();
        let text = msg.body_text(0).unwrap_or_default().into_owned();
        eprintln!("html 含 DeepSeek: {}", html.contains("DeepSeek"));
        eprintln!("text 含 DeepSeek: {}", text.contains("DeepSeek"));
        eprintln!("text 前300: {:?}", &text[..text.len().min(300)]);
        eprintln!("text 含CSS残留 margin: {}", text.contains("margin"));
        eprintln!("text 含box-sizing: {}", text.contains("box-sizing"));
    }
}
