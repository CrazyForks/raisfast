//! Redemption codes (wallet domain, design.md §定稿决策: llm 底座不做兑换码,
//! wallet 域发码/核销后 `credit wallet`,llm 无感知)。参考 new-api redemption
//! code 模型:码明文只显示一次(库存 hash),`user_id NULL` = 公共码(谁兑谁得),
//! 有值 = 定向码(仅该用户可兑);激活时记录 `redeemed_by` / `redeemed_at`。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    RedemptionCodeStatus {
        Pending = "pending",
        Redeemed = "redeemed",
        Disabled = "disabled",
    }
);

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct RedemptionCode {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub code_hash: String,
    /// 明文可逆加密(APP_KEY),供管理端列表展示——同 llm_tokens.key_enc 先例。
    pub code_enc: Option<String>,
    /// 定向码的属主;NULL = 公共码(激活时绑定首个兑换者)。
    pub user_id: Option<SnowflakeId>,
    pub currency: String,
    pub amount: Price,
    pub status: RedemptionCodeStatus,
    pub redeemed_by: Option<SnowflakeId>,
    pub redeemed_at: Option<Timestamp>,
    pub redemption_tx_no: Option<String>,
    pub created_by: Option<SnowflakeId>,
    /// 过期时间(UTC);过期后不可兑换。
    pub expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Insert one code row (hash only — plaintext never stored).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    code_hash: &str,
    user_id: Option<SnowflakeId>,
    currency: &str,
    amount: Price,
    created_by: Option<SnowflakeId>,
    expires_at: Option<Timestamp>,
    code_enc: Option<String>,
) -> AppResult<RedemptionCode> {
    let id = crate::utils::id::new_snowflake_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(pool, "redemption_codes", [
        "id" => id,
        "code_hash" => code_hash,
        "code_enc" => code_enc,
        "user_id" => user_id,
        "currency" => currency,
        "amount" => amount,
        "status" => RedemptionCodeStatus::Pending.as_str(),
        "created_by" => created_by,
        "expires_at" => expires_at,
        "created_at" => now,
        "updated_at" => now
    ], tenant: tenant_id)?;
    raisfast_derive::crud_find_one!(pool, "redemption_codes", RedemptionCode, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

/// Id lookup (admin activate path).
pub async fn find_by_id(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
) -> AppResult<RedemptionCode> {
    raisfast_derive::crud_find_one!(pool, "redemption_codes", RedemptionCode, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

/// Hash lookup — the redeem path's entry point.
pub async fn find_by_hash(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    code_hash: &str,
) -> AppResult<Option<RedemptionCode>> {
    raisfast_derive::crud_find!(pool, "redemption_codes", RedemptionCode, where: ("code_hash", code_hash), tenant: tenant_id)
        .map_err(Into::into)
}

/// Paged admin list with optional owner / status filters (dynamic paged
/// query, models/wallet.rs 先例).
pub async fn query_paged(
    pool: &crate::db::Pool,
    filters: &RedemptionCodeFilters,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<RedemptionCode>, i64)> {
    use crate::db::driver::DbDriver;
    let ph = crate::db::Driver::ph;
    let mut idx = 1usize;
    let mut conds = String::new();
    if filters.user_id.is_some() {
        conds.push_str(&format!(" AND user_id = {}", ph(idx)));
        idx += 1;
    }
    if filters
        .status
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        conds.push_str(&format!(" AND status = {}", ph(idx)));
        idx += 1;
    }
    let tenant = if tenant_id.is_some() {
        let frag = format!(" AND tenant_id = {}", ph(idx));
        idx += 1;
        frag
    } else {
        String::new()
    };

    let offset = (page - 1).max(0) * page_size;
    let data_sql = format!(
        "SELECT * FROM redemption_codes WHERE 1=1{conds}{tenant} \
         ORDER BY created_at DESC, id DESC LIMIT {lim} OFFSET {off}",
        lim = ph(idx),
        off = ph(idx + 1)
    );
    let count_expr = crate::db::Driver::cast_int("COUNT(*)");
    let count_sql = format!("SELECT {count_expr} FROM redemption_codes WHERE 1=1{conds}{tenant}");

    let mut dq =
        sqlx::query_as::<crate::db::pool::Db, RedemptionCode>(crate::db::safe_sql(&data_sql));
    let mut cq = sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&count_sql));
    if let Some(u) = filters.user_id {
        dq = dq.bind(u);
        cq = cq.bind(u);
    }
    if let Some(s) = &filters.status
        && !s.trim().is_empty()
    {
        dq = dq.bind(s.trim());
        cq = cq.bind(s.trim());
    }
    if tenant_id.is_some() {
        let tv = crate::db::tenant::resolve_tenant(tenant_id);
        dq = dq.bind(tv);
        cq = cq.bind(tv);
    }
    dq = dq.bind(page_size).bind(offset);
    let data = dq.fetch_all(pool).await?;
    let total = cq.fetch_one(pool).await?;
    Ok((data, total))
}

pub struct RedemptionCodeFilters {
    pub user_id: Option<SnowflakeId>,
    pub status: Option<String>,
}

/// Atomic activation: `pending → redeemed` CAS, binding the redeemer and the
/// activation time. Public codes (user_id NULL) get bound to `bind_user` on
/// first activation. `rows_affected == 0` means someone else got there first
/// (or the code is not redeemable) — the caller decides the error.
pub async fn tx_activate(
    tx: &mut crate::db::pool::DbConnection,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    redeemed_by: SnowflakeId,
    redemption_tx_no: &str,
    bind_user: Option<SnowflakeId>,
) -> AppResult<bool> {
    use crate::db::driver::DbDriver;
    let ph = crate::db::Driver::ph;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE redemption_codes SET status = {}, redeemed_by = {}, redeemed_at = {}, \
         redemption_tx_no = {}, user_id = COALESCE(user_id, {}), updated_at = {} \
         WHERE id = {} AND status = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8)
    );
    let status = RedemptionCodeStatus::Redeemed.as_str();
    let result: crate::db::pool::DbQueryResult = sqlx::query(crate::db::safe_sql(&sql))
        .bind(status)
        .bind(redeemed_by)
        .bind(now)
        .bind(redemption_tx_no)
        .bind(bind_user)
        .bind(now)
        .bind(id)
        .bind(RedemptionCodeStatus::Pending.as_str())
        .execute(&mut *tx)
        .await?;
    let _ = tenant_id;
    Ok(result.rows_affected() > 0)
}

