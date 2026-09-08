//! S8 context assembly — budget-bounded, FAQ pinned
//! [抄WK:chat_pipeline/into_chat_message.go + context_template.yaml 语义].
//!
//! Token estimation: chars/4 + constant (the agent glossary U heuristic
//! [抄RF:glossary B1]); calibrated during E2E evaluation.

use crate::kb::pipeline::{AskOutcome, ContextUnit};

const CHARS_PER_TOKEN: usize = 4;

/// Estimate tokens for context-budget accounting.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / CHARS_PER_TOKEN + 8
}

/// Select the units that fit the budget. FAQ units are pinned first (S7①
/// semantics: standard answers get top context priority), then the rest in
/// relevance order until the budget is exhausted.
pub fn assemble(outcome: &AskOutcome, budget_tokens: u32) -> Vec<ContextUnit> {
    let budget = budget_tokens as usize;
    let mut selected: Vec<ContextUnit> = Vec::new();
    let mut used = 0usize;

    let (faqs, rest): (Vec<&ContextUnit>, Vec<&ContextUnit>) =
        outcome.context_units.iter().partition(|u| u.is_faq);

    for unit in faqs.into_iter().chain(rest) {
        let cost = estimate_tokens(&unit.content) + estimate_tokens(&unit.title);
        if used + cost > budget && !selected.is_empty() {
            break;
        }
        used += cost;
        selected.push(unit.clone());
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: i64, content: &str, is_faq: bool) -> ContextUnit {
        ContextUnit {
            unit_id: id,
            kind: if is_faq { "faq" } else { "document" }.into(),
            title: format!("t{id}"),
            content: content.to_string(),
            score: 1.0,
            is_faq,
        }
    }

    #[test]
    fn faq_pinned_first() {
        let outcome = AskOutcome {
            status: "answered",
            question: "q".into(),
            answer: String::new(),
            references: Vec::new(),
            top_score: 1.0,
            context_units: vec![
                unit(1, &"aaaa".repeat(100), false),
                unit(2, "FAQ 答案", true),
            ],
        };
        let selected = assemble(&outcome, 10_000);
        assert_eq!(selected[0].unit_id, 2, "FAQ must lead the context");
    }

    #[test]
    fn budget_truncates() {
        let outcome = AskOutcome {
            status: "answered",
            question: "q".into(),
            answer: String::new(),
            references: Vec::new(),
            top_score: 1.0,
            context_units: (1..=10).map(|i| unit(i, &"x".repeat(400), false)).collect(),
        };
        let selected = assemble(&outcome, 200);
        assert!(selected.len() < 10, "budget must cut units");
    }
}
