//! Excerpt extraction helper.
//!
//! Used by the post service to auto-generate excerpts from content.

pub fn extract_excerpt(content: &str, max_len: usize) -> String {
    let plain = content
        .chars()
        .take(max_len.saturating_mul(2))
        .collect::<String>();
    if plain.len() > max_len {
        format!("{}...", &plain[..plain.ceil_char_boundary(max_len)])
    } else {
        plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_excerpt_short_content() {
        let result = extract_excerpt("Hello world", 200);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn extract_excerpt_truncates_long_content() {
        let content: String = "A".repeat(300);
        let result = extract_excerpt(&content, 200);
        assert_eq!(result.len(), 203); // 200 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn extract_excerpt_exact_boundary() {
        let content: String = "A".repeat(200);
        let result = extract_excerpt(&content, 200);
        assert_eq!(result.len(), 200);
        assert!(!result.ends_with("..."));
    }

    #[test]
    fn extract_excerpt_unicode_safe() {
        let content = "你好世界".repeat(100);
        let result = extract_excerpt(&content, 10);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn extract_excerpt_zero_max_len() {
        let result = extract_excerpt("Hello", 0);
        assert_eq!(result, "");
    }
}
