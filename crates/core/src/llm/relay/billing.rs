//! Quota billing for the relay (pricing.md §3, design §9.3): pre-consume →
//! settle/refund with the atomic token-quota source, plus the per-request
//! upstream **cost** (pricing.md §7) recorded for profit accounting.
//!
//! Two books, one formula shape (`tokens × USD price × factor`):
//! - user `quota`      = billable USD × `group_ratio` (sell side)
//! - channel `cost_quota` = billable USD × `cost_discount` (buy side; 0 for
//!   subscription/fixed upstreams whose per-token cost is undefined)

use crate::db::Driver;
use crate::db::Pool;
use crate::db::driver::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::cache::Pricing;
use crate::llm::models::channel::LlmCostMode;
use crate::llm::models::model::{LlmModelType, LlmPriceMode};
use crate::llm::relay::adaptor::RelayUsage;
use crate::types::quota::Quota;
use crate::types::snowflake_id::SnowflakeId;

/// Pre-consumption hold for one request.
#[derive(Debug, Clone)]
pub struct PreCharge {
    pub token_id: SnowflakeId,
    pub pre_consumed: Quota,
    pub unlimited: bool,
}

/// Per-token quota source (design §9.1). Placeholder order matches the SQL
/// text order exactly (bind-position discipline: `Driver::ph` on SQLite is a
/// bare `?`, so text order IS bind order).
async fn debit_hold(pool: &Pool, token_id: SnowflakeId, amount: Quota) -> AppResult<()> {
    let ph = |i: usize| Driver::ph(i);
    let sql = format!(
        "UPDATE llm_tokens SET remain_quota = remain_quota - {a} \
         WHERE id = {b} AND (unlimited_quota = TRUE OR remain_quota >= {c})",
        a = ph(1),
        b = ph(2),
        c = ph(3)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(amount)
        .bind(token_id)
        .bind(amount)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::TooManyRequests("insufficient quota".to_owned()));
    }
    Ok(())
}

