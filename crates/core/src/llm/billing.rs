//! Internal-consumption billing (design §10.3): **metering is unconditional**
//! — `llm_logs.quota` always carries the real settled value so "用了多少" is
//! always known; the tenant policy only decides whether the wallet is
//! actually charged.
//!
//! Two tenant modes (`llm.billing.mode`, tenant option → global fallback):
//! - `free` (default): LLM is a base service — no charge. Optional per-user
//!   daily cap `llm.billing.free_daily_user_quota` (quota units; 0/absent =
//!   unlimited); the cap reads the day aggregate of `llm_logs`.
//! - `metered`: strict pre-hold on the wallet (`llm_hold`) sized by the
//!   modality estimate, post-settle refunds/collects the diff (`llm_settle`);
//!   zero balance stops service at the pre-check.
//!
//! System-triggered calls (no caller identity) bypass billing entirely.
//!
//! Currency: `llm.billing.currency` tenant option → global → `"CNY"`
//! ([自造-钉死] fallback; flip to an explicit option when a deployment needs
//! another default).
//!
//! Unit conversion: llm quota is USD×1e6 (`QUOTA_PER_USD`), wallet `Price` is
//! cents — $1 = 1e6 quota = 100¢ → `Price = quota / 10_000` (floor; loss
//! bounded below 1e-4 ¢ per call, always in the platform's favor).

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::cache::Pricing;
use crate::llm::relay::adaptor::RelayUsage;
use crate::llm::relay::billing::settle_quota;
use crate::types::price::Price;
use crate::types::quota::Quota;
use crate::types::snowflake_id::SnowflakeId;

/// quota → cent divisor: 1_000_000 quota per USD ÷ 100 cents per USD.
pub const QUOTA_PER_CENT: i64 = 10_000;

/// Floor conversion (platform-favoring); negative input clamps to zero.
pub fn quota_to_price(quota: i64) -> Price {
    Price((quota / QUOTA_PER_CENT).max(0))
}

const MODE_KEY: &str = "llm.billing.mode";
const FREE_DAILY_KEY: &str = "llm.billing.free_daily_user_quota";
const CURRENCY_KEY: &str = "llm.billing.currency";
const FALLBACK_CURRENCY: &str = "CNY";

/// Tenant billing policy (options `llm.billing.*`, tenant → global).
#[derive(Debug, Clone)]
pub enum BillingMode {
    /// Base service: no charge; optional per-user daily cap (quota units).
    Free { daily_user_quota: Option<i64> },
    /// Real billing: wallet pre-hold + settle in the given currency.
    Metered { currency: String },
}

/// One call's billing plan, resolved pre-dispatch and carried by the kernel.
pub struct InternalBilling {
    pub user: SnowflakeId,
    pub currency: String,
    pub pricing: Pricing,
    /// Sell multiplier; internal calls have no sk- token group → 1.0.
    pub group_ratio: f64,
    pub mode: BillingMode,
    /// Pre-held amount (0 = free mode / zero-price estimate).
    pub hold_price: Price,
    /// Idempotency key tying hold + settle (`llm-{snowflake}`).
    pub hold_no: String,
    /// Modality estimate (hold sizing + free-mode cap check); separate from
    /// the actual-usage cell which the facade fills from the response.
    pub estimate: RelayUsage,
    /// Filled by the facade closure from the actual response.
    pub usage: std::sync::Arc<std::sync::Mutex<Option<RelayUsage>>>,
}

