//! 全局常量定义
//!
//! 集中管理系统中大量重复使用的硬编码字符串。
//! 只收录多处重复的，一次性的不需要常量化。

// ─── 默认租户 ───

/// 默认租户 ID
pub const DEFAULT_TENANT: &str = "default";

// ─── 系统列名 ───

/// 所有权 — 创建者
pub const COL_CREATED_BY: &str = "created_by";
/// 所有权 — 更新者
pub const COL_UPDATED_BY: &str = "updated_by";
/// 时间戳 — 创建时间
pub const COL_CREATED_AT: &str = "created_at";
/// 时间戳 — 更新时间
pub const COL_UPDATED_AT: &str = "updated_at";
/// 软删除 — 删除时间
pub const COL_DELETED_AT: &str = "deleted_at";
/// 软删除 — 删除者
pub const COL_DELETED_BY: &str = "deleted_by";
/// 版本控制 — 内容修订版本号
pub const COL_VERSION: &str = "version";
/// 乐观锁 — 锁版本号
pub const COL_LOCK_VERSION: &str = "lock_version";
/// 排序 — 排序键
pub const COL_SORT_KEY: &str = "created_at";
/// 过期 — 过期时间
pub const COL_EXPIRES_AT: &str = "expires_at";
/// 嵌套 — 父级 ID
pub const COL_PARENT_ID: &str = "parent_id";
/// 嵌套 — 层级深度
pub const COL_DEPTH: &str = "depth";
/// 嵌套 — 同级位置
pub const COL_POSITION: &str = "position";
/// 元数据 JSON 列
pub const COL_META: &str = "__meta";
/// 租户 ID 列
pub const COL_TENANT_ID: &str = "tenant_id";
/// 主键列
pub const COL_ID: &str = "id";

// ─── Auth Header ───

pub const HEADER_AUTHORIZATION: &str = "authorization";
pub const HEADER_TENANT_ID: &str = "x-tenant-id";
pub const HEADER_API_TOKEN: &str = "x-api-token";
pub const AUTH_BEARER_PREFIX: &str = "Bearer ";

// ─── API 路由前缀 ───

/// API 基础路径
pub const API_PREFIX: &str = "/api/v1";
/// Content Type 公开路由前缀（完整路径）
pub const CMS_PREFIX: &str = "/api/v1/cms";
/// Content Type 管理路由前缀（完整路径）
pub const CMS_ADMIN_PREFIX: &str = "/api/v1/admin/cms";
/// Content Type 公开路由段（相对 API_PREFIX）
pub const CMS_ROUTE: &str = "/cms";
/// Content Type 管理路由段（相对 API_PREFIX）
pub const CMS_ADMIN_ROUTE: &str = "/admin/cms";

/// 插件宿主函数全局对象名（JS/Lua）
pub const PLUGIN_HOST_GLOBAL: &str = "RaisFastHost";
