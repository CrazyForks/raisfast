//! sqlx-based `OptionsRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::options::OptionRow;
use raisfast_derive::repository;

#[repository(model = "options", struct_name = SqlxOptionsRepository)]
pub trait OptionsRepository: Send + Sync {
    async fn find_autoload(&self) -> AppResult<Vec<OptionRow>>;

    async fn find_by_key(&self, key: &str, tenant_id: Option<i64>) -> AppResult<Option<OptionRow>>;

    async fn find_all(&self, tenant_id: Option<i64>) -> AppResult<Vec<OptionRow>>;

    async fn upsert_value(&self, key: &str, value: &str, tenant_id: Option<i64>) -> AppResult<()>;

    async fn delete_by_key(&self, key: &str, tenant_id: Option<i64>) -> AppResult<()>;
}
