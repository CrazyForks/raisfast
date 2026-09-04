//! Agent definition model (`ai_agents`): provider/model/system_prompt,
//! tool allowlist and memory switch. Multi-tenant (tenant_id filter).

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// One agent definition.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AiAgent {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: Option<SnowflakeId>,
    pub name: String,
    pub system_prompt: String,
    pub provider: String,
    pub model: String,
    pub temperature: Option<f64>,
    pub max_iterations: i32,
    pub tools: serde_json::Value,
    pub memory_enabled: bool,
    pub params: Option<serde_json::Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create an agent and return it.
#[allow(clippy::too_many_arguments)]
pub async fn create_agent(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: Option<SnowflakeId>,
    name: &str,
    system_prompt: &str,
    provider: &str,
    model: &str,
    temperature: Option<f64>,
    tools: Vec<String>,
    memory_enabled: bool,
    params: Option<serde_json::Value>,
) -> AppResult<AiAgent> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    let tools = serde_json::to_value(tools).unwrap_or(serde_json::Value::Array(vec![]));
    raisfast_derive::crud_insert!(
        pool,
        "ai_agents",
        [
            "id" => id,
            "user_id" => user_id,
            "name" => name,
            "system_prompt" => system_prompt,
            "provider" => provider,
            "model" => model,
            "temperature" => temperature,
            "max_iterations" => 10i32,
            "tools" => tools,
            "memory_enabled" => memory_enabled,
            "params" => params,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_agent_by_id(pool, id, tenant_id).await
}

/// Find an agent by id (tenant-scoped).
pub async fn find_agent_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<AiAgent> {
    let result: AiAgent = raisfast_derive::crud_find_one!(
        pool,
        "ai_agents",
        AiAgent,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// List all agents of a tenant, ordered by name.
pub async fn list_agents(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<AiAgent>> {
    let result: Vec<AiAgent> = raisfast_derive::crud_list!(pool, "ai_agents", AiAgent, order_by: "name", tenant: tenant_id)?;
    Ok(result)
}

/// Update an agent's editable columns (full overwrite of provided values).
#[allow(clippy::too_many_arguments)]
pub async fn update_agent(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    system_prompt: &str,
    provider: &str,
    model: &str,
    temperature: Option<f64>,
    max_iterations: i32,
    tools: serde_json::Value,
    memory_enabled: bool,
    params: Option<serde_json::Value>,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "ai_agents",
        bind: [
            "system_prompt" => system_prompt,
            "provider" => provider,
            "model" => model,
            "temperature" => temperature,
            "max_iterations" => max_iterations,
            "tools" => tools,
            "memory_enabled" => memory_enabled,
            "params" => params,
            "updated_at" => &now
        ],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "ai_agent")
}

/// Delete an agent by id (tenant-scoped).
pub async fn delete_agent(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "ai_agents",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "ai_agent")
}
