//! 加密工具函数。

/// 使用 HMAC-SHA1 签名（Base64 编码）。
///
/// key 会自动追加 `&` 后缀（OAuth 1.0 签名规范）。
pub fn hmac_sha1_sign(key: &str, data: &str) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha1::Sha1;
    type HmacSha1 = Hmac<Sha1>;

    let key_with_suffix = format!("{key}&");
    let mut mac = HmacSha1::new_from_slice(key_with_suffix.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(data.as_bytes());
    let result = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(result)
}
