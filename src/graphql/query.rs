//! GraphQL Query resolvers

use super::types::{ContentConnection, ContentItem, JsonScalar, QueryRoot};
use crate::content_type::handler::{ListParams, do_get, do_list};
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

fn get_auth(ctx: &Context<'_>) -> Result<AuthUser> {
    ctx.data::<AuthUser>()
        .cloned()
        .map_err(|_| async_graphql::Error::new("missing auth"))
}

#[Object]
impl QueryRoot {
    /// Paginated content list query
    #[allow(clippy::too_many_arguments)]
    async fn content(
        &self,
        ctx: &Context<'_>,
        r#type: String,
        page: Option<i32>,
        page_size: Option<i32>,
        sort: Option<String>,
        status: Option<String>,
        search: Option<String>,
        include: Option<String>,
        skip_total: Option<bool>,
    ) -> Result<ContentConnection> {
        let state = get_state(ctx)?;
        let auth = get_auth(ctx)?;

        let ct = state.content_type_registry.get(&r#type).ok_or_else(|| {
            async_graphql::Error::new(format!("content type '{}' not found", r#type))
        })?;

        check_api_access(ct.api.list.access, &auth).map_err(
            |e: crate::errors::app_error::AppError| async_graphql::Error::new(e.to_string()),
        )?;

        let params = ListParams {
            page: page.map(|p| p as i64),
            page_size: page_size.map(|s| s as i64),
            sort,
            status,
            search,
            include,
            skip_total,
            extra: std::collections::HashMap::new(),
        };

        let result = do_list(&state, &ct, params, &auth)
            .await
            .map_err(|e| e.to_string())?;

        connection_from_json(result)
    }

    /// Get a single content item by ID
    async fn content_by_id(
        &self,
        ctx: &Context<'_>,
        r#type: String,
        id: ID,
        #[graphql(desc = "Relation expansion (e.g. author,tags)")] include: Option<String>,
    ) -> Result<Option<ContentItem>> {
        let state = get_state(ctx)?;
        let auth = get_auth(ctx)?;

        let ct = state.content_type_registry.get(&r#type).ok_or_else(|| {
            async_graphql::Error::new(format!("content type '{}' not found", r#type))
        })?;

        let int_id = id
            .as_str()
            .parse::<i64>()
            .map_err(|_| async_graphql::Error::new("invalid id"))?;
        let result = match do_get(&state, &ct, SnowflakeId(int_id), &auth).await {
            Ok(val) => val,
            Err(crate::errors::app_error::AppError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(async_graphql::Error::new(e.to_string())),
        };

        let _ = include;
        Ok(Some(json_to_content_item(result)))
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

fn connection_from_json(val: serde_json::Value) -> Result<ContentConnection> {
    let items = val
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().cloned().map(json_to_content_item).collect())
        .unwrap_or_default();

    let total = val.get("total").and_then(|v| v.as_i64()).map(|t| t as i32);

    let page = val.get("page").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
    let page_size = val.get("page_size").and_then(|v| v.as_i64()).unwrap_or(20) as i32;

    Ok(ContentConnection {
        items,
        total,
        page,
        page_size,
    })
}
