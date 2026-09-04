//! Transcript replay windowing: fold oldest whole user-turns into a durable
//! summary when the raw transcript would exceed a token budget.
//!
//! Reference: zeroclaw `consolidation` semantics (old turns → one summary kept
//! retrievable), adapted to our append-only replay (`[自造]` host state in
//! `ai_sessions.meta.ctx`). No LLM here: pure selection + text shaping; the
//! summarization call and durability live in the service layer.

use serde::{Deserialize, Serialize};

/// Durable fold state stored on the session (`meta.ctx`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CtxState {
    /// Highest persisted `seq` covered by `text` (rows with `seq <= cover_seq`
    /// are folded and no longer replayed verbatim).
    pub cover_seq: i64,
    /// Summary text of the covered range (replayed as a leading context block).
    pub text: String,
}

/// One conversation row projected for windowing decisions.
#[derive(Debug, Clone, Copy)]
pub struct RowMeta {
    pub seq: i64,
    /// True when this row starts a new user turn (user role).
    pub is_user: bool,
    /// Approximate replayed length (bytes/chars).
    pub len: usize,
}

/// Rough token estimate: 4 chars per token (OpenAI convention). Used only to
/// gate folding; exact per-model tokenizers vary.
pub fn estimate_tokens(text_len: usize) -> i64 {
    (text_len / 4) as i64
}

/// Choose the fold boundary: keep the newest suffix of whole user-turns whose
/// replayed size fits `budget_chars`; fold the prefix before it. Returns the
/// 0-based index of the last folded row, or `None` when everything already
/// fits (or the newest turn alone is kept).
///
/// Groups are whole user-turns: from each `user` row through the rows before
/// the next `user` row (assistant text + tool pairs belong to the preceding
/// user turn). The boundary therefore never splits a turn.
pub fn select_cover(rows: &[RowMeta], budget_chars: usize) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    // Indexes (into rows) where a user turn starts.
    let starts: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.is_user)
        .map(|(i, _)| i)
        .collect();
    if starts.is_empty() {
        // No user row at all: never fold (nothing meaningful to keep).
        return None;
    }

    // Turn sizes (chars). Turn `k` spans [starts[k], starts[k+1]).
    let mut turn_chars: Vec<usize> = Vec::with_capacity(starts.len());
    for (k, &start) in starts.iter().enumerate() {
        let end = if k + 1 < starts.len() {
            starts[k + 1]
        } else {
            rows.len()
        };
        turn_chars.push(rows[start..end].iter().map(|r| r.len).sum());
    }

    // Walk backward keeping whole turns until adding the next would exceed the
    // budget; always keep at least the newest turn.
    let total: usize = rows.iter().map(|r| r.len).sum();
    if total <= budget_chars {
        return None;
    }
    let mut suffix: usize = 0;
    let mut kept_turns = 0usize;
    for (k, chars) in turn_chars.iter().enumerate().rev() {
        if kept_turns > 0 && suffix + chars > budget_chars {
            break;
        }
        suffix += chars;
        kept_turns += 1;
        let _ = k;
    }
    if kept_turns == turn_chars.len() {
        return None; // even trimming to one keeps nothing older.
    }
    let first_kept = starts.len() - kept_turns;
    if first_kept == 0 {
        return None;
    }
    Some(starts[first_kept].saturating_sub(1))
}

/// Shape the covered prefix rows into the compact transcript text handed to the
/// summarizer (one line per row; tool/assistant content trimmed to stay bounded).
pub fn fold_text(rows: &[(String, String)]) -> String {
    let mut out = String::new();
    for (role, content) in rows {
        let label = match role.as_str() {
            "user" => "user",
            "tool" => "tool",
            _ => "assistant",
        };
        let mut line = content.trim().to_string();
        if line.len() > 2_000 {
            let mut boundary = 2_000;
            while boundary > 0 && !line.is_char_boundary(boundary) {
                boundary -= 1;
            }
            line.truncate(boundary);
            line.push_str(" …");
        }
        if !line.is_empty() {
            out.push_str(&format!("[{label}] {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[(bool, usize)]) -> Vec<RowMeta> {
        v.iter()
            .enumerate()
            .map(|(i, (is_user, len))| RowMeta {
                seq: (i + 1) as i64,
                is_user: *is_user,
                len: *len,
            })
            .collect()
    }

    #[test]
    fn within_budget_folds_nothing() {
        // two turns, 100 chars each, budget 500
        let r = rows(&[(true, 100), (false, 100), (true, 100), (false, 100)]);
        assert_eq!(select_cover(&r, 500), None);
    }

    #[test]
    fn folds_oldest_turns_keeps_newest_suffix() {
        // turn1 (u 200 + a 200), turn2 (u 100 + a 100): total 600, budget 250
        // keep newest turn (200) → fold through index 1 (end of turn1).
        let r = rows(&[(true, 200), (false, 200), (true, 100), (false, 100)]);
        assert_eq!(select_cover(&r, 250), Some(1));
    }

    #[test]
    fn never_folds_everything_keeps_one_turn() {
        // Single huge turn: budget smaller than the turn → still keep it.
        let r = rows(&[(true, 500), (false, 300)]);
        assert_eq!(select_cover(&r, 100), None);
    }

    #[test]
    fn boundary_is_never_inside_a_turn() {
        // three turns; budget such that fold stops at the start of turn 3.
        let r = rows(&[
            (true, 100),
            (false, 300), // turn1 400
            (true, 100),
            (false, 500), // turn2 600
            (true, 100),
            (false, 100), // turn3 200
        ]);
        // total 1100; budget 700 → suffix turn3(200)+turn2(600)=800 >700, so
        // keep only turn3(200) → fold through index 3 (end of turn2).
        assert_eq!(select_cover(&r, 700), Some(3));
    }

    #[test]
    fn estimate_is_chars_over_four() {
        assert_eq!(estimate_tokens(400), 100);
    }
}
