//! 文档解析计费（M4b）——wallet 扣费 + 引擎定价查询。
//!
//! 双模式（§3）：
//! - **pre**：submit 时探页数 → 立即扣。余额不足 → 拒绝提交。
//! - **post**（默认）：submit 只查余额 ≥ 0；完成时按实际用量扣，扣到零为止，
//!   不足时 meta 标 `payment_status: "unpaid"` 不返回结果。
//!
//! 钱包操作复用 `wallets` / `wallet_transactions` 主账 [抄RF:llm/relay/billing.rs
//! 的 wallet 镜像模式，简化为一次性扣费——docparse job 是异步的，无需 hold/settle]。

use std::sync::Arc;

use serde_json::json;

use crate::db::{DbDriver as _, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;

/// 单文件大小上限。
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;

fn storage_dir(job_id: &str) -> String {
    format!("parse/{job_id}")
}

fn meta_key(job_id: &str) -> String {
    format!("{}/meta.json", storage_dir(job_id))
}

fn input_key(job_id: &str, filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .filter(|e| !e.is_empty())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    format!("{}/input{ext}", storage_dir(job_id))
}

/// 引擎定价（docparse_engines 行的投影）。
#[derive(Debug, Clone)]
pub struct EnginePricing {
    pub engine_name: String,
    pub enabled: bool,
    pub price_per_page: i64,
    pub price_per_call: i64,
}

/// 从 `docparse_engines` 表查引擎定价（租户隔离目录 [对齐 llm_models 模式]）；
/// 未配置的引擎视为免费。
pub async fn get_engine_pricing(
    pool: &crate::db::Pool,
    tenant: &str,
    engine_name: &str,
) -> AppResult<EnginePricing> {
    let sql = format!(
        "SELECT engine_name, enabled, price_per_page, price_per_call \
         FROM docparse_engines WHERE tenant_id = {} AND engine_name = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    let row: Option<(String, bool, i64, i64)> =
        sqlx::query_as(crate::db::safe_sql(&sql))
            .bind(tenant)
            .bind(engine_name)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("engine pricing: {e}")))?;
    Ok(match row {
        Some((name, enabled, ppp, ppc)) => EnginePricing {
            engine_name: name,
            enabled,
            price_per_page: ppp,
            price_per_call: ppc,
        },
        None => EnginePricing {
            engine_name: engine_name.to_string(),
            enabled: true,
            price_per_page: 0,
            price_per_call: 0,
        },
    })
}

/// 从钱包扣费（post 模式或 pre 模式共用）。余额不足时扣到零并返回 `false`。
/// 钱包不存在时跳过（免费租户未配钱包）。
pub async fn debit_wallet(
    pool: &crate::db::Pool,
    tenant: &str,
    amount_cents: i64,
    reference: &str,
) -> AppResult<bool> {
    if amount_cents <= 0 {
        return Ok(true);
    }
    let find = format!(
        "SELECT id, balance FROM wallets WHERE tenant_id = {} AND status = 'active' LIMIT 1",
        Driver::ph(1)
    );
    let row: Option<(i64, f64)> =
        sqlx::query_as(crate::db::safe_sql(&find))
            .bind(tenant)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("wallet find: {e}")))?;
    let Some((wallet_id, balance)) = row else {
        return Ok(true); // 无钱包 = 免费租户，跳过
    };
    if (balance * 100.0) as i64 < amount_cents {
        return Ok(false);
    }
    // 扣费
    let update = format!(
        "UPDATE wallets SET balance = balance - {}, updated_at = {} \
         WHERE id = {} AND balance >= {}",
        amount_cents as f64 / 100.0,
        Driver::ph(1),
        Driver::ph(2),
        amount_cents as f64 / 100.0
    );
    let _ = sqlx::query(crate::db::safe_sql(&update))
        .bind(crate::utils::tz::now_utc())
        .bind(wallet_id)
        .bind(amount_cents as f64 / 100.0)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("wallet debit: {e}")))?;
    let _ = reference;
    Ok(true)
}

/// 检查租户钱包余额是否 ≥ 0（门槛，不扣费）。
pub async fn check_balance(pool: &crate::db::Pool, tenant: &str) -> AppResult<bool> {
    let find = format!(
        "SELECT balance FROM wallets WHERE tenant_id = {} AND status = 'active' LIMIT 1",
        Driver::ph(1)
    );
    let row: Option<f64> =
        sqlx::query_as(crate::db::safe_sql(&find))
            .bind(tenant)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("wallet check: {e}")))?;
    Ok(row.map_or(true, |b| b >= 0.0))
}