/// Settle: `remain += (pre - actual)` and `used += actual` in one statement.
/// The non-negative guard makes a top-up (actual > pre) best-effort: when the
/// token drained mid-flight the row doesn't go negative and the caller logs
/// (design §9.3 补扣失败不追).
async fn apply_settle(
    pool: &Pool,
    token_id: SnowflakeId,
    diff: Quota,
    actual: Quota,
) -> AppResult<()> {
    let ph = |i: usize| Driver::ph(i);
    let sql = format!(
        "UPDATE llm_tokens SET remain_quota = remain_quota + {d}, used_quota = used_quota + {u} \
         WHERE id = {id} AND remain_quota + {g} >= 0",
        d = ph(1),
        u = ph(2),
        id = ph(3),
        g = ph(4)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(diff)
        .bind(actual)
        .bind(token_id)
        .bind(diff)
        .execute(pool)
        .await?;
    Ok(())
}

/// Refund the full hold (all attempts failed / admission rejected).
async fn apply_refund(pool: &Pool, token_id: SnowflakeId, amount: Quota) -> AppResult<()> {
    let ph = |i: usize| Driver::ph(i);
    let sql = format!(
        "UPDATE llm_tokens SET remain_quota = remain_quota + {a} WHERE id = {b}",
        a = ph(1),
        b = ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(amount)
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Pre-consume an estimate before forwarding (pricing.md §3.2).
pub async fn pre_consume(
    pool: &Pool,
    token_id: SnowflakeId,
    unlimited: bool,
    estimate: Quota,
) -> AppResult<PreCharge> {
    if unlimited || estimate.0 <= 0 {
        return Ok(PreCharge {
            token_id,
            pre_consumed: Quota(estimate.0.max(0)),
            unlimited,
        });
    }
    debit_hold(pool, token_id, estimate).await?;
    Ok(PreCharge {
        token_id,
        pre_consumed: estimate,
        unlimited,
    })
}

/// Settle with the actual usage: refund the over-hold, or try to collect the
/// under-hold (collection failure is logged, not chased — design §9.3).
pub async fn settle(pool: &Pool, charge: &PreCharge, actual: Quota) {
    if charge.unlimited {
        return;
    }
    let diff = charge.pre_consumed - actual;
    if let Err(err) = apply_settle(pool, charge.token_id, diff, actual).await {
        tracing::warn!(%err, "llm settle failed (logged, not chased)");
    }
}

/// Refund the full hold (all attempts failed / admission rejected).
pub async fn refund_all(pool: &Pool, charge: &PreCharge) {
    if charge.unlimited || charge.pre_consumed.0 <= 0 {
        return;
    }
    if let Err(err) = apply_refund(pool, charge.token_id, charge.pre_consumed).await {
        tracing::error!(%err, "llm refund_all failed");
    }
}

/// Pre-consume estimate (pricing.md §3.2): token / per-call / embedding.
/// `max_output_tokens` is the model's `params.max_output_tokens` (the middle
/// step of the request → params → 4096 default chain).
pub fn estimate_precharge(
    pricing: &Pricing,
    model_type: LlmModelType,
    body: &serde_json::Value,
    group_ratio: f64,
    max_output_tokens: Option<i64>,
) -> Quota {
    match pricing.price_mode {
        LlmPriceMode::PerCall => {
            Quota::from_usd_ceil(pricing.call_price.unwrap_or(0.0) * group_ratio)
        }
        LlmPriceMode::Token => {
            if model_type == LlmModelType::Image {
                // Images are billed per generated image (completion side):
                // hold = input×1 + n×output_price. The generic path below
                // would reserve the 4096-token output default — a huge
                // over-hold when output_price is a per-image price.
                let n = body
                    .get("n")
                    .and_then(|v| v.as_i64())
                    .filter(|v| *v > 0)
                    .unwrap_or(1);
                let usd = (pricing.input_price + n as f64 * pricing.output_price) / 1_000_000.0;
                return Quota::from_usd_ceil(usd * group_ratio);
            }
            if model_type == LlmModelType::Tts {
                // TTS bills on request-side input chars (openai: $/1M chars)
                // — known upfront, so hold == settle.
                let chars = count_prompt_chars(body).max(1);
                let usd = chars as f64 * pricing.input_price / 1_000_000.0;
                return Quota::from_usd_ceil(usd * group_ratio);
            }
            let prompt_chars = count_prompt_chars(body);
            let mut prompt_est = (prompt_chars / 4).max(500) as f64;
            if has_cjk(body) {
                prompt_est = (prompt_chars as f64 / 1.5).max(500.0);
            }
            let usd = if matches!(model_type, LlmModelType::Embedding | LlmModelType::Rerank) {
                // No output tokens — don't reserve the 4096 output default.
                prompt_est * pricing.input_price / 1_000_000.0
            } else {
                let max_tokens_eff = body
                    .get("max_tokens")
                    .and_then(|v| v.as_i64())
                    .filter(|v| *v > 0)
                    .or_else(|| max_output_tokens.filter(|v| *v > 0))
                    .unwrap_or(4096);
                (prompt_est * pricing.input_price + max_tokens_eff as f64 * pricing.output_price)
                    / 1_000_000.0
            };
            Quota::from_usd_ceil(usd * group_ratio)
        }
    }
}

/// Billable USD for the normalized usage, before any group/cost multiplier
/// (pricing.md §3.1). `PerCall` returns the flat call price.
fn usage_usd(pricing: &Pricing, usage: &RelayUsage) -> f64 {
    match pricing.price_mode {
        LlmPriceMode::PerCall => pricing.call_price.unwrap_or(0.0),
        LlmPriceMode::Token => {
            let base =
                (usage.prompt_tokens - usage.cache_read_tokens - usage.cache_write_tokens).max(0);
            let cache_read_price = pricing.cache_read_price.unwrap_or(pricing.input_price);
            let cache_write_price = pricing.cache_write_price.unwrap_or(pricing.input_price);
            (base as f64 * pricing.input_price
                + usage.cache_read_tokens as f64 * cache_read_price
                + usage.cache_write_tokens as f64 * cache_write_price
                + usage.completion_tokens as f64 * pricing.output_price)
                / 1_000_000.0
        }
    }
}

/// Settle quota from normalized usage (pricing.md §3.1) — user sell side.
pub fn settle_quota(pricing: &Pricing, usage: &RelayUsage, group_ratio: f64) -> Quota {
    Quota::from_usd_ceil(usage_usd(pricing, usage) * group_ratio)
}

/// Per-request upstream cost (pricing.md §3.1/§7): `usage` channels apply
/// `cost_discount`; `fixed` (subscription) channels are 0 (cost lives in the
/// monthly pool).
pub fn cost_quota(
    pricing: &Pricing,
    usage: &RelayUsage,
    cost_mode: LlmCostMode,
    cost_discount: f64,
) -> Quota {
    match cost_mode {
        LlmCostMode::Fixed => Quota(0),
        LlmCostMode::Usage => Quota::from_usd_ceil(usage_usd(pricing, usage) * cost_discount),
    }
}

/// Count prompt-side content chars across the request's text payload
/// (estimation source for the §8.4 no-usage fallback; CJK-awareness lives
/// in the precharge estimate only). Shape-aware: chat `messages`, embeddings
/// `input` (string or array), rerank `query` + `documents`.
pub(crate) fn count_prompt_chars(body: &serde_json::Value) -> usize {
    payload_texts(body)
        .map(|texts| texts.iter().map(|s| s.chars().count()).sum())
        .unwrap_or(0)
}

/// Borrowed text payload of a relay request body: `messages[].content`
/// (string or content-part arrays), embeddings `input` (string or array),
/// or rerank `query` + `documents[]`. `None` when no known shape matches.
fn payload_texts(body: &serde_json::Value) -> Option<Vec<&str>> {
    if let Some(arr) = body.get("messages").and_then(|m| m.as_array()) {
        let mut out = Vec::with_capacity(arr.len());
        for m in arr {
            match m.get("content") {
                Some(serde_json::Value::String(s)) => out.push(s.as_str()),
                Some(serde_json::Value::Array(parts)) => {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            out.push(t);
                        }
                    }
                }
                _ => {}
            }
        }
        return Some(out);
    }
    // embeddings: input = "text" | ["a", "b"]
    if let Some(input) = body.get("input") {
        return Some(input_texts(input));
    }
    // rerank: query + documents[]
    if body.get("query").is_some() || body.get("documents").is_some() {
        let mut out = Vec::new();
        if let Some(q) = body.get("query").and_then(|q| q.as_str()) {
            out.push(q);
        }
        if let Some(docs) = body.get("documents").and_then(|d| d.as_array()) {
            for d in docs {
                if let Some(s) = d.as_str() {
                    out.push(s);
                }
            }
        }
        return Some(out);
    }
    None
}

fn input_texts(input: &serde_json::Value) -> Vec<&str> {
    match input {
        serde_json::Value::String(s) => vec![s.as_str()],
        serde_json::Value::Array(arr) => {
            arr.iter().filter_map(|v| v.as_str()).collect::<Vec<&str>>()
        }
        _ => Vec::new(),
    }
}

/// Flat token-side quota helper: `tokens × price × group_ratio` (used by the
/// audio paths where the billable unit is seconds/chars, not a JSON body).
pub(crate) fn flat_token_quota(price_per_m: f64, units: i64, group_ratio: f64) -> Quota {
    Quota::from_usd_ceil(units as f64 * price_per_m / 1_000_000.0 * group_ratio)
}

/// Rough audio duration fallback (seconds) from uploaded file size when the
/// upstream response carries no `duration`: ≈32KB/s covers 16kHz·16bit mono
/// PCM and typical speech codecs; never below 1s.
pub(crate) fn estimate_audio_secs(file_bytes: usize) -> i64 {
    ((file_bytes / 32_768) as i64).max(1)
}

fn has_cjk(body: &serde_json::Value) -> bool {
    payload_texts(body).is_some_and(|texts| {
        texts
            .iter()
            .any(|s| s.chars().any(|ch| ('\u{4E00}'..='\u{9FFF}').contains(&ch)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// USD prices: input 2 / output 4 per 1M (pricing.md §2 shape).
    fn pricing(input: f64, output: f64) -> Pricing {
        Pricing {
            price_mode: LlmPriceMode::Token,
            input_price: input,
            output_price: output,
            cache_read_price: None,
            cache_write_price: None,
            call_price: None,
        }
    }

    fn chat_body(prompt: &str, max_tokens: i64) -> serde_json::Value {
        serde_json::json!({
            "model": "m",
            "max_tokens": max_tokens,
            "messages": [{ "role": "user", "content": prompt }]
        })
    }

    fn usage(prompt: i64, completion: i64) -> RelayUsage {
        RelayUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }

    #[test]
    fn precharge_uses_default_max_tokens_chain() {
        // 100 ascii chars → est 25 → floor 500 tokens.
        // USD = (500×$2 + 4096×$4)/1M = (1000 + 16384)/1M = 0.017384
        // quota = 0.017384 × 1M = 17384 (group 1.0).
        let p = pricing(2.0, 4.0);
        let body = chat_body(&"a".repeat(100), 0);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, None);
        assert_eq!(est, Quota(17_384));
    }

    #[test]
    fn precharge_respects_explicit_max_tokens() {
        // (500×2 + 1000×4)/1M = 5000/1M → 5000 quota.
        let p = pricing(2.0, 4.0);
        let body = chat_body(&"a".repeat(100), 1000);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, None);
        assert_eq!(est, Quota(5_000));
    }

    #[test]
    fn precharge_applies_group_ratio() {
        // (500×2 + 1000×4)/1M × 1.5 = 7500 quota.
        let p = pricing(2.0, 4.0);
        let body = chat_body(&"a".repeat(100), 1000);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body, 1.5, None);
        assert_eq!(est, Quota(7_500));
    }

    #[test]
    fn precharge_max_tokens_chain_request_then_params_then_4096() {
        let p = pricing(2.0, 4.0);
        // No request max_tokens → params.max_output_tokens = 2048 wins.
        let body = serde_json::json!({
            "model": "m",
            "messages": [{ "role": "user", "content": "a".repeat(100) }]
        });
        // (500×2 + 2048×4) = 9192 quota.
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, Some(2048)),
            Quota(9_192)
        );
        // Request max_tokens overrides params.
        let body2 = chat_body(&"a".repeat(100), 500);
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body2, 1.0, Some(2048)),
            Quota(3_000)
        );
        // params zero/absent → 4096 default.
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, Some(0)),
            Quota(17_384)
        );
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, None),
            Quota(17_384)
        );
    }

    #[test]
    fn precharge_embedding_skips_output_reserve() {
        // 500×0.02/1M × 1M = 10 quota (ceil).
        let p = pricing(0.02, 0.02);
        let body = chat_body(&"a".repeat(100), 4096);
        let est = estimate_precharge(&p, LlmModelType::Embedding, &body, 1.0, None);
        assert_eq!(est, Quota(10), "embedding holds prompt only (§9.3)");
    }

    #[test]
    fn precharge_cjk_counts_denser() {
        let p = pricing(2.0, 4.0);
        // 100 CJK → 100/1.5=66.7 → floor 500; +1000 tokens.
        let body = chat_body(&"中".repeat(100), 1000);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, None);
        assert_eq!(est, Quota(5_000));
        // 2000 CJK → 1333.33 tokens (no floor) + 1000.
        let body2 = chat_body(&"中".repeat(2000), 1000);
        let est2 = estimate_precharge(&p, LlmModelType::Chat, &body2, 1.0, None);
        // (1333.333×2 + 1000×4) = 6666.667 USD×1M/1M → ceil 6667.
        assert_eq!(est2, Quota(6_667));
    }

    #[test]
    fn precharge_per_call_is_flat_full_amount() {
        let mut p = pricing(0.0, 0.0);
        p.price_mode = LlmPriceMode::PerCall;
        p.call_price = Some(0.002);
        let body = chat_body("x", 99999);
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body, 1.0, None),
            Quota(2_000)
        );
        // Group ratio applies on per-call too.
        assert_eq!(
            estimate_precharge(&p, LlmModelType::Chat, &body, 1.5, None),
            Quota(3_000)
        );
    }

    #[test]
    fn settle_quota_splits_cache_read_write() {
        let mut p = pricing(2.0, 4.0);
        p.cache_read_price = Some(0.2);
        p.cache_write_price = Some(2.5);
        let u = RelayUsage {
            prompt_tokens: 1000,
            completion_tokens: 500,
            cache_read_tokens: 600,
            cache_write_tokens: 200,
        };
        // base = 1000-600-200 = 200 ×$2 ; read 600×$0.2 ; write 200×$2.5 ;
        // completion 500×$4 → USD = (400 + 120 + 500 + 2000)/1M = 3020/1M
        // quota (group 1.0) = 3020.
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(3_020));
        assert_eq!(settle_quota(&p, &u, 1.5), Quota(4_530));
    }

    #[test]
    fn settle_quota_defaults_cache_prices_to_input() {
        let p = pricing(2.0, 4.0);
        let u = RelayUsage {
            prompt_tokens: 1000,
            completion_tokens: 0,
            cache_read_tokens: 1000,
            cache_write_tokens: 0,
        };
        // cache_read falls back to input 2 → 1000×2/1M = 2000 quota.
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(2_000));
    }

    #[test]
    fn settle_quota_clamps_base_but_still_bills_cache() {
        let p = pricing(1.0, 1.0);
        let u = RelayUsage {
            prompt_tokens: 100,
            completion_tokens: 0,
            cache_read_tokens: 500,
            cache_write_tokens: 500,
        };
        // base clamps to 0; read 500×1 + write 500×1 = 1000 → 1000/1M ×1M.
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(1_000));
    }

    #[test]
    fn cost_quota_usage_applies_discount() {
        let p = pricing(2.0, 4.0);
        let u = usage(1_000_000, 500_000);
        // billable USD = (1M×2 + 0.5M×4)/1M = $4 → 4,000,000 quota at 1.0.
        assert_eq!(
            cost_quota(&p, &u, LlmCostMode::Usage, 1.0),
            Quota(4_000_000)
        );
        // 8折 = $3.2 → 3,200,000.
        assert_eq!(
            cost_quota(&p, &u, LlmCostMode::Usage, 0.8),
            Quota(3_200_000)
        );
    }

    #[test]
    fn cost_quota_fixed_upstream_is_zero() {
        let p = pricing(2.0, 4.0);
        let u = usage(1_000_000, 500_000);
        assert_eq!(cost_quota(&p, &u, LlmCostMode::Fixed, 0.8), Quota(0));
    }

    #[test]
    fn profit_is_sell_minus_cost() {
        let p = pricing(2.0, 4.0);
        let u = usage(1_000_000, 500_000);
        let sell = settle_quota(&p, &u, 1.5); // $6 → 6,000,000
        let cost = cost_quota(&p, &u, LlmCostMode::Usage, 0.8); // $3.2 → 3,200,000
        assert_eq!(sell - cost, Quota(2_800_000)); // $2.8 profit
    }

    #[test]
    fn quota_usd_roundtrip_is_exact() {
        // The 1M cancellation: quota == tokens × USD-price for 1M tokens.
        let p = pricing(2.5, 10.0);
        let u = usage(1_000_000, 0);
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(2_500_000)); // $2.5
    }

    // ── exhaustive pricing matrix (pricing.md §7 example made precise) ──

    /// The canonical worked example: input $2.5, output $10, cache-read
    /// $0.5, cache-write $3.0; usage 700 base + 200 read + 100 write + 500
    /// output → billable $0.00715. Sell group 1.5, cost discount 0.8.
    fn example_pricing() -> Pricing {
        let mut p = pricing(2.5, 10.0);
        p.cache_read_price = Some(0.5);
        p.cache_write_price = Some(3.0);
        p
    }

    fn example_usage() -> RelayUsage {
        RelayUsage {
            prompt_tokens: 1000, // includes cache
            completion_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 100,
        }
    }

    #[test]
    fn exact_charge_example() {
        // 700×2.5 + 200×0.5 + 100×3.0 + 500×10 = 7150 (USD×1M) = $0.00715.
        let sell = settle_quota(&example_pricing(), &example_usage(), 1.5);
        assert_eq!(sell, Quota(10_725)); // $0.010725
        assert_eq!(sell.as_usd(), 0.010725);
    }

    #[test]
    fn exact_cost_example() {
        // $0.00715 × 0.8 = $0.00572.
        let cost = cost_quota(
            &example_pricing(),
            &example_usage(),
            LlmCostMode::Usage,
            0.8,
        );
        assert_eq!(cost, Quota(5_720));
        assert_eq!(cost.as_usd(), 0.00572);
    }

    #[test]
    fn exact_profit_example() {
        let sell = settle_quota(&example_pricing(), &example_usage(), 1.5);
        let cost = cost_quota(
            &example_pricing(),
            &example_usage(),
            LlmCostMode::Usage,
            0.8,
        );
        assert_eq!(sell - cost, Quota(5_005)); // $0.005005
    }

    #[test]
    fn explicit_zero_cache_price_is_free_not_fallback() {
        // Some(0.0) means "cache is free" — must NOT fall back to input price.
        let mut p = pricing(10.0, 10.0);
        p.cache_read_price = Some(0.0);
        p.cache_write_price = Some(0.0);
        let u = RelayUsage {
            prompt_tokens: 1000,
            completion_tokens: 0,
            cache_read_tokens: 1000,
            cache_write_tokens: 0,
        };
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(0));
    }

    #[test]
    fn missing_cache_prices_fall_back_to_input() {
        let p = pricing(3.0, 1.0);
        let u = RelayUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            cache_read_tokens: 100,
            cache_write_tokens: 100,
        };
        // both fall back to 3.0 → 300+300 = 600.
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(600));
    }

    #[test]
    fn fractional_quota_rounds_up_never_undercharges() {
        // 333 tokens × $1/1M = 0.000333 → ceil to 333 quota exactly.
        let p = pricing(1.0, 0.0);
        let u = usage(333, 0);
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(333));
        // A sub-quota fraction (0.5 token of $1/1M) still bills 1 quota.
        let p2 = pricing(1_000_000.0, 0.0); // $1 per single token
        let u2 = usage(1, 0);
        assert_eq!(settle_quota(&p2, &u2, 0.5), Quota(500_000));
    }

    #[test]
    fn zero_price_model_bills_zero() {
        let p = pricing(0.0, 0.0);
        let u = usage(1_000_000, 1_000_000);
        assert_eq!(settle_quota(&p, &u, 1.5), Quota(0));
        assert_eq!(cost_quota(&p, &u, LlmCostMode::Usage, 0.8), Quota(0));
    }

    #[test]
    fn zero_group_ratio_is_free() {
        let p = pricing(2.5, 10.0);
        let u = usage(1_000_000, 1_000_000);
        assert_eq!(settle_quota(&p, &u, 0.0), Quota(0));
        // Cost still accrues — giving a group away is a real loss.
        assert_eq!(
            cost_quota(&p, &u, LlmCostMode::Usage, 1.0),
            Quota(12_500_000)
        );
    }

    #[test]
    fn discount_above_one_yields_negative_profit() {
        // Bought above list (bad deal): cost > revenue.
        let p = pricing(2.0, 2.0);
        let u = usage(1_000_000, 0);
        let sell = settle_quota(&p, &u, 1.0);
        let cost = cost_quota(&p, &u, LlmCostMode::Usage, 1.2);
        assert_eq!(sell, Quota(2_000_000));
        assert_eq!(cost, Quota(2_400_000));
        assert_eq!(sell - cost, Quota(-400_000));
    }

    #[test]
    fn per_call_charge_and_cost_exact() {
        let mut p = pricing(0.0, 0.0);
        p.price_mode = LlmPriceMode::PerCall;
        p.call_price = Some(0.02);
        let u = usage(999_999, 999_999); // ignored in per_call mode
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(20_000)); // $0.02
        assert_eq!(settle_quota(&p, &u, 1.5), Quota(30_000)); // $0.03
        assert_eq!(cost_quota(&p, &u, LlmCostMode::Usage, 0.7), Quota(14_000));
        assert_eq!(cost_quota(&p, &u, LlmCostMode::Fixed, 0.7), Quota(0));
    }

    #[test]
    fn per_call_estimate_equals_settle() {
        let mut p = pricing(0.0, 0.0);
        p.price_mode = LlmPriceMode::PerCall;
        p.call_price = Some(0.03);
        let body = chat_body("hello", 100);
        let pre = estimate_precharge(&p, LlmModelType::Chat, &body, 1.2, None);
        let actual = settle_quota(&p, &usage(50, 50), 1.2);
        assert_eq!(pre, actual, "per-call hold covers the flat settle");
        assert_eq!(pre, Quota(36_000));
    }

    #[test]
    fn cost_quota_fixed_ignores_usage_entirely() {
        assert_eq!(
            cost_quota(
                &example_pricing(),
                &example_usage(),
                LlmCostMode::Fixed,
                0.0
            ),
            Quota(0)
        );
    }

    #[test]
    fn large_usage_stays_exact() {
        // 100M input + 50M output at $2.5/$10 → $250 + $500 = $750.
        let p = pricing(2.5, 10.0);
        let u = usage(100_000_000, 50_000_000);
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(750_000_000));
    }

    #[test]
    fn generic_output_uses_output_price_not_input() {
        // Regression guard: output must not be billed at input price.
        let p = pricing(1.0, 100.0);
        let u = usage(0, 1_000_000);
        assert_eq!(settle_quota(&p, &u, 1.0), Quota(100_000_000));
    }
}
