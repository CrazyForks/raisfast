//! Redemption code service (wallet 域,design.md 定稿:兑换码不进 llm 底座)。
//! 生成 → 明文一次性展示(hash 落库,参照 sk- token);兑换 → 事务内
//! 「CAS 激活 + credit wallet」原子提交,激活时间/激活人落码行,钱包流水
//! `WalletTxType::Redemption` 以 `redemption-{id}` 幂等关联。

use crate::errors::app_error::{AppError, AppResult};
use crate::models::redemption_code::{self, RedemptionCode, RedemptionCodeStatus};
use crate::types::price::Price;
use crate::types::snowflake_id::SnowflakeId;

/// Fresh plaintext code: `rc-` + 16 url-safe chars in 4-char groups
/// (compact yet ~95 bits of entropy; groups aid manual transcription).
/// Shown exactly once — only the hash is stored.
fn generate_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut raw = [0u8; 16];
    let _ = getrandom::fill(&mut raw);
    let body: String = raw
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    let grouped = body
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-");
    format!("rc-{grouped}")
}

fn hash_code(plain: &str) -> String {
    crate::services::api_token::hash_token(plain)
}

pub struct GeneratedCode {
    pub id: SnowflakeId,
    pub code: String,
}

/// Admin: generate `count` codes (a batch shares currency/amount/owner).
#[allow(clippy::too_many_arguments)]
pub async fn generate(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    admin_id: SnowflakeId,
    user_id: Option<SnowflakeId>,
    currency: &str,
    amount: Price,
    count: i64,
    expires_at: Option<crate::utils::tz::Timestamp>,
) -> AppResult<Vec<GeneratedCode>> {
    if amount.0 <= 0 {
        return Err(AppError::BadRequest("amount_must_be_positive".into()));
    }
    let count = count.clamp(1, 100);
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let plain = generate_code();
        let row = redemption_code::insert(
            pool,
            tenant_id,
            &hash_code(&plain),
            user_id,
            currency,
            amount,
            Some(admin_id),
            expires_at,
            Some(crate::llm::crypto::encrypt(&plain)?),
        )
        .await?;
        out.push(GeneratedCode {
            id: row.id,
            code: plain,
        });
    }
    Ok(out)
}

/// Redeem brute-force guard: per-user failure counter in a fixed window.
/// Codes carry ~95 bits of entropy; this only blunts online guessing (an
/// attacker needs a valid account to even reach the lookup). Successful
/// redeems reset the counter; unrelated errors (DB down) don't count.
const REDEEM_FAIL_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);
const REDEEM_FAIL_MAX: u32 = 10;

fn redeem_fail_cache() -> &'static moka::sync::Cache<String, u32> {
    static CACHE: std::sync::OnceLock<moka::sync::Cache<String, u32>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(100_000)
            .time_to_idle(REDEEM_FAIL_WINDOW)
            .time_to_live(REDEEM_FAIL_WINDOW)
            .build()
    })
}

/// User: redeem. 事务内「CAS 激活(pending → redeemed,落激活人/激活时间)
/// → credit wallet」原子提交:CAS 抢不到(并发先至/已禁用)整笔回滚,
/// credit 失败同样回滚激活——两步不会脱节。
pub async fn redeem(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: SnowflakeId,
    code_plain: &str,
) -> AppResult<RedemptionCode> {
    let fail_key = format!("redeem:{user_id}");
    if redeem_fail_cache().get(&fail_key).unwrap_or(0) >= REDEEM_FAIL_MAX {
        return Err(AppError::TooManyRequests(
            "too many failed redemption attempts; try again later".into(),
        ));
    }

    let outcome = redeem_inner(pool, tenant_id, user_id, code_plain).await;
    match &outcome {
        // Guess-relevant failures count toward the lockout; success resets.
        Err(AppError::NotFound(_))
        | Err(AppError::BadRequest(_))
        | Err(AppError::ForbiddenOwnership) => {
            let fails = redeem_fail_cache().get(&fail_key).unwrap_or(0) + 1;
            redeem_fail_cache().insert(fail_key, fails);
        }
        Ok(_) => {
            redeem_fail_cache().invalidate(&fail_key);
        }
        _ => {}
    }
    outcome
}

