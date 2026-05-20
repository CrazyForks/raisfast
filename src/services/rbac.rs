//! RBAC service layer — role/permission CRUD + permission checking
//!
//! All database operations are delegated to the [`RbacRepository`] trait implementation;
//! this layer only contains business logic (permission matching, condition validation, etc.).

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::CreatePermissionCmd;
use crate::errors::app_error::AppError;
use crate::models::rbac::{Permission, Role};
use crate::utils::tz::Timestamp;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PermissionEntry {
    pub action: String,
    pub subject: String,
    pub fields: Option<Vec<String>>,
    pub conditions: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetPermissionsRequest {
    pub permissions: Vec<PermissionEntry>,
}

/// Handler-facing Permission view (fields/conditions already deserialized)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionView {
    #[serde(serialize_with = "crate::types::snowflake_id::serialize_id_as_string")]
    pub id: SnowflakeId,
    pub role_id: SnowflakeId,
    pub action: String,
    pub subject: String,
    pub fields: Option<Vec<String>>,
    pub conditions: Option<HashMap<String, String>>,
    pub created_at: Timestamp,
}

fn perm_to_view(p: &Permission) -> PermissionView {
    PermissionView {
        id: p.id,
        role_id: p.role_id,
        action: p.action.clone(),
        subject: p.subject.clone(),
        fields: p.fields.as_ref().and_then(|f| serde_json::from_str(f).ok()),
        conditions: p
            .conditions
            .as_ref()
            .and_then(|c| serde_json::from_str(c).ok()),
        created_at: p.created_at,
    }
}

/// RBAC service
pub struct RbacService {
    pool: Arc<crate::db::Pool>,
}

impl RbacService {
    /// Create a `RbacService` instance
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }

    /// List all roles
    pub async fn list_roles(&self) -> Result<Vec<Role>, AppError> {
        crate::models::rbac::list_roles(&self.pool).await
    }

    /// Get a role by ID
    pub async fn get_role(&self, id: SnowflakeId) -> Result<Option<Role>, AppError> {
        crate::models::rbac::find_role_by_id(&self.pool, id).await
    }

    pub async fn create_role(&self, req: &CreateRoleRequest) -> Result<Role, AppError> {
        crate::models::rbac::create_role(&self.pool, &req.name, req.description.as_deref()).await
    }

    pub async fn update_role(
        &self,
        id: SnowflakeId,
        req: &UpdateRoleRequest,
    ) -> Result<Role, AppError> {
        crate::models::rbac::update_role(
            &self.pool,
            id,
            req.name.as_deref(),
            req.description.as_deref(),
        )
        .await
    }

    pub async fn delete_role(&self, id: SnowflakeId) -> Result<(), AppError> {
        let role = crate::models::rbac::find_role_by_id(&self.pool, id)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{id}")))?;
        if role.is_system {
            return Err(AppError::BadRequest("cannot delete system role".into()));
        }
        crate::models::rbac::delete_role(&self.pool, id).await
    }

    pub async fn get_permissions(&self, role_id: &str) -> Result<Vec<PermissionView>, AppError> {
        let rid = crate::types::snowflake_id::parse_id(role_id)?;
        let role = crate::models::rbac::find_role_by_id(&self.pool, rid)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{role_id}")))?;
        let perms = crate::models::rbac::find_permissions_by_role_id(&self.pool, role.id).await?;
        Ok(perms.iter().map(perm_to_view).collect())
    }

    pub async fn set_permissions(
        &self,
        role_id: &str,
        entries: &[PermissionEntry],
    ) -> Result<Vec<PermissionView>, AppError> {
        let rid = crate::types::snowflake_id::parse_id(role_id)?;
        let role = crate::models::rbac::find_role_by_id(&self.pool, rid)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{role_id}")))?;
        crate::models::rbac::delete_permissions_by_role_id(&self.pool, role.id).await?;

        for entry in entries {
            let fields_json = entry
                .fields
                .as_ref()
                .map(|f| serde_json::to_string(f).unwrap_or_default());
            let conditions_json = entry
                .conditions
                .as_ref()
                .map(|c| serde_json::to_string(c).unwrap_or_default());

            crate::models::rbac::insert_permission(
                &self.pool,
                &CreatePermissionCmd {
                    role_id: role.id,
                    action: entry.action.clone(),
                    subject: entry.subject.clone(),
                    fields: fields_json,
                    conditions: conditions_json,
                },
            )
            .await?;
        }
        self.get_permissions(role_id).await
    }

    /// Check permission
    pub async fn check_permission(
        &self,
        role_id: &str,
        action: &str,
        subject: &str,
        user_context: Option<&HashMap<String, Value>>,
    ) -> Result<(), AppError> {
        let permissions = self.get_permissions(role_id).await?;
        for perm in &permissions {
            if matches_action(&perm.action, action) && matches_subject(&perm.subject, subject) {
                if let Some(ref conditions) = perm.conditions {
                    if let Some(ctx) = user_context {
                        if !check_conditions(conditions, ctx) {
                            continue;
                        }
                    } else if !conditions.is_empty() {
                        continue;
                    }
                }
                return Ok(());
            }
        }
        Err(AppError::Forbidden)
    }

    /// Get role ID by role name
    pub async fn get_role_id_by_name(&self, name: &str) -> Result<Option<i64>, AppError> {
        crate::models::rbac::find_role_id_by_name(&self.pool, name).await
    }
}

/// Permission action matching (supports `*` wildcard and `::` namespace)
#[must_use]
pub fn matches_action(pattern: &str, action: &str) -> bool {
    if pattern == "*" || pattern == action {
        return true;
    }
    let (p_ns, p_op) = rsplit_dot(pattern);
    let (a_ns, a_op) = rsplit_dot(action);

    if !ns_matches(p_ns, a_ns) {
        return false;
    }

    p_op == "*" || p_op == a_op
}

