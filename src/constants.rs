//! Global constants
//!
//! Centralized management of hardcoded strings used repeatedly across the system.
//! Only includes values that are reused in multiple places; one-off values need not be constants.

// ─── Default Tenant ───

/// Default tenant ID
pub const DEFAULT_TENANT: &str = "default";

// ─── System Column Names ───

/// Ownership — creator
pub const COL_CREATED_BY: &str = "created_by";
/// Ownership — updater
pub const COL_UPDATED_BY: &str = "updated_by";
/// Timestamp — creation time
pub const COL_CREATED_AT: &str = "created_at";
/// Timestamp — update time
pub const COL_UPDATED_AT: &str = "updated_at";
/// Soft delete — deletion time
pub const COL_DELETED_AT: &str = "deleted_at";
/// Soft delete — deleter
pub const COL_DELETED_BY: &str = "deleted_by";
/// Version control — content revision number
pub const COL_VERSION: &str = "version";
/// Optimistic lock — lock version number
pub const COL_LOCK_VERSION: &str = "lock_version";
/// Sorting — sort key
pub const COL_SORT_KEY: &str = "created_at";
/// Status — status value
pub const COL_STATUS: &str = "status";
/// Expiration — expiration time
pub const COL_EXPIRES_AT: &str = "expires_at";
/// Nested — parent ID
pub const COL_PARENT_ID: &str = "parent_id";
/// Nested — depth level
pub const COL_DEPTH: &str = "depth";
/// Nested — sibling position
pub const COL_POSITION: &str = "position";
/// Metadata JSON column
pub const COL_META: &str = "__meta";
/// Tenant ID column
pub const COL_TENANT_ID: &str = "tenant_id";
/// Primary key column
pub const COL_ID: &str = "id";
// ─── Auth Header ───

pub const HEADER_AUTHORIZATION: &str = "authorization";
pub const HEADER_TENANT_ID: &str = "x-tenant-id";
pub const HEADER_API_TOKEN: &str = "x-api-token";
pub const AUTH_BEARER_PREFIX: &str = "Bearer ";

// ─── API Route Prefixes ───

/// API base path
pub const API_PREFIX: &str = "/api/v1";
/// Content Type public route prefix (full path)
pub const CMS_PREFIX: &str = "/api/v1/cms";
/// Content Type admin route prefix (full path)
pub const CMS_ADMIN_PREFIX: &str = "/api/v1/admin/cms";
/// Content Type public route segment (relative to API_PREFIX)
pub const CMS_ROUTE: &str = "/cms";
/// Content Type admin route segment (relative to API_PREFIX)
pub const CMS_ADMIN_ROUTE: &str = "/admin/cms";

/// Plugin host function global object name (JS/Lua)
pub const PLUGIN_HOST_GLOBAL: &str = "RaisFastHost";