async fn redeem_inner(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: SnowflakeId,
    code_plain: &str,
) -> AppResult<RedemptionCode> {
    let code_hash = hash_code(code_plain.trim());
    let code = redemption_code::find_by_hash(pool, tenant_id, &code_hash)
        .await?
        .ok_or_else(|| AppError::NotFound("redemption_code".to_owned()))?;
    if code.status != RedemptionCodeStatus::Pending {
        return Err(AppError::BadRequest(
            "redemption_code_not_redeemable".into(),
        ));
    }
    // 定向码:仅属主可兑;公共码(user_id NULL):首兑绑定。
    if let Some(owner) = code.user_id
        && owner != user_id
    {
        return Err(AppError::ForbiddenOwnership);
    }
    let tx_no = format!("redemption-{}", code.id.0);

    crate::in_transaction!(pool, tx, {
        let activated = redemption_code::tx_activate(
            &mut tx,
            tenant_id,
            code.id,
            user_id,
            &tx_no,
            Some(user_id),
        )
        .await?;
        if !activated {
            return Err(AppError::Conflict("redemption_code_not_redeemable".into()));
        }
        crate::services::wallet::tx_credit_by_user(
            &mut tx,
            tenant_id,
            user_id,
            &code.currency,
            code.amount,
            crate::models::wallet_transaction::WalletTxType::Redemption,
            &tx_no,
            Some(format!("redemption_code:{}", code.id.0)),
        )
        .await?;
        Ok(())
    })?;

    redemption_code::find_by_hash(pool, tenant_id, &code_hash)
        .await?
        .ok_or_else(|| AppError::not_found("redemption_code"))
}

/// Admin: 直接激活——定向码到账属主;公共码 `bind_user` 指定到账人并绑定。
/// 钱包入账 + 记录激活时间;已激活/已禁用/已过期同样拒绝。
#[allow(clippy::too_many_arguments)]
pub async fn activate_for_owner(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    bind_user: Option<SnowflakeId>,
) -> AppResult<RedemptionCode> {
    let code = redemption_code::find_by_id(pool, tenant_id, id).await?;
    if code.status != RedemptionCodeStatus::Pending {
        return Err(AppError::BadRequest(
            "redemption_code_not_redeemable".into(),
        ));
    }
    // 定向码 → 属主;公共码 → 管理员指定的人。
    let owner = code
        .user_id
        .or(bind_user)
        .ok_or_else(|| AppError::BadRequest("redemption_code_has_no_owner".into()))?;
    if let Some(exp) = code.expires_at
        && exp <= crate::utils::tz::now_utc()
    {
        return Err(AppError::BadRequest("redemption_code_expired".into()));
    }
    let tx_no = format!("redemption-{}", code.id.0);
    crate::in_transaction!(pool, tx, {
        let activated =
            redemption_code::tx_activate(&mut tx, tenant_id, code.id, owner, &tx_no, Some(owner))
                .await?;
        if !activated {
            return Err(AppError::Conflict("redemption_code_not_redeemable".into()));
        }
        crate::services::wallet::tx_credit_by_user(
            &mut tx,
            tenant_id,
            owner,
            &code.currency,
            code.amount,
            crate::models::wallet_transaction::WalletTxType::Redemption,
            &tx_no,
            Some(format!("redemption_code:{}", code.id.0)),
        )
        .await?;
        Ok(())
    })?;
    redemption_code::find_by_id(pool, tenant_id, id).await
}

/// Admin: list codes (paged + owner/status filters).
pub async fn list(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    filters: &redemption_code::RedemptionCodeFilters,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<RedemptionCode>, i64)> {
    redemption_code::query_paged(pool, filters, page, page_size, tenant_id).await
}

/// Admin: invalidate a pending code. Returns false when already redeemed.
pub async fn disable(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<bool> {
    crate::in_transaction!(pool, tx, { redemption_code::tx_disable(&mut tx, id).await })
}

/// Admin: re-enable a disabled code (back to redeemable). Returns false when
/// the code is not disabled.
pub async fn enable(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<bool> {
    crate::in_transaction!(pool, tx, { redemption_code::tx_enable(&mut tx, id).await })
}

/// Admin: delete a code. Redeemed codes are kept (audit chain to wallet
/// ledger). Returns false when the code was redeemed and cannot be deleted.
pub async fn delete(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<bool> {
    redemption_code::delete_by_id(pool, id).await
}
