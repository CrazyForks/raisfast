use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::currencies::{self, Currency};

pub async fn list(pool: &Pool, tenant_id: Option<&str>) -> AppResult<Vec<Currency>> {
    currencies::find_all(pool, tenant_id).await
}

pub async fn get_by_code(pool: &Pool, code: &str, tenant_id: Option<&str>) -> AppResult<Currency> {
    currencies::find_by_code(pool, code, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("currency"))
}

pub async fn create(
    pool: &Pool,
    tenant_id: &str,
    code: &str,
    name: &str,
    decimals: i64,
) -> AppResult<Currency> {
    if !(0..=18).contains(&decimals) {
        return Err(AppError::BadRequest(
            "decimals must be between 0 and 18".into(),
        ));
    }
    currencies::create(pool, tenant_id, code, name, decimals).await
}

pub async fn update(
    pool: &Pool,
    code: &str,
    name: Option<&str>,
    is_active: Option<bool>,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    match currencies::update(pool, code, name, is_active, tenant_id).await? {
        Some(_) => Ok(true),
        None => Err(AppError::not_found("currency")),
    }
}

pub async fn delete(pool: &Pool, code: &str, tenant_id: Option<&str>) -> AppResult<()> {
    currencies::delete_by_code(pool, code, tenant_id)
        .await?
        .then_some(())
        .ok_or_else(|| AppError::not_found("currency"))
}