fn rsplit_dot(s: &str) -> (&str, &str) {
    match s.rfind('.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

fn ns_matches(pattern: &str, action: &str) -> bool {
    if pattern == action {
        return true;
    }
    let pp: Vec<&str> = pattern.split("::").collect();
    let ap: Vec<&str> = action.split("::").collect();
    if pp.len() != ap.len() {
        return false;
    }
    pp.iter().zip(ap.iter()).all(|(p, a)| *p == "*" || *p == *a)
}

/// Permission subject matching (supports `*` wildcard and `::` namespace)
#[must_use]
pub fn matches_subject(pattern: &str, subject: &str) -> bool {
    if pattern == "*" || pattern == subject {
        return true;
    }
    let pp: Vec<&str> = pattern.split("::").collect();
    let sp: Vec<&str> = subject.split("::").collect();
    if pp.len() != sp.len() {
        return false;
    }
    pp.iter().zip(sp.iter()).all(|(p, s)| *p == "*" || *p == *s)
}

fn check_conditions(
    conditions: &HashMap<String, String>,
    context: &HashMap<String, Value>,
) -> bool {
    for (key, expected) in conditions {
        let resolved = resolve_template(expected, context);
        match context.get(key) {
            Some(val) => {
                let val_str = match val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if val_str != resolved {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

fn resolve_template(template: &str, context: &HashMap<String, Value>) -> String {
    if let Some(var) = template.strip_prefix("$user.") {
        context
            .get(var)
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    } else {
        template.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_action_wildcard() {
        assert!(matches_action("*", "content-type::post.create"));
        assert!(matches_action(
            "content-type::*.*",
            "content-type::post.create"
        ));
        assert!(matches_action(
            "content-type::post.*",
            "content-type::post.create"
        ));
        assert!(matches_action(
            "content-type::post.create",
            "content-type::post.create"
        ));
        assert!(!matches_action(
            "content-type::post.delete",
            "content-type::post.create"
        ));
        assert!(matches_action("*", "anything"));
        assert!(matches_action(
            "content-type::*.*",
            "content-type::comment.delete"
        ));
    }

    #[test]
    fn matches_subject_wildcard() {
        assert!(matches_subject("*", "content-type::post"));
        assert!(matches_subject("content-type::*", "content-type::post"));
        assert!(matches_subject("content-type::post", "content-type::post"));
        assert!(!matches_subject(
            "content-type::post",
            "content-type::comment"
        ));
    }

    #[test]
    fn check_conditions_basic() {
        let mut conditions = HashMap::new();
        conditions.insert("author_id".into(), "$user.id".into());
        let mut context = HashMap::new();
        context.insert("author_id".into(), Value::String("u-123".into()));
        context.insert("id".into(), Value::String("u-123".into()));
        assert!(check_conditions(&conditions, &context));
        context.insert("id".into(), Value::String("u-456".into()));
        assert!(!check_conditions(&conditions, &context));
    }

    #[test]
    fn resolve_template_user_var() {
        let mut ctx = HashMap::new();
        ctx.insert("id".into(), Value::String("user-1".into()));
        assert_eq!(resolve_template("$user.id", &ctx), "user-1");
        assert_eq!(resolve_template("literal_value", &ctx), "literal_value");
    }

    #[test]
    fn rsplit_dot_with_dot() {
        assert_eq!(rsplit_dot("a.b.c"), ("a.b", "c"));
        assert_eq!(rsplit_dot("single"), ("single", ""));
    }

    #[test]
    fn rsplit_dot_no_dot() {
        assert_eq!(rsplit_dot("noun"), ("noun", ""));
    }

    #[test]
    fn ns_matches_exact() {
        assert!(ns_matches("content-type", "content-type"));
        assert!(ns_matches("a::b", "a::b"));
    }

    #[test]
    fn ns_matches_wildcard() {
        assert!(ns_matches("*", "content-type"));
        assert!(ns_matches("*::*", "content-type::post"));
        assert!(!ns_matches("a::*", "b::c"));
    }

    #[test]
    fn ns_matches_different_depth() {
        assert!(!ns_matches("a", "a::b"));
        assert!(!ns_matches("a::b::c", "a::b"));
    }

    #[test]
    fn matches_action_exact_match() {
        assert!(matches_action("post.create", "post.create"));
        assert!(!matches_action("post.create", "post.delete"));
    }

    #[test]
    fn matches_action_wildcard_op() {
        assert!(matches_action("post.*", "post.create"));
        assert!(matches_action("post.*", "post.delete"));
    }

    #[test]
    fn matches_subject_wildcard_parts() {
        assert!(matches_subject("content-type::*", "content-type::post"));
        assert!(matches_subject("*::*", "content-type::post"));
        assert!(!matches_subject("blog::*", "content-type::post"));
    }

    #[test]
    fn matches_subject_different_depth() {
        assert!(!matches_subject("a", "a::b"));
    }

    #[test]
    fn check_conditions_missing_key() {
        let mut conditions = HashMap::new();
        conditions.insert("missing_key".into(), "value".into());
        let context = HashMap::new();
        assert!(!check_conditions(&conditions, &context));
    }

    #[test]
    fn check_conditions_empty_passes() {
        let conditions = HashMap::new();
        let context = HashMap::new();
        assert!(check_conditions(&conditions, &context));
    }

    #[test]
    fn resolve_template_missing_var() {
        let ctx = HashMap::new();
        assert_eq!(resolve_template("$user.nonexistent", &ctx), "");
    }

    #[test]
    fn resolve_template_non_user_var() {
        let ctx = HashMap::new();
        assert_eq!(resolve_template("plain_text", &ctx), "plain_text");
    }
}
