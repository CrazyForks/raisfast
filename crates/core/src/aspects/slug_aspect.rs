//! Slug Aspect — auto-generate slug from title/name on create
//!
//! Targets: posts, pages, tags, categories, products (and CMS tables with a slug field).
//! Priority: -300 (after timestampable -400, ownable -500).
//!
//! On before_create: if slug is empty/missing, generate from "title" or "name" field
//! with a random hex suffix for uniqueness.

use async_trait::async_trait;
use serde_json::json;
use slug::slugify;

use super::{
    Advice, Aspect, AspectResult, DataBeforeCreateContext, DataBeforeUpdateContext, Layer,
    Operation, Pointcut, TargetMatcher, When,
};

const SLUG_TABLES: &[&str] = &["posts", "pages", "tags", "categories", "products"];

pub struct SlugAspect;

#[async_trait]
impl Aspect for SlugAspect {
    fn name(&self) -> &str {
        "slug"
    }

    fn priority(&self) -> i32 {
        -300
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        let tables: Vec<String> = SLUG_TABLES.iter().map(|s| (*s).to_string()).collect();
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::Tables(tables.clone()),
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::Before,
                target: TargetMatcher::Tables(tables),
            },
        ]
    }

    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult {
        if slug_is_empty(&ctx.record)
            && let Some(source) = slug_source(&ctx.record)
        {
            ctx.record
                .insert("slug".into(), json!(make_unique_slug(&source)));
        }
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if slug_is_empty(&ctx.new_record)
            && let Some(source) = slug_source(&ctx.new_record)
        {
            ctx.new_record
                .insert("slug".into(), json!(make_unique_slug(&source)));
        }
        Ok(Advice::Continue)
    }
}

fn slug_is_empty(record: &serde_json::Map<String, serde_json::Value>) -> bool {
    record
        .get("slug")
        .is_none_or(|v| v.as_str().is_none_or(|s| s.is_empty()))
}

fn slug_source(record: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    record
        .get("title")
        .or_else(|| record.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

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
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, Record};

    #[test]
    fn slug_is_empty_true_when_missing() {
        let r = Record::new();
        assert!(slug_is_empty(&r));
    }

    #[test]
    fn slug_is_empty_true_when_empty() {
        let mut r = Record::new();
        r.insert("slug".into(), json!(""));
        assert!(slug_is_empty(&r));
    }

    #[test]
    fn slug_is_empty_false_when_set() {
        let mut r = Record::new();
        r.insert("slug".into(), json!("hello-world"));
        assert!(!slug_is_empty(&r));
    }

    #[test]
    fn slug_source_from_title() {
        let mut r = Record::new();
        r.insert("title".into(), json!("Hello World"));
        assert_eq!(slug_source(&r), Some("Hello World".to_string()));
    }

    #[test]
    fn slug_source_from_name() {
        let mut r = Record::new();
        r.insert("name".into(), json!("Rust"));
        assert_eq!(slug_source(&r), Some("Rust".to_string()));
    }

    #[test]
    fn slug_source_title_preferred_over_name() {
        let mut r = Record::new();
        r.insert("title".into(), json!("Title"));
        r.insert("name".into(), json!("Name"));
        assert_eq!(slug_source(&r), Some("Title".to_string()));
    }

    #[test]
    fn slug_source_none_when_both_empty() {
        let mut r = Record::new();
        r.insert("title".into(), json!(""));
        r.insert("name".into(), json!(""));
        assert_eq!(slug_source(&r), None);
    }

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

    #[tokio::test]
    async fn aspect_generates_slug_on_create() {
        let engine = AspectEngine::new();
        engine.register(SlugAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: {
                let mut r = Record::new();
                r.insert("title".into(), json!("My First Post"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        let slug = ctx.record.get("slug").unwrap().as_str().unwrap();
        assert!(slug.starts_with("my-first-post-"));
    }

    #[tokio::test]
    async fn aspect_skips_when_slug_provided() {
        let engine = AspectEngine::new();
        engine.register(SlugAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: {
                let mut r = Record::new();
                r.insert("title".into(), json!("My First Post"));
                r.insert("slug".into(), json!("custom-slug"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("slug").unwrap(), &json!("custom-slug"));
    }

    #[tokio::test]
    async fn aspect_ignores_non_target_table() {
        let engine = AspectEngine::new();
        engine.register(SlugAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "users".into(),
            record: {
                let mut r = Record::new();
                r.insert("title".into(), json!("Should Not Slug"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("users", &mut ctx)
            .await
            .unwrap();

        assert!(ctx.record.get("slug").is_none());
    }

    #[tokio::test]
    async fn aspect_uses_name_for_tags() {
        let engine = AspectEngine::new();
        engine.register(SlugAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "tags".into(),
            record: {
                let mut r = Record::new();
                r.insert("name".into(), json!("Rust Lang"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("tags", &mut ctx)
            .await
            .unwrap();

        let slug = ctx.record.get("slug").unwrap().as_str().unwrap();
        assert!(slug.starts_with("rust-lang-"));
    }
}
