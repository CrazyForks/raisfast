//! Excerpt Aspect — auto-generate excerpt from content on create/update
//!
//! Targets: posts (and CMS tables with a content + excerpt field).
//! Priority: -200 (after slug -300, timestampable -400, ownable -500).
//!
//! On before_create/update: if excerpt is empty/missing, extract from "content" field.

use async_trait::async_trait;
use serde_json::json;

use super::{
    Advice, Aspect, AspectResult, DataBeforeCreateContext, DataBeforeUpdateContext, Layer,
    Operation, Pointcut, TargetMatcher, When,
};

const DEFAULT_MAX_LEN: usize = 200;

const EXCERPT_TABLES: &[&str] = &["posts"];

pub struct ExcerptAspect;

#[async_trait]
impl Aspect for ExcerptAspect {
    fn name(&self) -> &str {
        "excerpt"
    }

    fn priority(&self) -> i32 {
        -200
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        let tables: Vec<String> = EXCERPT_TABLES.iter().map(|s| (*s).to_string()).collect();
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
        if excerpt_is_empty(&ctx.record)
            && let Some(content) = content_str(&ctx.record)
        {
            ctx.record.insert(
                "excerpt".into(),
                json!(extract_excerpt(&content, DEFAULT_MAX_LEN)),
            );
        }
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        if excerpt_is_empty(&ctx.new_record) {
            let content = content_str(&ctx.new_record).or_else(|| content_str(&ctx.old_record));
            if let Some(content) = content {
                ctx.new_record.insert(
                    "excerpt".into(),
                    json!(extract_excerpt(&content, DEFAULT_MAX_LEN)),
                );
            }
        }
        Ok(Advice::Continue)
    }
}

fn excerpt_is_empty(record: &serde_json::Map<String, serde_json::Value>) -> bool {
    record
        .get("excerpt")
        .is_none_or(|v| v.as_str().is_none_or(|s| s.is_empty()))
}

fn content_str(record: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    record
        .get("content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

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
    use crate::aspects::engine::AspectEngine;
    use crate::aspects::{BaseContext, Record};

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

    #[tokio::test]
    async fn aspect_generates_excerpt_on_create() {
        let engine = AspectEngine::new();
        engine.register(ExcerptAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: {
                let mut r = Record::new();
                r.insert("content".into(), json!("This is my post content"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(
            ctx.record.get("excerpt").unwrap(),
            &json!("This is my post content")
        );
    }

    #[tokio::test]
    async fn aspect_skips_when_excerpt_provided() {
        let engine = AspectEngine::new();
        engine.register(ExcerptAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            record: {
                let mut r = Record::new();
                r.insert("content".into(), json!("Some content"));
                r.insert("excerpt".into(), json!("Custom excerpt"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(ctx.record.get("excerpt").unwrap(), &json!("Custom excerpt"));
    }

    #[tokio::test]
    async fn aspect_ignores_non_target_table() {
        let engine = AspectEngine::new();
        engine.register(ExcerptAspect);

        let mut ctx = DataBeforeCreateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "pages".into(),
            record: {
                let mut r = Record::new();
                r.insert("content".into(), json!("Some content"));
                r
            },
            schema: None,
        };

        engine
            .dispatch_data_before_create("pages", &mut ctx)
            .await
            .unwrap();

        assert!(ctx.record.get("excerpt").is_none());
    }

    #[tokio::test]
    async fn aspect_uses_old_content_on_update() {
        let engine = AspectEngine::new();
        engine.register(ExcerptAspect);

        let mut old = Record::new();
        old.insert("content".into(), json!("Old content here"));

        let new = Record::new();

        let mut ctx = DataBeforeUpdateContext {
            base: BaseContext::new(None, "default".into(), "now".into()),
            table: "posts".into(),
            old_record: old,
            new_record: new,
            schema: None,
        };

        engine
            .dispatch_data_before_update("posts", &mut ctx)
            .await
            .unwrap();

        assert_eq!(
            ctx.new_record.get("excerpt").unwrap(),
            &json!("Old content here")
        );
    }
}
