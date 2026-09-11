//! Quota billing for the relay (design §9.3): pre-consume → settle/refund
//! with the atomic token-quota source. Wallet-backed sources land with the
//! relay-station evolution (§16) behind the same call sites.

use crate::db::Driver;
use crate::db::Pool;
use crate::db::driver::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::cache::Pricing;
use crate::llm::models::model::LlmModelType;
use crate::llm::models::model::LlmPriceMode;
use crate::llm::relay::adaptor::RelayUsage;
use crate::types::snowflake_id::SnowflakeId;

/// $1 = 500,000 quota (design §5.4).
pub const QUOTA_PER_USD: f64 = 500_000.0;

/// Pre-consumption hold for one request.
#[derive(Debug, Clone)]
pub struct PreCharge {
    pub token_id: SnowflakeId,
    pub pre_consumed: i64,
    pub unlimited: bool,
}

/// Per-token quota source (design §9.1). Placeholder order matches the SQL
/// text order exactly (bind-position discipline: `Driver::ph` on SQLite is a
/// bare `?`, so text order IS bind order).
async fn debit_hold(pool: &Pool, token_id: SnowflakeId, amount: i64) -> AppResult<()> {
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
async fn apply_settle(pool: &Pool, token_id: SnowflakeId, diff: i64, actual: i64) -> AppResult<()> {
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
async fn apply_refund(pool: &Pool, token_id: SnowflakeId, amount: i64) -> AppResult<()> {
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

/// Pre-consume an estimate before forwarding (design §9.3 formulas; ratio /
/// per-call / embedding variants).
pub async fn pre_consume(
    pool: &Pool,
    token_id: SnowflakeId,
    unlimited: bool,
    estimate: i64,
) -> AppResult<PreCharge> {
    if unlimited || estimate <= 0 {
        return Ok(PreCharge {
            token_id,
            pre_consumed: estimate.max(0),
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
pub async fn settle(pool: &Pool, charge: &PreCharge, actual: i64) {
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
    if charge.unlimited || charge.pre_consumed <= 0 {
        return;
    }
    if let Err(err) = apply_refund(pool, charge.token_id, charge.pre_consumed).await {
        tracing::error!(%err, "llm refund_all failed");
    }
}

/// Pre-consume estimate (design §9.3): ratio / per-call / embedding variants.
pub fn estimate_precharge(
    pricing: &Pricing,
    model_type: LlmModelType,
    body: &serde_json::Value,
) -> i64 {
    match pricing.price_mode {
        LlmPriceMode::PerCall => (pricing.call_price.unwrap_or(0.0) * QUOTA_PER_USD).ceil() as i64,
        LlmPriceMode::Ratio => {
            let prompt_chars = count_prompt_chars(body);
            let mut prompt_est = (prompt_chars / 4).max(500) as f64;
            if has_cjk(body) {
                prompt_est = (prompt_chars as f64 / 1.5).max(500.0);
            }
            if model_type == LlmModelType::Embedding {
                return (prompt_est * pricing.model_ratio).ceil() as i64;
            }
            let max_tokens_eff = body
                .get("max_tokens")
                .and_then(|v| v.as_i64())
                .filter(|v| *v > 0)
                .unwrap_or(4096);
            ((prompt_est + max_tokens_eff as f64) * pricing.model_ratio).ceil() as i64
        }
    }
}

/// Settle quota from normalized usage (design §9.3, cache read/write split).
pub fn settle_quota(pricing: &Pricing, usage: &RelayUsage) -> i64 {
    match pricing.price_mode {
        LlmPriceMode::PerCall => (pricing.call_price.unwrap_or(0.0) * QUOTA_PER_USD).ceil() as i64,
        LlmPriceMode::Ratio => {
            let per = |tokens: i64, ratio: f64| tokens as f64 * ratio / 1_000_000.0 * QUOTA_PER_USD;
            let base =
                (usage.prompt_tokens - usage.cache_read_tokens - usage.cache_write_tokens).max(0);
            let cache_read_ratio = pricing.cache_ratio.unwrap_or(pricing.model_ratio);
            let cache_write_ratio = pricing.cache_write_ratio.unwrap_or(pricing.model_ratio);
            let total = per(base, pricing.model_ratio)
                + per(usage.cache_read_tokens, cache_read_ratio)
                + per(usage.cache_write_tokens, cache_write_ratio)
                + per(usage.completion_tokens, pricing.completion_ratio);
            total.ceil() as i64
        }
    }
}

fn count_prompt_chars(body: &serde_json::Value) -> usize {
    body.get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| match m.get("content") {
                    Some(serde_json::Value::String(s)) => s.chars().count(),
                    Some(serde_json::Value::Array(parts)) => parts
                        .iter()
                        .map(|p| {
                            p.get("text")
                                .and_then(|t| t.as_str())
                                .map(|t| t.chars().count())
                                .unwrap_or(0)
                        })
                        .sum(),
                    _ => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn has_cjk(body: &serde_json::Value) -> bool {
    body.get("messages")
        .and_then(|m| m.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|m| {
                m.get("content")
                    .and_then(|c| c.as_str())
                    .is_some_and(|s| s.chars().any(|ch| ('\u{4E00}'..='\u{9FFF}').contains(&ch)))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing(ratio: f64, completion: f64) -> Pricing {
        Pricing {
            price_mode: LlmPriceMode::Ratio,
            model_ratio: ratio,
            completion_ratio: completion,
            cache_ratio: None,
            cache_write_ratio: None,
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

    #[test]
    fn precharge_ratio_uses_default_max_tokens_chain() {
        // 100 ascii chars → est 25 → floor 500; +4096 default; ×1 ratio.
        let p = pricing(1.0, 1.0);
        let body = chat_body(&"a".repeat(100), 0);
        // max_tokens: 0 present → treated as explicit 0? chain: req → dir → 4096.
        // A zero in the request is treated as absent → 4096.
        let est = estimate_precharge(&p, LlmModelType::Chat, &body);
        assert_eq!(est, 500 + 4096);
    }

    #[test]
    fn precharge_respects_explicit_max_tokens() {
        let p = pricing(2.0, 2.0);
        let body = chat_body(&"a".repeat(100), 1000);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body);
        assert_eq!(est, (500 + 1000) * 2);
    }

    #[test]
    fn precharge_embedding_skips_output_reserve() {
        let p = pricing(1.0, 1.0);
        let body = chat_body(&"a".repeat(100), 4096);
        let est = estimate_precharge(&p, LlmModelType::Embedding, &body);
        assert_eq!(est, 500, "embedding holds prompt only (§9.3)");
    }

    #[test]
    fn precharge_cjk_counts_denser() {
        let p = pricing(1.0, 1.0);
        // 100 CJK → /1.5 = 66 → floor 500; +1000.
        let body = chat_body(&"中".repeat(100), 1000);
        let est = estimate_precharge(&p, LlmModelType::Chat, &body);
        assert_eq!(est, 1500);
        // 2000 CJK → 1333.33 (>500, no floor) + 1000 → ceil 2334.
        let body2 = chat_body(&"中".repeat(2000), 1000);
        let est2 = estimate_precharge(&p, LlmModelType::Chat, &body2);
        assert_eq!(est2, f64::ceil((2000.0 / 1.5) + 1000.0) as i64);
    }

    #[test]
    fn precharge_per_call_is_flat_full_amount() {
        let mut p = pricing(0.0, 0.0);
        p.price_mode = LlmPriceMode::PerCall;
        p.call_price = Some(0.002);
        let body = chat_body("x", 99999);
        assert_eq!(estimate_precharge(&p, LlmModelType::Chat, &body), 1000);
    }

    #[test]
    fn settle_quota_splits_cache_read_write() {
        let mut p = pricing(1.0, 2.0);
        p.cache_ratio = Some(0.1);
        p.cache_write_ratio = Some(1.25);
        let usage = RelayUsage {
            prompt_tokens: 1000,
            completion_tokens: 500,
            cache_read_tokens: 600,
            cache_write_tokens: 200,
        };
        let quota = settle_quota(&p, &usage);
        // base = 1000-600-200 = 200 ×1.0; read 600×0.1; write 200×1.25;
        // completion 500×2.0 → (200+60+250+1000)/1M ×500k = 1510/2 = 755.
        let expected = 200.0 + 60.0 + 250.0 + 1000.0;
        assert_eq!(
            quota,
            (expected / 1_000_000.0 * QUOTA_PER_USD).ceil() as i64
        );
    }

    #[test]
    fn settle_quota_clamps_base_but_still_bills_cache() {
        let p = pricing(1.0, 1.0);
        let usage = RelayUsage {
            prompt_tokens: 100,
            completion_tokens: 0,
            cache_read_tokens: 500,
            cache_write_tokens: 500,
        };
        // Inconsistent usage: base clamps to 0 (never negative, §9.3), but
        // the reported cache tokens still bill at their ratios.
        assert_eq!(settle_quota(&p, &usage), 500);
    }

    #[test]
    fn settle_quota_per_call() {
        let mut p = pricing(0.0, 0.0);
        p.price_mode = LlmPriceMode::PerCall;
        p.call_price = Some(0.01);
        let usage = RelayUsage::default();
        assert_eq!(settle_quota(&p, &usage), 5000);
    }
}
