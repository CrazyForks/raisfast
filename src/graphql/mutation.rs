//! GraphQL Mutation resolvers

use super::types::{ContentItem, DeleteResult, JsonScalar, MutationRoot};
use crate::content_type::handler::cms_detail_cache_key;
use crate::content_type::repository::{ContentRepository, SaveContext};
use crate::content_type::schema::check_api_access;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use async_graphql::*;
use std::sync::Arc;

fn get_state(ctx: &Context<'_>) -> Result<Arc<crate::AppState>> {
    ctx.data::<Arc<crate::AppState>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("missing state"))
}

fn require_auth(ctx: &Context<'_>) -> Result<AuthUser> {
    let auth = ctx
        .data::<AuthUser>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("missing auth"))?;
    if !auth.is_authenticated() {
        return Err(async_graphql::Error::new("authentication required"));
    }
    Ok(auth)
}

#[Object]
impl MutationRoot {
    /// Create content
    async fn create_content(
        &self,
        ctx: &Context<'_>,
        r#type: String,
        data: JsonScalar,
    ) -> Result<ContentItem> {
        let state = get_state(ctx)?;
        let auth = require_auth(ctx)?;

        let ct = state.content_type_registry.get(&r#type).ok_or_else(|| {
            async_graphql::Error::new(format!("content type '{}' not found", r#type))
        })?;

        check_api_access(ct.api.create.access, &auth).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let obj = match data.0 {
            serde_json::Value::Object(map) => map,
            _ => return Err(async_graphql::Error::new("data must be a JSON object")),
        };

        let save_ctx = SaveContext {
            user_id: auth.user_id().map(|id| id.to_string()),
            user_int_id: auth.user_id(),
            user_role: Some(auth.role().to_string()),
            tenant_id: auth.tenant_id().map(|s| s.to_string()),
        };

        let repo = ContentRepository::new(state.pool.clone());
        let result = repo
            .create(&ct, serde_json::Value::Object(obj), None, &save_ctx)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        Ok(json_to_content_item(result))
    }

    /// Update content
    async fn update_content(
        &self,
        ctx: &Context<'_>,
        r#type: String,
        id: ID,
        data: JsonScalar,
    ) -> Result<ContentItem> {
        let state = get_state(ctx)?;
        let auth = require_auth(ctx)?;

        let ct = state.content_type_registry.get(&r#type).ok_or_else(|| {
            async_graphql::Error::new(format!("content type '{}' not found", r#type))
        })?;

        check_api_access(ct.api.update.access, &auth).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let repo = ContentRepository::new(state.pool.clone());
        let int_id: i64 = id
            .as_str()
            .parse()
            .map_err(|e| async_graphql::Error::new(format!("invalid id: {e}")))?;

        let existing = repo
            .find_by_id(&ct, SnowflakeId(int_id), None, true)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        if let Some(record) = &existing
            && let Some(rules) = ct.cached_rules.as_ref()
            && let Some(rule) = rules.update.filter.as_ref()
        {
            let rule_ctx = crate::content_type::rule_engine::RuleContext::from_auth(&auth);
            if !rule.evaluate(record, &rule_ctx, &state.config.rule_engine) {
                return Err(async_graphql::Error::new("forbidden"));
            }
        }

        let obj = match data.0 {
            serde_json::Value::Object(map) => map,
            _ => return Err(async_graphql::Error::new("data must be a JSON object")),
        };

        let save_ctx = SaveContext {
            user_id: auth.user_id().map(|id| id.to_string()),
            user_int_id: auth.user_id(),
            user_role: Some(auth.role().to_string()),
            tenant_id: auth.tenant_id().map(|s| s.to_string()),
        };

        let result = repo
            .update(
                &ct,
                SnowflakeId(int_id),
                serde_json::Value::Object(obj),
                None,
                &save_ctx,
            )
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        let cache_key = cms_detail_cache_key(&ct, SnowflakeId(int_id));
        state.cms_cache.remove(&cache_key);

        Ok(json_to_content_item(result))
    }

    /// Delete content
    async fn delete_content(
        &self,
        ctx: &Context<'_>,
        r#type: String,
        id: ID,
    ) -> Result<DeleteResult> {
        let state = get_state(ctx)?;
        let auth = require_auth(ctx)?;

        let ct = state.content_type_registry.get(&r#type).ok_or_else(|| {
            async_graphql::Error::new(format!("content type '{}' not found", r#type))
        })?;

        check_api_access(ct.api.delete.access, &auth).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let repo = ContentRepository::new(state.pool.clone());
        let int_id: i64 = id
            .as_str()
            .parse()
            .map_err(|e| async_graphql::Error::new(format!("invalid id: {e}")))?;

        let existing = repo
            .find_by_id(&ct, SnowflakeId(int_id), None, true)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        if let Some(record) = &existing
            && let Some(rules) = ct.cached_rules.as_ref()
            && let Some(rule) = rules.delete.filter.as_ref()
        {
            let rule_ctx = crate::content_type::rule_engine::RuleContext::from_auth(&auth);
            if !rule.evaluate(record, &rule_ctx, &state.config.rule_engine) {
                return Err(async_graphql::Error::new("forbidden"));
            }
        }

        repo.delete(
            &ct,
            SnowflakeId(int_id),
            None,
            &state.protocol_registry,
            &state.content_type_registry,
        )
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        let cache_key = cms_detail_cache_key(&ct, SnowflakeId(int_id));
        state.cms_cache.remove(&cache_key);

        Ok(DeleteResult {
            success: true,
            id: id.to_string(),
        })
    }
}

fn json_to_content_item(val: serde_json::Value) -> ContentItem {
    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    ContentItem {
        id,
        data: JsonScalar(val),
    }
}