impl InternalBilling {
    pub fn new(
        user: SnowflakeId,
        currency: String,
        pricing: Pricing,
        mode: BillingMode,
        estimate: RelayUsage,
    ) -> Self {
        let hold_price = match &mode {
            BillingMode::Metered { .. } => quota_to_price(settle_quota(&pricing, &estimate, 1.0).0),
            BillingMode::Free { .. } => Price(0),
        };
        Self {
            user,
            currency,
            pricing,
            group_ratio: 1.0,
            mode,
            hold_price,
            hold_no: format!("llm-{}", crate::utils::id::new_snowflake_id()),
            estimate,
            usage: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Actual usage filled by the facade from the response; modalities whose
    /// responses carry no usage fall back to the estimate (§8.4 sibling).
    fn take_usage(&self) -> RelayUsage {
        self.usage
            .lock()
            .ok()
            .and_then(|mut c| c.take())
            .or_else(|| Some(self.estimate.clone()))
            .unwrap_or_default()
    }

    /// Log-side quota preview — does not consume the actual-usage cell
    /// (the wallet settle inside [`Self::settle`] re-derives the same value).
    pub fn quota_of_preview(&self) -> Quota {
        let usage = self
            .usage
            .lock()
            .ok()
            .and_then(|c| c.clone())
            .or_else(|| Some(self.estimate.clone()));
        self.quota_of(&usage.unwrap_or_default())
    }

    fn quota_of(&self, usage: &RelayUsage) -> crate::types::quota::Quota {
        settle_quota(&self.pricing, usage, self.group_ratio)
    }

    /// Success (or closure-local failure after the upstream answered):
    /// settle the wallet diff (metered) and return the settled quota for
    /// the log row.
    pub(crate) async fn settle(self, pool: &Pool, tenant: Option<&str>) -> Quota {
        let usage = self.take_usage();
        let quota = self.quota_of(&usage);
        if let BillingMode::Metered { .. } = self.mode {
            let actual = quota_to_price(quota.0);
            if let Err(err) = crate::services::wallet::llm_settle(
                pool,
                tenant,
                self.user,
                &self.currency,
                self.hold_price,
                actual,
                &self.hold_no,
            )
            .await
            {
                tracing::warn!(%err, "llm internal settle failed (logged, not chased)");
            }
        }
        quota
    }

    /// All attempts failed: nothing was consumed — refund the full hold.
    pub(crate) async fn refund(self, pool: &Pool, tenant: Option<&str>) {
        if let BillingMode::Metered { .. } = self.mode
            && self.hold_price.0 > 0
            && let Err(err) = crate::services::wallet::llm_settle(
                pool,
                tenant,
                self.user,
                &self.currency,
                self.hold_price,
                Price(0),
                &self.hold_no,
            )
            .await
        {
            tracing::error!(%err, "llm internal hold refund failed");
        }
    }
}

async fn option_of(pool: &Pool, key: &str, scopes: [Option<&str>; 2]) -> AppResult<Option<String>> {
    for scope in scopes {
        if let Some(row) = crate::models::options::find_by_key(pool, key, scope).await? {
            // 数值/布尔种子以 JSON 标量落地（"100" → Number），字符串原样
            // —— 与 read_group_ratios 的双分支解码同一处理。
            let v = match &row.value {
                serde_json::Value::String(s) => s.trim().to_owned(),
                other => other.to_string(),
            };
            if !v.is_empty() {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

/// Resolve the tenant billing policy (`tenant` option → global option →
/// defaults: free/unlimited).
pub async fn resolve_policy(pool: &Pool, tenant: &str) -> AppResult<BillingMode> {
    let mode = option_of(pool, MODE_KEY, [Some(tenant), None]).await?;
    match mode.as_deref() {
        Some("metered") => {
            let currency = option_of(pool, CURRENCY_KEY, [Some(tenant), None])
                .await?
                .unwrap_or_else(|| FALLBACK_CURRENCY.to_owned());
            Ok(BillingMode::Metered { currency })
        }
        // "free" / unset / unknown → free with unlimited unless capped.
        _ => {
            let raw = option_of(pool, FREE_DAILY_KEY, [Some(tenant), None]).await?;
            let daily_user_quota = raw.and_then(|v| v.parse::<i64>().ok()).filter(|v| *v > 0);
            Ok(BillingMode::Free { daily_user_quota })
        }
    }
}

/// Sum of the user's settled quota for the natural day (free-mode cap
/// source). Reads the `llm_logs` day aggregate — no counter table until
/// volume demands one.
pub async fn daily_used_quota(pool: &Pool, user: SnowflakeId, day: &str) -> AppResult<i64> {
    use crate::db::Driver;
    use crate::db::driver::DbDriver;
    let ph = |i: usize| Driver::ph(i);
    let sql = format!(
        "SELECT COALESCE(SUM(quota), 0) FROM llm_logs \
         WHERE user_id = {u} AND day = {d}",
        u = ph(1),
        d = ph(2)
    );
    let used: Option<i64> = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .bind(user)
        .bind(day)
        .fetch_one(pool)
        .await?;
    Ok(used.unwrap_or(0))
}

/// Free-mode pre-check: the pending estimate counts against the cap so a
/// burst of maxed-out calls cannot sail through on stale aggregates.
pub async fn check_free_limit(
    pool: &Pool,
    user: SnowflakeId,
    day: &str,
    daily_user_quota: i64,
    pending: i64,
) -> AppResult<()> {
    let used = daily_used_quota(pool, user, day).await?;
    if used + pending > daily_user_quota {
        return Err(AppError::TooManyRequests(format!(
            "daily llm quota exceeded for user (limit {daily_user_quota})"
        )));
    }
    Ok(())
}

/// Strict pre-hold (metered): wallet debit refuses negative balances, so an
/// empty wallet stops service right here. Free mode checks the daily cap.
pub async fn preflight(
    pool: &Pool,
    tenant: &str,
    day: &str,
    billing: &InternalBilling,
) -> AppResult<()> {
    match &billing.mode {
        BillingMode::Free { daily_user_quota } => {
            if let Some(limit) = daily_user_quota {
                check_free_limit(
                    pool,
                    billing.user,
                    day,
                    *limit,
                    settle_quota(&billing.pricing, &billing.estimate, billing.group_ratio).0,
                )
                .await?;
            }
            Ok(())
        }
        BillingMode::Metered { currency } => {
            crate::services::wallet::llm_hold(
                pool,
                Some(tenant),
                billing.user,
                currency,
                billing.hold_price,
                &billing.hold_no,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_to_price_floors_in_platform_favor() {
        // 1e6 quota = $1 = 100¢ → 100¢.
        assert_eq!(quota_to_price(1_000_000), Price(100));
        // 10_000 quota = 1¢ exactly.
        assert_eq!(quota_to_price(10_000), Price(1));
        // Sub-cent remainder floors (9_999 quota < 1¢).
        assert_eq!(quota_to_price(9_999), Price(0));
        assert_eq!(quota_to_price(0), Price(0));
        assert_eq!(quota_to_price(-5), Price(0));
    }
}
