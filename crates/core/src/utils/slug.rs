//! Slug generation helpers.
//!
//! Used by post, page, tag, category, and product services to generate
//! URL-friendly slugs from titles or names.

use slug::slugify;

pub fn make_unique_slug(base: &str) -> String {
    let suffix = crate::utils::id::random_hex(2);
    format!("{}-{}", slugify(base), suffix)
}

pub fn generate_slug(title: &str) -> String {
    slugify(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_unique_slug_format() {
        let slug = make_unique_slug("Hello World");
        assert!(slug.starts_with("hello-world-"));
        assert_eq!(slug.len(), "hello-world-".len() + 4);
    }

    #[test]
    fn generate_slug_basic() {
        assert_eq!(generate_slug("Hello World"), "hello-world");
    }

    #[test]
    fn generate_slug_special_chars() {
        assert_eq!(generate_slug("Hello, World! (2024)"), "hello-world-2024");
    }
}