/// Admin re-enable: `disabled → pending` CAS.
pub async fn tx_enable(tx: &mut crate::db::pool::DbConnection, id: SnowflakeId) -> AppResult<bool> {
    use crate::db::driver::DbDriver;
    let ph = crate::db::Driver::ph;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE redemption_codes SET status = {}, updated_at = {} \
         WHERE id = {} AND status = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4)
    );
    let result: crate::db::pool::DbQueryResult = sqlx::query(crate::db::safe_sql(&sql))
        .bind(RedemptionCodeStatus::Pending.as_str())
        .bind(now)
        .bind(id)
        .bind(RedemptionCodeStatus::Disabled.as_str())
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Admin delete — redeemed codes are kept (audit chain to wallet ledger).
pub async fn delete_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<bool> {
    use crate::db::driver::DbDriver;
    let ph = crate::db::Driver::ph;
    let sql = format!(
        "DELETE FROM redemption_codes WHERE id = {} AND status <> {}",
        ph(1),
        ph(2)
    );
    let result: crate::db::pool::DbQueryResult = sqlx::query(crate::db::safe_sql(&sql))
        .bind(id)
        .bind(RedemptionCodeStatus::Redeemed.as_str())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Admin invalidate: `pending → disabled` CAS.
pub async fn tx_disable(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
) -> AppResult<bool> {
    use crate::db::driver::DbDriver;
    let ph = crate::db::Driver::ph;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE redemption_codes SET status = {}, updated_at = {} \
         WHERE id = {} AND status = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4)
    );
    let result: crate::db::pool::DbQueryResult = sqlx::query(crate::db::safe_sql(&sql))
        .bind(RedemptionCodeStatus::Disabled.as_str())
        .bind(now)
        .bind(id)
        .bind(RedemptionCodeStatus::Pending.as_str())
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() > 0)
}
