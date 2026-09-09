//! E2E evaluation — recall hit rate + BLEU-4 + ROUGE-L
//! (kb-technical-design §12, metric set [抄WK:CHANGELOG E2E Testing 指标集]).
//!
//! Metric implementations are canonical published formulas (BLEU: Papineni
//! et al. 2002; ROUGE-L: Lin 2004) as arithmetic-level code — crates.io has
//! no maintained Rust library for them (surveyed 2026-09: all candidates
//! are single-version toys), same class as the RRF fusion [自造+理由：
//! 标准公式算术实现，非解析处理]. CJK answers are tokenized per character.

use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

use crate::errors::app_error::AppResult;
use crate::kb::pipeline::{self, AskRequest};
use crate::kb::service::KbDeps;

/// One evaluation case: question + expected retrieval keywords + optional
/// reference answer for generation scoring.
#[derive(Debug, Clone)]
pub struct EvalCase {
    pub question: String,
    pub expected_keywords: Vec<String>,
    pub expected_answer: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CaseReport {
    pub question: String,
    pub recall_hit: bool,
    pub status: &'static str,
    pub bleu4: Option<f64>,
    pub rouge_l: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct EvalReport {
    pub cases: Vec<CaseReport>,
    pub recall_hit_rate: f64,
    pub avg_bleu4: Option<f64>,
    pub avg_rouge_l: Option<f64>,
}

/// Character n-grams (CJK-friendly tokenization).
fn ngrams(tokens: &[char], n: usize) -> Vec<(Vec<char>,)> {
    if tokens.len() < n {
        return Vec::new();
    }
    tokens.windows(n).map(|w| (w.to_vec(),)).collect()
}

/// BLEU-4 with brevity penalty; +1 smoothing on empty n-gram matches so
/// short answers don't hard-zero (documented deviation, keeps ranking sane).
pub fn bleu4(reference: &str, candidate: &str) -> f64 {
    let r: Vec<char> = reference.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    if c.is_empty() || r.is_empty() {
        return 0.0;
    }
    let mut log_precision_sum = 0.0_f64;
    for n in 1..=4usize {
        let cand_grams = ngrams(&c, n);
        if cand_grams.is_empty() {
            break;
        }
        let mut ref_counts: std::collections::HashMap<Vec<char>, usize> =
            std::collections::HashMap::new();
        for g in ngrams(&r, n) {
            *ref_counts.entry(g.0).or_default() += 1;
        }
        let mut clipped = 0usize;
        let mut used: std::collections::HashMap<Vec<char>, usize> =
            std::collections::HashMap::new();
        for g in &cand_grams {
            let taken = used.get(&g.0).copied().unwrap_or(0);
            if taken < ref_counts.get(&g.0).copied().unwrap_or(0) {
                clipped += 1;
                used.insert(g.0.clone(), taken + 1);
            }
        }
        let precision = (clipped as f64 + 1.0) / (cand_grams.len() as f64 + 1.0);
        log_precision_sum += precision.ln();
    }
    let bp = if c.len() >= r.len() {
        1.0
    } else {
        (1.0 - r.len() as f64 / c.len() as f64).exp()
    };
    bp * (log_precision_sum / 4.0).exp()
}

/// ROUGE-L: F1 over the longest common subsequence (Lin 2004), β = ∞
/// variant is recall-heavy in the paper; we use balanced F1.
pub fn rouge_l(reference: &str, candidate: &str) -> f64 {
    let r: Vec<char> = reference.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    if r.is_empty() || c.is_empty() {
        return 0.0;
    }
    // LCS length (DP, O(|r|·|c|)).
    let mut dp = vec![0usize; c.len() + 1];
    for i in 1..=r.len() {
        let mut prev = 0usize;
        for j in 1..=c.len() {
            let temp = dp[j];
            dp[j] = if r[i - 1] == c[j - 1] {
                prev + 1
            } else {
                dp[j].max(dp[j - 1])
            };
            prev = temp;
        }
    }
    let lcs = dp[c.len()] as f64;
    let precision = lcs / c.len() as f64;
    let recall = lcs / r.len() as f64;
    if precision + recall == 0.0 {
        return 0.0;
    }
    2.0 * precision * recall / (precision + recall)
}

/// Run the dataset against a prepared KB: retrieval hit = every expected
/// keyword appears in some recalled context unit.
pub async fn run_eval(deps: &KbDeps, kb_ids: &[i64], cases: &[EvalCase]) -> AppResult<EvalReport> {
    let mut reports = Vec::with_capacity(cases.len());
    for case in cases {
        let ask = AskRequest {
            kb_ids: kb_ids.to_vec(),
            doc_ids: Vec::new(),
            question: case.question.clone(),
        };
        let mut outcome = pipeline::prepare_answer(deps, &ask).await?;
        let recall_hit = case.expected_keywords.iter().all(|kw| {
            outcome
                .context_units
                .iter()
                .any(|u| u.content.contains(kw.as_str()))
        });
        pipeline::finish_answer(deps, &mut outcome).await?;
        let (bleu4, rouge_l) = case
            .expected_answer
            .as_ref()
            .map(|expected| {
                (
                    Some(bleu4(expected, &outcome.answer)),
                    Some(rouge_l(expected, &outcome.answer)),
                )
            })
            .unwrap_or((None, None));
        reports.push(CaseReport {
            question: case.question.clone(),
            recall_hit,
            status: outcome.status,
            bleu4,
            rouge_l,
        });
    }
    let n = reports.len().max(1) as f64;
    let hits = reports.iter().filter(|r| r.recall_hit).count() as f64;
    let bleus: Vec<f64> = reports.iter().filter_map(|r| r.bleu4).collect();
    let rouges: Vec<f64> = reports.iter().filter_map(|r| r.rouge_l).collect();
    Ok(EvalReport {
        recall_hit_rate: hits / n,
        avg_bleu4: (!bleus.is_empty()).then(|| bleus.iter().sum::<f64>() / bleus.len() as f64),
        avg_rouge_l: (!rouges.is_empty()).then(|| rouges.iter().sum::<f64>() / rouges.len() as f64),
        cases: reports,
    })
}

/// Scripted echo provider for eval runs against deterministic answers.
pub struct EvalEchoProvider;

#[async_trait::async_trait]
impl ModelProvider for EvalEchoProvider {
    fn name(&self) -> &str {
        "eval_echo"
    }

    async fn chat(
        &self,
        _request: &ChatRequest<'_>,
        _model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse::text_only("[1] 评估回答。"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bleu4_identical_is_one() {
        let score = bleu4("支持三种数据库后端", "支持三种数据库后端");
        assert!(
            (score - 1.0).abs() < 1e-9,
            "identical strings must score 1.0, got {score}"
        );
    }

    #[test]
    fn bleu4_disjoint_is_low() {
        let score = bleu4("完全不同的内容甲", "毫无交集的文字乙");
        assert!(score < 0.3, "disjoint answers must score low, got {score}");
    }

    #[test]
    fn rouge_l_known_value() {
        // r = ABC, c = ADC → LCS = AC (2); P=2/3, R=2/3, F1=2/3.
        let score = rouge_l("ABC", "ADC");
        assert!((score - 2.0 / 3.0).abs() < 1e-9, "got {score}");
    }

    #[test]
    fn rouge_l_empty_is_zero() {
        assert_eq!(rouge_l("", "abc"), 0.0);
        assert_eq!(rouge_l("abc", ""), 0.0);
    }

    #[tokio::test]
    async fn eval_dataset_recall_over_seeded_kb() {
        use crate::kb::service::{KbDeps, KbEmbedder};
        use crate::kb::vectors::BruteForceIndex;
        use std::sync::Arc;

        struct SumEmbedder;
        #[async_trait::async_trait]
        impl KbEmbedder for SumEmbedder {
            async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        let mut v = vec![0.0_f32; 4];
                        v[t.bytes().map(|b| b as usize).sum::<usize>() % 4] = 1.0;
                        v
                    })
                    .collect())
            }
        }

        let pool = crate::test_pool!();
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        config.kb.fallback_threshold = 0.05;
        let deps = KbDeps {
            pool,
            config: Arc::new(config),
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-eval-test", "/uploads").unwrap(),
            ),
            vector: Arc::new(BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(SumEmbedder),
            provider: Some(Arc::new(EvalEchoProvider)),
            emitter: crate::event::EventEmitter::eventbus_only(crate::eventbus::EventBus::new(16)),
        };
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "eval".into(),
                description: None,
                slug: "eval".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();
        let mut markdown = "# 数据库\n\n".to_string();
        markdown.push_str(
            &"raisfast 支持 SQLite PostgreSQL MySQL 三种数据库后端，向量检索使用 Qdrant。"
                .repeat(50),
        );
        let doc = crate::kb::service::create_online_document(
            &deps,
            kb.id,
            "数据库",
            &markdown,
            None,
            "default",
        )
        .await
        .unwrap();
        crate::kb::service::process_document(&deps, doc.id, "default")
            .await
            .unwrap();

        let report = run_eval(
            &deps,
            &[i64::from(kb.id)],
            &[
                EvalCase {
                    question: "支持哪些数据库".into(),
                    expected_keywords: vec!["PostgreSQL".into()],
                    expected_answer: None,
                },
                EvalCase {
                    question: "向量检索用什么".into(),
                    expected_keywords: vec!["Qdrant".into()],
                    expected_answer: Some("[1] 评估回答。".into()),
                },
            ],
        )
        .await
        .unwrap();
        assert_eq!(report.cases.len(), 2);
        assert!(
            report.recall_hit_rate >= 0.99,
            "both cases must recall their keywords: {:?}",
            report.cases
        );
        assert!(
            report.avg_rouge_l.unwrap_or(0.0) > 0.99,
            "echo answer must match reference"
        );
    }
}
