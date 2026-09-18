//! Parser engine registry — the supply side of the parsing quality layer
//! (kb-parser-engines-design v4 §2).
//!
//! Every engine implements one trait; routing is layered:
//! ⓪ per-document override (`kb_documents.parser_engine`, request-level)
//! ① per-KB rules (`kb_knowledge_bases.parser_config.rules`, by file type)
//! ② global default env (`RAISFAST_KB_PARSER_ENGINE`)
//! ③ builtin (anydoc; zero-dependency floor) — always present
//!
//! Failure semantics [抄WK:engines.go CheckAvailable + parse 错误上抛]:
//! an unavailable engine (probe=false) is skipped with a warn (fall
//! through to the next layer); a RUNNING engine's parse error fails the
//! document — never silently degrade to builtin.

pub mod builtin;
pub mod docreader;
pub mod docreader_proto;
pub mod mineru;
pub mod mineru_cloud;
pub mod paddleocr_vl;
pub mod paddleocr_vl_cloud;

use std::sync::Arc;

use crate::errors::app_error::{AppError, AppResult};

use crate::kb::service::KbDeps;

/// One engine-produced image: `ref_name` is the reference in the markdown
/// (`images/fig-1.jpg` / an external URL), so image↔chunk association
/// survives via the markdown position [抄WK:Asset.Name 形态].
#[derive(Debug, Clone)]
pub struct ParsedImage {
    pub ref_name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Parse options derived from pipeline config (recognition off → engines
/// skip the asset pass, halving parse cost for text-only indexing
/// [抄WK:anydoc_reader extractImages 开关]).
#[derive(Debug, Clone, Copy)]
pub struct ParseOpts {
    pub extract_images: bool,
}

/// Unified parse result: all engines isomorphic, downstream (chunk /
/// image registration / recognition) shared [抄WK:doc.go ReadResult 契约].
#[derive(Debug, Clone, Default)]
pub struct ParseOutcome {
    pub markdown: String,
    pub images: Vec<ParsedImage>,
    /// Parser that produced the outcome.
    pub engine: String,
    /// Total pages when known (PDF paths).
    pub pages: Option<u32>,
    /// Pages skipped as scanned.
    pub scanned_pages: Vec<u32>,
}

#[async_trait::async_trait]
pub trait ParseEngine: Send + Sync {
    fn name(&self) -> &'static str;
    /// Whether this engine takes the file (mime primary, filename
    /// extension fallback).
    fn supports(&self, mime: &str, filename: &str) -> bool;
    /// Availability probe (unconfigured instance → false; result cached by
    /// the caller where useful).
    async fn probe(&self) -> bool;
    async fn parse(
        &self,
        bytes: &[u8],
        mime: &str,
        filename: &str,
        opts: &ParseOpts,
    ) -> AppResult<ParseOutcome>;
}

/// One KB-row routing rule [抄WK:types/knowledgebase.go ParserEngineRule].
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParserEngineRule {
    pub file_types: Vec<String>,
    pub engine: String,
    /// Per-rule engine params (each engine interprets its own; v1 unused).
    #[serde(default)]
    pub params: serde_json::Value,
}

/// `kb_knowledge_bases.parser_config` JSON shape.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ParserConfig {
    #[serde(default)]
    pub rules: Vec<ParserEngineRule>,
}

/// Name → engine registry; `builtin` is always present and terminal.
pub struct ParserRegistry {
    engines: Vec<Arc<dyn ParseEngine>>,
}

impl ParserRegistry {
    pub fn new(mut engines: Vec<Arc<dyn ParseEngine>>) -> Self {
        // builtin last = terminal fallback; ensure present exactly once.
        engines.retain(|e| e.name() != "builtin");
        engines.push(Arc::new(builtin::BuiltinEngine));
        Self { engines }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ParseEngine>> {
        self.engines.iter().find(|e| e.name() == name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.engines.iter().map(|e| e.name()).collect()
    }

    /// Layered routing (⓪ doc override → ① KB rules → ② global default →
    /// ③ builtin). Unavailable engines are skipped with a warn.
    pub async fn route(
        &self,
        deps: &KbDeps,
        kb: &crate::kb::models::knowledge_base::KbKnowledgeBase,
        doc_engine_override: Option<&str>,
        mime: &str,
        filename: &str,
    ) -> AppResult<(Arc<dyn ParseEngine>, Vec<String>)> {
        let mut warnings: Vec<String> = Vec::new();
        let mut try_engine = |name: &str, layer: &str| -> Option<String> {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            match self.get(name) {
                None => {
                    warnings.push(format!("{layer}: unknown engine '{name}'"));
                    None
                }
                Some(_) => Some(name.to_string()),
            }
        };
        // ⓪ per-document override (request-level; persists for reparse)
        let mut picked = doc_engine_override
            .and_then(|n| try_engine(n, "doc override"))
            .or_else(|| {
                // ① KB-row rules by file type
                let cfg = kb
                    .parser_config
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<ParserConfig>(v.clone()).ok())
                    .unwrap_or_default();
                let hit = cfg.rules.iter().find(|r| {
                    r.file_types.iter().any(|t| {
                        let t = t.trim().trim_start_matches('.');
                        matches_type(t, mime, filename)
                    })
                });
                hit.and_then(|r| try_engine(&r.engine, "kb rule"))
            })
            .or_else(|| {
                // ② global default
                try_engine(
                    deps.config.kb.parser_engine.as_deref().unwrap_or_default(),
                    "global default",
                )
            });
        // Resolve picked name → engine, probing availability.
        while let Some(name) = picked.take() {
            let engine = self
                .get(&name)
                .unwrap_or_else(|| Arc::new(builtin::BuiltinEngine));
            if engine.probe().await {
                return Ok((engine, warnings));
            }
            warnings.push(format!(
                "engine '{name}' unavailable (probe failed) → falling back"
            ));
            // Fall through to builtin (picked name invalid now).
        }
        // ③ builtin terminal.
        Ok((
            self.get("builtin").expect("builtin always registered"),
            warnings,
        ))
    }
}

/// Match a rule type token against the file: token is an extension or a
/// mime (prefix match on `image/*` style wildcards).
fn matches_type(token: &str, mime: &str, filename: &str) -> bool {
    let token = token.trim().trim_start_matches('.').to_ascii_lowercase();
    if token.contains('/') {
        if let Some(tail) = token.strip_suffix("/*") {
            return mime.starts_with(&format!("{tail}/"))
                || mime.starts_with(&format!("{}/", tail));
        }
        return mime.eq_ignore_ascii_case(&token);
    }
    filename
        .rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(&token))
}

/// Validate `parser_config` on KB save: rules must reference known engines
/// and carry at least one file type (route-time unknowns only warn — save
/// time must catch typos hard, kb-parser-engines-design §2 D2).
pub fn validate_parser_config(
    registry: &ParserRegistry,
    config: &serde_json::Value,
) -> AppResult<()> {
    let cfg: ParserConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid parser_config: {e}")))?;
    for r in &cfg.rules {
        if r.file_types.is_empty() {
            return Err(AppError::BadRequest(
                "parser_config rule needs at least one file type".into(),
            ));
        }
        if registry.get(&r.engine).is_none() {
            return Err(AppError::BadRequest(format!(
                "parser_config references unknown engine '{}' (known: {})",
                r.engine,
                registry.names().join(", ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ParserRegistry {
        ParserRegistry::new(Vec::new())
    }

    #[test]
    fn type_tokens_match_mime_and_extension() {
        assert!(matches_type("pdf", "application/pdf", "paper.pdf"));
        assert!(matches_type(".pdf", "application/pdf", "paper.pdf"));
        assert!(!matches_type("pdf", "text/markdown", "notes.md"));
        assert!(matches_type("image/*", "image/png", "photo.png"));
        assert!(matches_type("image/png", "image/png", "photo.png"));
    }

    #[test]
    fn validate_rejects_unknown_engine_and_empty_types() {
        let reg = registry();
        let err = validate_parser_config(
            &reg,
            &serde_json::json!({"rules": [{"file_types": ["pdf"], "engine": "ghost"}]}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown engine 'ghost'"));
        let err = validate_parser_config(
            &reg,
            &serde_json::json!({"rules": [{"file_types": [], "engine": "builtin"}]}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("at least one file type"));
        validate_parser_config(
            &reg,
            &serde_json::json!({"rules": [{"file_types": ["pdf"], "engine": "builtin"}]}),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn builtin_takes_non_images_and_probes_true() {
        let reg = registry();
        let builtin = reg.get("builtin").expect("builtin always registered");
        assert!(builtin.supports("application/pdf", "a.pdf"));
        assert!(builtin.supports("text/markdown", "a.md"));
        assert!(
            !builtin.supports("image/png", "a.png"),
            "image-as-document needs a service engine"
        );
        assert!(builtin.probe().await);
    }
}
