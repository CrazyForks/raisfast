//! GraphQL Mutation 解析器

use super::types::{ContentItem, DeleteResult, JsonScalar, MutationRoot};
use crate::content_type::handler::cms_detail_cache_key;
use crate::content_type::repository::{ContentRepository, SaveContext};
use crate::content_type::schema::{AutoFillSource, check_api_access};
use crate::middleware::auth::AuthIdentity;
use async_graphql::*;
use std::sync::Arc;

fn get_state(ctx: &Context<'_>) -> Result<Arc<crate::AppState>> {
    ctx.data::<Arc<crate::AppState>>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("missing state"))
}

fn require_auth(ctx: &Context<'_>) -> Result<AuthIdentity> {
    ctx.data::<Option<AuthIdentity>>()
        .cloned()
        .ok()
        .flatten()
        .ok_or_else(|| async_graphql::Error::new("authentication required"))
}

#[Object]
impl MutationRoot {
    /// 创建内容
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

        check_api_access(ct.api.create.access, Some(&auth)).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let mut obj = match data.0 {
            serde_json::Value::Object(map) => map,
            _ => return Err(async_graphql::Error::new("data must be a JSON object")),
        };

        for field in &ct.fields {
            if let Some(auto) = &field.auto_fill {
                match auto {
                    AutoFillSource::UserId => {
                        obj.insert(
                            field.name.clone(),
                            serde_json::Value::String(auth.user_id.clone()),
                        );
                    }
                    AutoFillSource::UserRole => {
                        obj.insert(
                            field.name.clone(),
                            serde_json::Value::String(auth.role.clone()),
                        );
                    }
                    AutoFillSource::CurrentTenantId => {
                        obj.insert(
                            field.name.clone(),
                            serde_json::Value::String(auth.tenant_id.clone()),
                        );
                    }
                    AutoFillSource::CurrentTimestamp => {
                        obj.insert(
                            field.name.clone(),
                            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
                        );
                    }
                }
            }
        }

        let save_ctx = SaveContext {
            user_id: Some(auth.user_id.clone()),
            user_role: Some(auth.role.clone()),
            tenant_id: Some(auth.tenant_id.clone()),
        };

        let repo = ContentRepository::new(state.pool.clone());
        let result = repo
            .create(&ct, serde_json::Value::Object(obj), None, &save_ctx)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        Ok(json_to_content_item(result))
    }

    /// 更新内容
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

        check_api_access(ct.api.update.access, Some(&auth)).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let repo = ContentRepository::new(state.pool.clone());
        let existing = repo
            .find_by_id(&ct, id.as_str(), None, true)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        if let Some(record) = &existing
            && let Some(rules) = ct.cached_rules.as_ref()
            && let Some(rule) = rules.update.filter.as_ref()
        {
            let rule_ctx = crate::content_type::rule_engine::RuleContext::from_auth(Some(&auth));
            if !rule.evaluate(record, &rule_ctx, &state.config.rule_engine) {
                return Err(async_graphql::Error::new("forbidden"));
            }
        }

        let obj = match data.0 {
            serde_json::Value::Object(map) => map,
            _ => return Err(async_graphql::Error::new("data must be a JSON object")),
        };

        let save_ctx = SaveContext {
            user_id: Some(auth.user_id.clone()),
            user_role: Some(auth.role.clone()),
            tenant_id: Some(auth.tenant_id.clone()),
        };

        let result = repo
            .update(
                &ct,
                id.as_str(),
                serde_json::Value::Object(obj),
                None,
                &save_ctx,
            )
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        let cache_key = cms_detail_cache_key(&ct, id.as_str());
        state.cms_cache.remove(&cache_key);

        Ok(json_to_content_item(result))
    }

    /// 删除内容
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

        check_api_access(ct.api.delete.access, Some(&auth)).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let repo = ContentRepository::new(state.pool.clone());
        let existing = repo
            .find_by_id(&ct, id.as_str(), None, true)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        if let Some(record) = &existing
            && let Some(rules) = ct.cached_rules.as_ref()
            && let Some(rule) = rules.delete.filter.as_ref()
        {
            let rule_ctx = crate::content_type::rule_engine::RuleContext::from_auth(Some(&auth));
            if !rule.evaluate(record, &rule_ctx, &state.config.rule_engine) {
                return Err(async_graphql::Error::new("forbidden"));
            }
        }

        repo.delete(&ct, id.as_str(), None)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        let cache_key = cms_detail_cache_key(&ct, id.as_str());
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
