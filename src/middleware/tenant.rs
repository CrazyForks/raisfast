//! 租户解析中间件
//!
//! 提供两种租户提取器：
//!
//! - [`TenantContext`]：仅从 `X-Tenant-ID` Header 解析（简单场景）
//! - [`ResolvedTenant`]：综合 JWT 和 Header，按业务规则解析（推荐）
//!
//! # `ResolvedTenant` 解析规则
//!
//! | 场景 | tenant_id | 说明 |
//! |---|---|---|
//! | 超管 + `X-Tenant-ID` | `Some(header)` | 超管切换到指定租户 |
//! | 超管 + 无 Header | `None` | 超管查看所有租户数据 |
//! | 普通用户 | `Some(claims.tenant_id)` | 忽略 Header，使用 JWT 中的租户 |
//! | 未认证 + `X-Tenant-ID` | `Some(header)` | 公开 API 指定租户 |
//! | 未认证 + 无 Header | `Some("default")` | 兜底 |
//!
//! `None` 表示不加 `WHERE tenant_id = ?` 过滤（超管查看所有数据）。

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::AppState;
use crate::errors::app_error::AppError;

/// 简单租户上下文提取器
///
/// 仅从 `X-Tenant-ID` 请求头解析租户 ID，未提供时回退到 `default`。
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: String,
}

/// 统一租户解析结果
///
/// - `tenant_id: Some(id)` → 只看该租户数据
/// - `tenant_id: None` → 超管看所有数据
/// - `is_super_admin` → 标记是否为超管
#[derive(Debug, Clone)]
pub struct ResolvedTenant {
    pub tenant_id: Option<String>,
    pub is_super_admin: bool,
}

impl ResolvedTenant {
    /// 返回 `Option<&str>`，直接传给 repo/service 层。
    pub fn as_str(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

fn extract_header_tenant(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("X-Tenant-ID")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(std::string::ToString::to_string)
}

fn extract_jwt_claims(parts: &Parts, state: &AppState) -> Option<crate::services::auth::Claims> {
    let mut parts_mut = parts.clone();
    crate::utils::auth::extract_claims(&mut parts_mut, state).ok()
}

impl FromRequestParts<AppState> for TenantContext {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let tenant_id = extract_header_tenant(parts)
            .unwrap_or_else(|| crate::db::tenant::DEFAULT_TENANT.to_string());

        async move { Ok(TenantContext { tenant_id }) }
    }
}

impl FromRequestParts<AppState> for ResolvedTenant {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let claims = extract_jwt_claims(parts, state);
        let header_tenant = extract_header_tenant(parts);

        let resolved = match (claims, header_tenant) {
            // 超管 + Header → 切换到指定租户
            (Some(c), Some(ht)) if c.role == "admin" => ResolvedTenant {
                tenant_id: Some(ht),
                is_super_admin: true,
            },
            // 超管 + 无 Header → 看所有
            (Some(c), None) if c.role == "admin" => ResolvedTenant {
                tenant_id: None,
                is_super_admin: true,
            },
            // 普通用户 → 使用 JWT 中的 tenant_id，忽略 Header
            (Some(c), _) => ResolvedTenant {
                tenant_id: Some(c.tenant_id),
                is_super_admin: false,
            },
            // 未认证 + Header → 公开 API 指定租户
            (None, Some(ht)) => ResolvedTenant {
                tenant_id: Some(ht),
                is_super_admin: false,
            },
            // 未认证 + 无 Header → 兜底 default
            (None, None) => ResolvedTenant {
                tenant_id: Some(crate::db::tenant::DEFAULT_TENANT.to_string()),
                is_super_admin: false,
            },
        };

        async move { Ok(resolved) }
    }
}
