use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::user_address::{CreateUserAddressRequest, UpdateUserAddressRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::user_address::UserAddress;
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait UserAddressService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        req: CreateUserAddressRequest,
    ) -> AppResult<UserAddress>;

    async fn update(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        id: SnowflakeId,
        req: UpdateUserAddressRequest,
    ) -> AppResult<UserAddress>;

    async fn delete(&self, auth: &AuthUser, user_id: SnowflakeId, id: SnowflakeId)
    -> AppResult<()>;

    async fn list(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<Vec<UserAddress>>;

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<UserAddress>;
}

pub struct UserAddressServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl UserAddressServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserAddressService for UserAddressServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        req: CreateUserAddressRequest,
    ) -> AppResult<UserAddress> {
        crate::models::user_address::insert(
            &self.pool,
            &crate::commands::CreateUserAddressCmd {
                user_id,
                label: req.label.unwrap_or_default(),
                recipient_name: req.recipient_name,
                phone: req.phone,
                country: req.country.unwrap_or_else(|| "CN".to_string()),
                province: req.province.unwrap_or_default(),
                city: req.city.unwrap_or_default(),
                district: req.district.unwrap_or_default(),
                address_line1: req.address_line1,
                address_line2: req.address_line2,
                postal_code: req.postal_code,
                is_default: req.is_default.unwrap_or(false),
                address_type: req.address_type.unwrap_or_else(|| "shipping".to_string()),
            },
            auth.tenant_id(),
        )
        .await
    }

    async fn update(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        id: SnowflakeId,
        req: UpdateUserAddressRequest,
    ) -> AppResult<UserAddress> {
        let existing = crate::models::user_address::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("user_address"))?;

        if existing.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        let updated = crate::models::user_address::update(
            &self.pool,
            &crate::commands::UpdateUserAddressCmd {
                id: existing.id,
                user_id,
                label: req.label.unwrap_or(existing.label),
                recipient_name: req.recipient_name.unwrap_or(existing.recipient_name),
                phone: req.phone.unwrap_or(existing.phone),
                country: req.country.unwrap_or(existing.country),
                province: req.province.unwrap_or(existing.province),
                city: req.city.unwrap_or(existing.city),
                district: req.district.unwrap_or(existing.district),
                address_line1: req.address_line1.unwrap_or(existing.address_line1),
                address_line2: req.address_line2.or(existing.address_line2),
                postal_code: req.postal_code.or(existing.postal_code),
                is_default: req.is_default.unwrap_or(existing.is_default),
                address_type: req.address_type.unwrap_or(existing.address_type),
            },
            auth.tenant_id(),
        )
        .await?;

        if !updated {
            return Err(AppError::not_found("user_address"));
        }

        crate::models::user_address::find_by_id(&self.pool, existing.id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("user_address"))
    }

    async fn delete(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        id: SnowflakeId,
    ) -> AppResult<()> {
        let existing = crate::models::user_address::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("user_address"))?;

        if existing.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        crate::models::user_address::delete_by_id(
            &self.pool,
            existing.id,
            user_id,
            auth.tenant_id(),
        )
        .await?;

        Ok(())
    }

    async fn list(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<Vec<UserAddress>> {
        crate::models::user_address::find_by_user_id(&self.pool, user_id, auth.tenant_id()).await
    }

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<UserAddress> {
        crate::models::user_address::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("user_address"))
    }
}
