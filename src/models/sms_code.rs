//! 短信验证码模型与数据库查询

use chrono::Utc;
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::id;

/// 短信验证码数据库行模型
#[derive(Debug, FromRow)]
#[non_exhaustive]
pub struct SmsCode {
    pub id: i64,
    pub document_id: String,
    pub phone: String,
    pub code: String,
    pub purpose: String,
    pub expires_at: String,
    pub verified_at: Option<String>,
    pub attempts: i64,
    pub ip_address: Option<String>,
    pub created_at: String,
}

/// 生成指定位数的随机数字验证码
pub fn generate_code(length: u32) -> String {
    let digits: Vec<u8> = (0..length)
        .map(|_| {
            let mut byte = [0u8; 1];
            getrandom::getrandom(&mut byte).unwrap_or_default();
            byte[0] % 10
        })
        .collect();
    digits
        .iter()
        .map(|d| char::from_digit(*d as u32, 10).unwrap_or('0'))
        .collect()
}

/// 创建新的短信验证码记录
///
/// 同一手机号同一目的 60 秒内不允许重复发送。
pub async fn create(
    pool: &crate::db::Pool,
    phone: &str,
    code: &str,
    purpose: &str,
    expires_in_secs: u64,
    ip_address: Option<&str>,
) -> AppResult<SmsCode> {
    let (document_id, now) = id::new_document_id_and_timestamp();
    let expires_at = (Utc::now() + chrono::Duration::seconds(expires_in_secs as i64)).to_rfc3339();

    let sql = format!(
        "INSERT INTO sms_codes (document_id, phone, code, purpose, expires_at, ip_address, created_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(phone)
        .bind(code)
        .bind(purpose)
        .bind(&expires_at)
        .bind(ip_address)
        .bind(&now)
        .execute(pool)
        .await?;

    let sql2 = format!("SELECT * FROM sms_codes WHERE document_id = {}", ph(1));
    sqlx::query_as::<_, SmsCode>(&sql2)
        .bind(&document_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "failed to fetch sms code"
            ))
        })
}

/// 根据 ID 查找验证码
pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<Option<SmsCode>> {
    let sql = format!("SELECT * FROM sms_codes WHERE id = {}", ph(1));
    let row = sqlx::query_as::<_, SmsCode>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 查找手机号最近的未验证验证码
pub async fn find_latest_unverified(
    pool: &crate::db::Pool,
    phone: &str,
    purpose: &str,
) -> AppResult<Option<SmsCode>> {
    let sql = format!(
        "SELECT * FROM sms_codes WHERE phone = {} AND purpose = {} AND verified_at IS NULL ORDER BY created_at DESC LIMIT 1",
        ph(1),
        ph(2),
    );
    let row = sqlx::query_as::<_, SmsCode>(&sql)
        .bind(phone)
        .bind(purpose)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 检查是否在限流期内（同一手机号同一目的最近 N 秒内是否有发送记录）
pub async fn is_rate_limited(
    pool: &crate::db::Pool,
    phone: &str,
    purpose: &str,
    within_secs: u64,
) -> AppResult<bool> {
    let cutoff = (Utc::now() - chrono::Duration::seconds(within_secs as i64)).to_rfc3339();
    let sql = format!(
        "SELECT COUNT(*) as cnt FROM sms_codes WHERE phone = {} AND purpose = {} AND created_at > {}",
        ph(1),
        ph(2),
        ph(3),
    );
    let row: (i64,) = sqlx::query_as(&sql)
        .bind(phone)
        .bind(purpose)
        .bind(&cutoff)
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}

/// 验证码验证：匹配后标记已验证，错误时增加 attempts
pub async fn verify_code(
    pool: &crate::db::Pool,
    id: i64,
    input_code: &str,
) -> AppResult<VerifyResult> {
    let sms = find_by_id(pool, id)
        .await?
        .ok_or_else(|| crate::errors::app_error::AppError::BadRequest("invalid_code".into()))?;

    if sms.verified_at.is_some() {
        return Ok(VerifyResult::AlreadyUsed);
    }

    let expires_at = chrono::DateTime::parse_from_rfc3339(&sms.expires_at).map_err(|_| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("invalid expiry"))
    })?;

    if expires_at < Utc::now() {
        return Ok(VerifyResult::Expired);
    }

    if sms.attempts >= 5 {
        return Ok(VerifyResult::MaxAttempts);
    }

    if sms.code != input_code {
        let sql = format!(
            "UPDATE sms_codes SET attempts = attempts + 1 WHERE id = {}",
            ph(1),
        );
        sqlx::query(&sql).bind(id).execute(pool).await?;
        return Ok(VerifyResult::WrongCode);
    }

    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE sms_codes SET verified_at = {} WHERE id = {}",
        ph(1),
        ph(2),
    );
    sqlx::query(&sql).bind(&now).bind(id).execute(pool).await?;

    Ok(VerifyResult::Verified)
}

/// 验证结果
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Verified,
    WrongCode,
    Expired,
    AlreadyUsed,
    MaxAttempts,
}

/// 清理过期的验证码记录
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    let now = Utc::now().to_rfc3339();
    let sql = format!("DELETE FROM sms_codes WHERE expires_at < {}", ph(1));
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
}
