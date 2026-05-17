//! SMS verification code model and database queries

use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::id;
use crate::utils::tz::Timestamp;

/// SMS verification code database row model
#[derive(Debug, FromRow)]
#[non_exhaustive]
pub struct SmsCode {
    pub id: i64,
    pub document_id: String,
    pub phone: String,
    pub code: String,
    pub purpose: String,
    pub expires_at: Timestamp,
    pub verified_at: Option<Timestamp>,
    pub attempts: i64,
    pub ip_address: Option<String>,
    pub created_at: Timestamp,
}

/// Generate a random numeric verification code of the specified length
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

/// Create a new SMS verification code record
///
/// Duplicate sending for the same phone number and purpose within 60 seconds is not allowed.
pub async fn create(
    pool: &crate::db::Pool,
    phone: &str,
    code: &str,
    purpose: &str,
    expires_in_secs: u64,
    ip_address: Option<&str>,
) -> AppResult<SmsCode> {
    let (document_id, now) = id::new_document_id_and_timestamp();
    let expires_at =
        crate::utils::tz::now_utc() + chrono::Duration::seconds(expires_in_secs as i64);

    crud_insert!(pool, "sms_codes", [
        "document_id" => &document_id,
        "phone" => phone,
        "code" => code,
        "purpose" => purpose,
        "expires_at" => expires_at,
        "ip_address" => ip_address,
        "created_at" => now
    ])?;

    crud_find!(pool, "sms_codes" => SmsCode, "document_id" => &document_id)?.ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("failed to fetch sms code"))
    })
}

/// Find a verification code by ID
pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<Option<SmsCode>> {
    Ok(crud_find!(pool, "sms_codes" => SmsCode, "id" => id)?)
}

/// Find the most recent unverified code for a phone number
pub async fn find_latest_unverified(
    pool: &crate::db::Pool,
    phone: &str,
    purpose: &str,
) -> AppResult<Option<SmsCode>> {
    check_schema!("sms_codes", "phone", "purpose", "verified_at", "created_at");
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

/// Check if rate-limited (whether there is a sending record for the same phone number and purpose within the last N seconds)
pub async fn is_rate_limited(
    pool: &crate::db::Pool,
    phone: &str,
    purpose: &str,
    within_secs: u64,
) -> AppResult<bool> {
    check_schema!("sms_codes", "phone", "purpose", "created_at");
    let cutoff = crate::utils::tz::now_utc() - chrono::Duration::seconds(within_secs as i64);
    let sql = format!(
        "SELECT COUNT(*) as cnt FROM sms_codes WHERE phone = {} AND purpose = {} AND created_at > {}",
        ph(1),
        ph(2),
        ph(3),
    );
    let row: (i64,) = sqlx::query_as(&sql)
        .bind(phone)
        .bind(purpose)
        .bind(cutoff)
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}

/// Verify code: on match, mark as verified; on mismatch, increment attempts
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

    if sms.expires_at < crate::utils::tz::now_utc() {
        return Ok(VerifyResult::Expired);
    }

    if sms.attempts >= 5 {
        return Ok(VerifyResult::MaxAttempts);
    }

    if sms.code != input_code {
        crud_update!(pool, "sms_codes",
            bind: [],
            raw: ["attempts" => "attempts + 1"],
            where: "id" => id
        )?;
        return Ok(VerifyResult::WrongCode);
    }

    let now = crate::utils::tz::now_utc();
    crud_update!(pool, "sms_codes",
        bind: ["verified_at" => now],
        where: "id" => id
    )?;

    Ok(VerifyResult::Verified)
}

/// Verification result
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Verified,
    WrongCode,
    Expired,
    AlreadyUsed,
    MaxAttempts,
}

/// Clean up expired verification code records
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    check_schema!("sms_codes", "expires_at");
    let now = crate::utils::tz::now_utc();
    let sql = format!("DELETE FROM sms_codes WHERE expires_at < {}", ph(1));
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn unique_phone() -> String {
        let id = crate::utils::id::new_document_id();
        let hash = id
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        format!("1380000{:04}", hash % 10000)
    }

    #[test]
    fn generate_code_length() {
        let code = super::generate_code(6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let phone = unique_phone();
        let code = super::generate_code(6);

        let sms = super::create(&pool, &phone, &code, "login", 300, Some("127.0.0.1"))
            .await
            .unwrap();

        let found = super::find_by_id(&pool, sms.id).await.unwrap();
        assert!(found.is_some());
        let row = found.unwrap();
        assert_eq!(row.phone, phone);
        assert_eq!(row.code, code);
        assert_eq!(row.purpose, "login");
    }

    #[tokio::test]
    async fn find_latest_unverified() {
        let pool = setup_pool().await;
        let phone = unique_phone();

        let _first = super::create(&pool, &phone, "111111", "login", 300, None)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let second = super::create(&pool, &phone, "222222", "login", 300, None)
            .await
            .unwrap();

        let latest = super::find_latest_unverified(&pool, &phone, "login")
            .await
            .unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().id, second.id);
    }

    #[tokio::test]
    async fn verify_code_correct() {
        let pool = setup_pool().await;
        let phone = unique_phone();
        let code = "654321";

        let sms = super::create(&pool, &phone, code, "login", 300, None)
            .await
            .unwrap();

        let result = super::verify_code(&pool, sms.id, code).await.unwrap();
        assert_eq!(result, super::VerifyResult::Verified);
    }

    #[tokio::test]
    async fn verify_code_wrong() {
        let pool = setup_pool().await;
        let phone = unique_phone();

        let sms = super::create(&pool, &phone, "123456", "login", 300, None)
            .await
            .unwrap();

        let result = super::verify_code(&pool, sms.id, "000000").await.unwrap();
        assert_eq!(result, super::VerifyResult::WrongCode);
    }

    #[tokio::test]
    async fn is_rate_limited() {
        let pool = setup_pool().await;
        let phone = unique_phone();

        super::create(&pool, &phone, "111111", "login", 300, None)
            .await
            .unwrap();

        let limited = super::is_rate_limited(&pool, &phone, "login", 60)
            .await
            .unwrap();
        assert!(limited);
    }

    #[tokio::test]
    async fn cleanup_expired() {
        let pool = setup_pool().await;
        let phone = unique_phone();

        super::create(&pool, &phone, "111111", "login", 0, None)
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let removed = super::cleanup_expired(&pool).await.unwrap();
        assert!(removed > 0);
    }
}
