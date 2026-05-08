//! RBAC 服务层 — 角色/权限 CRUD + 权限检查
//!
//! 所有数据库操作委托给 [`RbacRepository`] trait 实现，
//! 本层仅包含业务逻辑（权限匹配、条件校验等）。

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::app_error::AppError;
use crate::models::rbac::{Permission, Role};
use crate::repositories::RbacRepository;

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetPermissionsRequest {
    pub permissions: Vec<PermissionEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionEntry {
    pub action: String,
    pub subject: String,
    pub fields: Option<Vec<String>>,
    pub conditions: Option<HashMap<String, String>>,
}

/// 面向 handler 的 Permission 视图（fields/conditions 已反序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionView {
    pub id: i64,
    pub document_id: String,
    pub role_id: i64,
    pub action: String,
    pub subject: String,
    pub fields: Option<Vec<String>>,
    pub conditions: Option<HashMap<String, String>>,
    pub created_at: String,
}

fn perm_to_view(p: &Permission) -> PermissionView {
    PermissionView {
        id: p.id,
        document_id: p.document_id.clone(),
        role_id: p.role_id,
        action: p.action.clone(),
        subject: p.subject.clone(),
        fields: p.fields.as_ref().and_then(|f| serde_json::from_str(f).ok()),
        conditions: p
            .conditions
            .as_ref()
            .and_then(|c| serde_json::from_str(c).ok()),
        created_at: p.created_at.clone(),
    }
}

/// RBAC 服务
pub struct RbacService {
    repo: Arc<dyn RbacRepository>,
}

impl RbacService {
    /// 创建 `RbacService` 实例
    pub fn new(repo: Arc<dyn RbacRepository>) -> Self {
        Self { repo }
    }

    /// 列出所有角色
    pub async fn list_roles(&self) -> Result<Vec<Role>, AppError> {
        self.repo.list_roles().await
    }

    /// 根据 ID 获取角色
    pub async fn get_role(&self, id: &str) -> Result<Option<Role>, AppError> {
        self.repo.find_role_by_id(id).await
    }

    /// 创建角色
    pub async fn create_role(&self, req: &CreateRoleRequest) -> Result<Role, AppError> {
        let (id, now) = crate::utils::id::new_id_and_timestamp();
        self.repo
            .create_role(&id, &req.name, req.description.as_deref(), &now)
            .await
    }

    /// 更新角色
    pub async fn update_role(&self, id: &str, req: &UpdateRoleRequest) -> Result<Role, AppError> {
        let now = crate::utils::tz::now_str();
        self.repo
            .update_role(id, req.name.as_deref(), req.description.as_deref(), &now)
            .await
    }

    /// 删除角色（系统角色不可删除）
    pub async fn delete_role(&self, id: &str) -> Result<(), AppError> {
        let role = self
            .repo
            .find_role_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{id}")))?;
        if role.is_system {
            return Err(AppError::BadRequest("cannot delete system role".into()));
        }
        self.repo.delete_role(id).await
    }

    /// 获取角色的所有权限（fields/conditions 从 JSON 反序列化）
    pub async fn get_permissions(&self, role_id: &str) -> Result<Vec<PermissionView>, AppError> {
        let role = self
            .repo
            .find_role_by_id(role_id)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{role_id}")))?;
        let perms = self.repo.find_permissions_by_role_id(role.id).await?;
        Ok(perms.iter().map(perm_to_view).collect())
    }

    pub async fn set_permissions(
        &self,
        role_id: &str,
        entries: &[PermissionEntry],
    ) -> Result<Vec<PermissionView>, AppError> {
        let role = self
            .repo
            .find_role_by_id(role_id)
            .await?
            .ok_or_else(|| AppError::not_found(&format!("role/{role_id}")))?;
        self.repo.delete_permissions_by_role_id(role.id).await?;

        for entry in entries {
            let (doc_id, now) = crate::utils::id::new_id_and_timestamp();
            let fields_json = entry
                .fields
                .as_ref()
                .map(|f| serde_json::to_string(f).unwrap_or_default());
            let conditions_json = entry
                .conditions
                .as_ref()
                .map(|c| serde_json::to_string(c).unwrap_or_default());

            self.repo
                .insert_permission(
                    &doc_id,
                    role.id,
                    &entry.action,
                    &entry.subject,
                    fields_json.as_deref(),
                    conditions_json.as_deref(),
                    &now,
                )
                .await?;
        }
        self.get_permissions(role_id).await
    }

    /// 检查权限
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

    /// 根据角色名获取角色 ID
    pub async fn get_role_id_by_name(&self, name: &str) -> Result<Option<i64>, AppError> {
        self.repo.find_role_id_by_name(name).await
    }
}

/// 权限 action 匹配（支持 `*` 通配符和 `::` 命名空间）
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

/// 权限 subject 匹配（支持 `*` 通配符和 `::` 命名空间）
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
}
