//! 应用配置结构体与加载逻辑。
//!
//! 所有配置项通过环境变量传入，支持 `.env` 文件。
//! 缺失的可选变量会使用合理的默认值。

use std::env;

use serde::{Deserialize, Serialize};

/// 应用全局配置。
///
/// | 环境变量 | 类型 | 默认值 | 说明 |
/// |----------|------|--------|------|
/// | `APP_HOST` | String | `0.0.0.0` | 监听地址 |
/// | `APP_PORT` | u16 | `3000` | 监听端口 |
/// | `APP_ENV` | String | `development` | 运行环境 |
/// | `DATABASE_URL` | String | (按数据库后端不同) | 数据库连接字符串 |
/// | `DB_POOL_SIZE` | u32 | `5` | 连接池大小 |
/// | `JWT_SECRET` | String | (内置默认值) | JWT 签名密钥 |
/// | `JWT_ACCESS_EXPIRES` | u64 | `900` (15 分钟) | Access Token 过期时间（秒） |
/// | `JWT_REFRESH_EXPIRES` | u64 | `604800` (7 天) | Refresh Token 过期时间（秒） |
/// | `UPLOAD_DIR` | String | `./uploads` | 上传文件存储目录 |
/// | `MAX_UPLOAD_SIZE` | usize | `104857600` (100 MB) | 上传文件大小上限（字节） |
/// | `STATIC_DIR` | String | `./static` | 静态文件目录（favicon、robots.txt 等） |
/// | `BASE_URL` | String | `http://{host}:{port}` | 站点完整 URL（用于生成 RSS/媒体链接） |
/// | `CORS_ORIGINS` | String | (空=允许所有) | CORS 允许的来源，多个用逗号分隔 |
/// | `TLS_CERT_PATH` | String | (空=HTTP) | TLS 证书文件路径（PEM 格式） |
/// | `TLS_KEY_PATH` | String | (空=HTTP) | TLS 私钥文件路径（PEM 格式） |
/// | `PLUGIN_WASM_POOL_SIZE` | u32 | `4` | WASM 实例池大小 |
/// | `PLUGIN_LUA_POOL_SIZE` | u32 | `4` | Lua 实例池大小 |
/// | `PLUGIN_JS_POOL_SIZE` | u32 | `4` | JS 实例池大小 |
/// | `APP_TIMEZONE` | String | `UTC` | 站点时区（IANA 格式，如 `Asia/Shanghai`） |
/// | `GRAPHQL_ENABLED` | bool | `false` | 是否启用 GraphQL API |
/// | `WEBSOCKET_ENABLED` | bool | `false` | 是否启用 WebSocket 实时推送 |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub env: String,
    pub database_url: String,
    pub db_pool_size: u32,
    pub jwt_secret: String,
    pub jwt_access_expires: u64,
    pub jwt_refresh_expires: u64,
    pub upload_dir: String,
    pub max_upload_size: usize,
    pub static_dir: String,
    pub base_url: String,
    pub cors_origins: Option<String>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub plugin_dir: Option<String>,
    #[serde(default)]
    pub plugin_hot_reload: bool,
    #[serde(default = "default_plugin_max_memory")]
    pub plugin_max_memory_mb: u32,
    #[serde(default = "default_plugin_timeout")]
    pub plugin_default_timeout_ms: u64,
    #[serde(default = "default_plugin_wasm_pool_size")]
    pub plugin_wasm_pool_size: u32,
    #[serde(default = "default_plugin_lua_pool_size")]
    pub plugin_lua_pool_size: u32,
    #[serde(default = "default_plugin_js_pool_size")]
    pub plugin_js_pool_size: u32,
    #[serde(default)]
    pub plugin_disabled: Vec<String>,
    #[serde(default = "default_plugin_vfs_root")]
    pub plugin_vfs_root: String,
    #[serde(default = "default_plugin_vfs_max_file_size")]
    pub plugin_vfs_max_file_size: usize,
    #[serde(default = "default_plugin_vfs_max_total_size")]
    pub plugin_vfs_max_total_size: usize,
    #[serde(default = "default_log_dir")]
    pub log_dir: String,
    #[serde(default = "default_log_max_files")]
    pub log_max_files: usize,
    #[serde(default = "default_rate_limit_global_max")]
    pub rate_limit_global_max: u32,
    #[serde(default = "default_rate_limit_global_window")]
    pub rate_limit_global_window: u64,
    #[serde(default = "default_rate_limit_register_max")]
    pub rate_limit_register_max: u32,
    #[serde(default = "default_rate_limit_register_window")]
    pub rate_limit_register_window: u64,
    #[serde(default = "default_rate_limit_login_max")]
    pub rate_limit_login_max: u32,
    #[serde(default = "default_rate_limit_login_window")]
    pub rate_limit_login_window: u64,
    #[serde(default = "default_rate_limit_comment_max")]
    pub rate_limit_comment_max: u32,
    #[serde(default = "default_rate_limit_comment_window")]
    pub rate_limit_comment_window: u64,
    #[serde(default = "default_rate_limit_api_token_max")]
    pub rate_limit_api_token_max: u32,
    #[serde(default = "default_rate_limit_api_token_window")]
    pub rate_limit_api_token_window: u64,
    #[serde(default)]
    pub worker_enabled: bool,
    #[serde(default = "default_worker_concurrency")]
    pub worker_concurrency: usize,
    #[serde(default = "default_worker_poll_interval_ms")]
    pub worker_poll_interval_ms: u64,
    #[serde(default = "default_worker_max_attempts")]
    pub worker_default_max_attempts: u32,
    #[serde(default = "default_worker_cron_tick_ms")]
    pub worker_cron_tick_ms: u64,
    #[serde(default)]
    pub cron_seed_enabled: bool,
    #[serde(default = "default_cron_schedules")]
    pub cron_schedules: Vec<CronScheduleConfig>,
    #[serde(default = "default_cron_log_retention_days")]
    pub cron_log_retention_days: i64,
    #[serde(default = "default_search_engine")]
    pub search_engine: String,
    #[serde(default = "default_search_index_dir")]
    pub search_index_dir: String,
    #[serde(default = "default_content_type_dir")]
    pub content_type_dir: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_extension_dir")]
    pub extension_dir: String,
    #[serde(default = "default_protected_tables")]
    pub protected_tables: Vec<String>,
    #[serde(default = "default_storage_driver")]
    pub storage_driver: String,
    pub s3_endpoint: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    #[serde(default = "default_s3_bucket")]
    pub s3_bucket: String,
    #[serde(default = "default_s3_region")]
    pub s3_region: String,
    pub s3_public_url: Option<String>,
    #[serde(default)]
    pub rule_engine: RuleEngineConfig,
    /// 是否启用 GraphQL API（默认 false）
    #[serde(default)]
    pub graphql_enabled: bool,
    /// 是否启用 WebSocket 实时推送（默认 false）
    #[serde(default)]
    pub websocket_enabled: bool,
    #[serde(default)]
    pub oauth: crate::config::oauth::OAuthConfig,
    #[serde(default = "default_true")]
    pub registration_email_enabled: bool,
    #[serde(default)]
    pub registration_sms_enabled: bool,
    #[serde(default = "default_sms_code_expires_in")]
    pub sms_code_expires_in: u64,
    #[serde(default = "default_sms_code_length")]
    pub sms_code_length: u32,
    #[serde(default = "default_sms_rate_limit_secs")]
    pub sms_rate_limit_secs: u64,
    #[serde(default)]
    pub require_email_verification: bool,
    #[serde(default = "default_email_provider")]
    pub email_provider: String,
    pub email_smtp_host: Option<String>,
    #[serde(default = "default_email_smtp_port")]
    pub email_smtp_port: u16,
    pub email_smtp_user: Option<String>,
    pub email_smtp_pass: Option<String>,
    pub email_from: Option<String>,
    pub email_from_name: Option<String>,
    pub email_sendgrid_api_key: Option<String>,
    pub email_resend_api_key: Option<String>,
    pub email_aliyun_access_key_id: Option<String>,
    pub email_aliyun_access_key_secret: Option<String>,
    pub email_aliyun_region: Option<String>,
    pub email_tencent_secret_id: Option<String>,
    pub email_tencent_secret_key: Option<String>,
    pub email_tencent_region: Option<String>,
    #[serde(default = "default_sms_provider")]
    pub sms_provider: String,
    pub sms_aliyun_access_key_id: Option<String>,
    pub sms_aliyun_access_key_secret: Option<String>,
    pub sms_aliyun_sign_name: Option<String>,
    pub sms_aliyun_template_code: Option<String>,
    pub sms_twilio_account_sid: Option<String>,
    pub sms_twilio_auth_token: Option<String>,
    pub sms_twilio_from: Option<String>,
}

/// API Rule 引擎配置
///
/// 所有规则引擎中的硬编码值均可通过环境变量覆盖，便于：
/// - 适配不同数据库后端（SQLite / PostgreSQL / MySQL）
/// - 调整缓存策略
/// - 自定义表达式前缀
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEngineConfig {
    /// 表达式前缀：认证用户 ID（默认 `@request.auth.id`）
    pub prefix_auth_id: String,
    /// 表达式前缀：认证用户角色（默认 `@request.auth.role`）
    pub prefix_auth_role: String,
    /// 表达式前缀：请求体字段（默认 `@request.body.`）
    pub prefix_request_body: String,
    /// 表达式前缀：URL 查询参数（默认 `@request.query.`）
    pub prefix_request_query: String,
    /// 表达式前缀：当前时间（默认 `@now`）
    pub prefix_now: String,
    /// 表达式前缀：跨表引用（默认 `@table.`，Phase 3 使用）
    pub prefix_cross_table: String,
    /// SQL 中 @now 编译为的函数（默认 `datetime('now')`，PG 可改为 `NOW()`）
    pub sql_now_fn: String,
    /// SQL 中 :isset 编译为的操作符（默认 `IS NOT NULL`）
    pub sql_isset_op: String,
    /// SQL 中 :length 编译为的函数名（默认 `LENGTH`，PG 可改为 `CHAR_LENGTH`）
    pub sql_length_fn: String,
    /// SQL LIKE 通配符（默认 `%`）
    pub sql_like_wildcard: String,
    /// SQL LIKE 单字符通配符（默认 `_`）
    pub sql_like_single_char: String,
    /// LIKE 通配符对应的正则（默认 `.*`）
    pub regex_like_wildcard: String,
    /// LIKE 单字符对应的正则（默认 `.`）
    pub regex_like_single_char: String,
    /// CMS 列表缓存 TTL（秒，默认 30）
    pub cms_cache_ttl_secs: u64,
}

impl Default for RuleEngineConfig {
    fn default() -> Self {
        Self {
            prefix_auth_id: "@request.auth.id".into(),
            prefix_auth_role: "@request.auth.role".into(),
            prefix_request_body: "@request.body.".into(),
            prefix_request_query: "@request.query.".into(),
            prefix_now: "@now".into(),
            prefix_cross_table: "@table.".into(),
            sql_now_fn: "datetime('now')".into(),
            sql_isset_op: "IS NOT NULL".into(),
            sql_length_fn: "LENGTH".into(),
            sql_like_wildcard: "%".into(),
            sql_like_single_char: "_".into(),
            regex_like_wildcard: ".*".into(),
            regex_like_single_char: ".".into(),
            cms_cache_ttl_secs: 30,
        }
    }
}

impl RuleEngineConfig {
    /// 从环境变量加载，缺失项使用默认值
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            prefix_auth_id: env::var("RULE_PREFIX_AUTH_ID").unwrap_or(defaults.prefix_auth_id),
            prefix_auth_role: env::var("RULE_PREFIX_AUTH_ROLE")
                .unwrap_or(defaults.prefix_auth_role),
            prefix_request_body: env::var("RULE_PREFIX_REQUEST_BODY")
                .unwrap_or(defaults.prefix_request_body),
            prefix_request_query: env::var("RULE_PREFIX_REQUEST_QUERY")
                .unwrap_or(defaults.prefix_request_query),
            prefix_now: env::var("RULE_PREFIX_NOW").unwrap_or(defaults.prefix_now),
            prefix_cross_table: env::var("RULE_PREFIX_CROSS_TABLE")
                .unwrap_or(defaults.prefix_cross_table),
            sql_now_fn: env::var("RULE_SQL_NOW_FN").unwrap_or(defaults.sql_now_fn),
            sql_isset_op: env::var("RULE_SQL_ISSET_OP").unwrap_or(defaults.sql_isset_op),
            sql_length_fn: env::var("RULE_SQL_LENGTH_FN").unwrap_or(defaults.sql_length_fn),
            sql_like_wildcard: env::var("RULE_SQL_LIKE_WILDCARD")
                .unwrap_or(defaults.sql_like_wildcard),
            sql_like_single_char: env::var("RULE_SQL_LIKE_SINGLE_CHAR")
                .unwrap_or(defaults.sql_like_single_char),
            regex_like_wildcard: env::var("RULE_REGEX_LIKE_WILDCARD")
                .unwrap_or(defaults.regex_like_wildcard),
            regex_like_single_char: env::var("RULE_REGEX_LIKE_SINGLE_CHAR")
                .unwrap_or(defaults.regex_like_single_char),
            cms_cache_ttl_secs: env::var("CMS_CACHE_TTL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.cms_cache_ttl_secs),
        }
    }
}

/// 单条 Cron 调度配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronScheduleConfig {
    pub label: String,
    pub job_type: String,
    pub payload: Option<String>,
    pub cron_expr: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_worker_cron_tick_ms() -> u64 {
    60000
}

fn default_cron_log_retention_days() -> i64 {
    30
}

fn default_search_engine() -> String {
    "none".into()
}

fn default_search_index_dir() -> String {
    "./data/search_index".into()
}

fn default_content_type_dir() -> String {
    "./content_types".into()
}

fn default_timezone() -> String {
    "UTC".into()
}

fn default_extension_dir() -> String {
    "./extensions".into()
}

pub fn default_protected_tables() -> Vec<String> {
    vec![
        "users".into(),
        "roles".into(),
        "permissions".into(),
        "extensions".into(),
        "audit_log".into(),
        "plugin_storage".into(),
        "options".into(),
        "rbac_roles".into(),
        "rbac_permissions".into(),
        "rbac_role_permissions".into(),
        "tenants".into(),
    ]
}

#[must_use]
pub fn default_cron_schedules() -> Vec<CronScheduleConfig> {
    vec![
        CronScheduleConfig {
            label: "Generate Sitemap".into(),
            job_type: "generate_sitemap".into(),
            payload: None,
            cron_expr: "0 0 */6 * * *".into(),
            enabled: true,
        },
        CronScheduleConfig {
            label: "Cleanup Old Jobs".into(),
            job_type: "invalidate_cache".into(),
            payload: Some(r#"{"keys":["jobs:cleanup"]}"#.into()),
            cron_expr: "0 0 3 * * *".into(),
            enabled: true,
        },
    ]
}

fn default_log_dir() -> String {
    "./logs".into()
}

fn default_log_max_files() -> usize {
    7
}

fn default_rate_limit_global_max() -> u32 {
    60
}

fn default_rate_limit_global_window() -> u64 {
    60
}

fn default_rate_limit_register_max() -> u32 {
    5
}

fn default_rate_limit_register_window() -> u64 {
    3600
}

fn default_rate_limit_login_max() -> u32 {
    10
}

fn default_rate_limit_login_window() -> u64 {
    60
}

fn default_rate_limit_comment_max() -> u32 {
    3
}

fn default_rate_limit_comment_window() -> u64 {
    60
}

fn default_rate_limit_api_token_max() -> u32 {
    120
}

fn default_rate_limit_api_token_window() -> u64 {
    60
}

fn default_worker_concurrency() -> usize {
    2
}

fn default_worker_poll_interval_ms() -> u64 {
    500
}

fn default_worker_max_attempts() -> u32 {
    3
}

fn default_plugin_max_memory() -> u32 {
    32
}

fn default_plugin_timeout() -> u64 {
    5000
}

fn default_plugin_wasm_pool_size() -> u32 {
    4
}

fn default_plugin_lua_pool_size() -> u32 {
    4
}

fn default_plugin_js_pool_size() -> u32 {
    4
}

fn default_plugin_vfs_root() -> String {
    "./plugins-data".into()
}

fn default_storage_driver() -> String {
    "local".into()
}

fn default_s3_bucket() -> String {
    "blog".into()
}

fn default_s3_region() -> String {
    "us-east-1".into()
}

fn default_plugin_vfs_max_file_size() -> usize {
    1048576 // 1 MB
}

fn default_plugin_vfs_max_total_size() -> usize {
    10485760 // 10 MB
}

fn default_sms_code_expires_in() -> u64 {
    300
}

fn default_sms_code_length() -> u32 {
    6
}

fn default_sms_rate_limit_secs() -> u64 {
    60
}

fn default_email_provider() -> String {
    "log".into()
}

fn default_email_smtp_port() -> u16 {
    587
}

fn default_sms_provider() -> String {
    "log".into()
}

const DEFAULT_JWT_SECRET: &str = "change-me-in-production-at-least-32-chars";

impl AppConfig {
    /// 从环境变量构建配置，缺失变量使用默认值。
    #[must_use]
    pub fn from_env() -> Self {
        let host = env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".into());
        let port: u16 = env::var("APP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);

        let base_url = env::var("BASE_URL").unwrap_or_else(|_| format!("http://{host}:{port}"));

        let cors_origins = env::var("CORS_ORIGINS").ok().filter(|s| !s.is_empty());
        let tls_cert_path = env::var("TLS_CERT_PATH").ok().filter(|s| !s.is_empty());
        let tls_key_path = env::var("TLS_KEY_PATH").ok().filter(|s| !s.is_empty());

        Self {
            host,
            port,
            env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
                #[cfg(feature = "db-sqlite")]
                {
                    "sqlite:./data/blog.db?mode=rwc".into()
                }
                #[cfg(feature = "db-postgres")]
                {
                    "postgres://localhost/blog".into()
                }
                #[cfg(feature = "db-mysql")]
                {
                    "mysql://root@localhost/blog".into()
                }
            }),
            db_pool_size: env::var("DB_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            jwt_secret: env::var("JWT_SECRET").unwrap_or_else(|_| DEFAULT_JWT_SECRET.into()),
            jwt_access_expires: env::var("JWT_ACCESS_EXPIRES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            jwt_refresh_expires: env::var("JWT_REFRESH_EXPIRES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(604800),
            upload_dir: env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".into()),
            max_upload_size: env::var("MAX_UPLOAD_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(104857600),
            static_dir: env::var("STATIC_DIR").unwrap_or_else(|_| "./static".into()),
            base_url,
            cors_origins,
            tls_cert_path,
            tls_key_path,
            plugin_dir: env::var("PLUGIN_DIR").ok().filter(|s| !s.is_empty()),
            plugin_hot_reload: env::var("PLUGIN_HOT_RELOAD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            plugin_max_memory_mb: env::var("PLUGIN_MAX_MEMORY_MB")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_max_memory()),
            plugin_default_timeout_ms: env::var("PLUGIN_DEFAULT_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_timeout()),
            plugin_wasm_pool_size: env::var("PLUGIN_WASM_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_wasm_pool_size()),
            plugin_lua_pool_size: env::var("PLUGIN_LUA_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_lua_pool_size()),
            plugin_js_pool_size: env::var("PLUGIN_JS_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_js_pool_size()),
            plugin_disabled: env::var("PLUGIN_DISABLED")
                .ok()
                .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
                .unwrap_or_default(),
            log_dir: env::var("LOG_DIR").unwrap_or_else(|_| default_log_dir()),
            log_max_files: env::var("LOG_MAX_FILES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_log_max_files()),
            rate_limit_global_max: env::var("RATE_LIMIT_GLOBAL_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_global_max()),
            rate_limit_global_window: env::var("RATE_LIMIT_GLOBAL_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_global_window()),
            rate_limit_register_max: env::var("RATE_LIMIT_REGISTER_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_register_max()),
            rate_limit_register_window: env::var("RATE_LIMIT_REGISTER_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_register_window()),
            rate_limit_login_max: env::var("RATE_LIMIT_LOGIN_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_login_max()),
            rate_limit_login_window: env::var("RATE_LIMIT_LOGIN_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_login_window()),
            rate_limit_comment_max: env::var("RATE_LIMIT_COMMENT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_comment_max()),
            rate_limit_comment_window: env::var("RATE_LIMIT_COMMENT_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_comment_window()),
            rate_limit_api_token_max: env::var("RATE_LIMIT_API_TOKEN_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_api_token_max()),
            rate_limit_api_token_window: env::var("RATE_LIMIT_API_TOKEN_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_rate_limit_api_token_window()),
            plugin_vfs_root: env::var("PLUGIN_VFS_ROOT")
                .unwrap_or_else(|_| default_plugin_vfs_root()),
            plugin_vfs_max_file_size: env::var("PLUGIN_VFS_MAX_FILE_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_vfs_max_file_size()),
            plugin_vfs_max_total_size: env::var("PLUGIN_VFS_MAX_TOTAL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_plugin_vfs_max_total_size()),
            worker_enabled: env::var("WORKER_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            worker_concurrency: env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_worker_concurrency()),
            worker_poll_interval_ms: env::var("WORKER_POLL_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_worker_poll_interval_ms()),
            worker_default_max_attempts: env::var("WORKER_DEFAULT_MAX_ATTEMPTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_worker_max_attempts()),
            worker_cron_tick_ms: env::var("WORKER_CRON_TICK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_worker_cron_tick_ms()),
            cron_seed_enabled: env::var("CRON_SEED_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            cron_schedules: env::var("CRON_SCHEDULES")
                .ok()
                .and_then(|v| serde_json::from_str(&v).ok())
                .unwrap_or_default(),
            cron_log_retention_days: env::var("CRON_LOG_RETENTION_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_cron_log_retention_days()),
            search_engine: env::var("SEARCH_ENGINE").unwrap_or_else(|_| default_search_engine()),
            search_index_dir: env::var("SEARCH_INDEX_DIR")
                .unwrap_or_else(|_| default_search_index_dir()),
            content_type_dir: env::var("CONTENT_TYPE_DIR")
                .unwrap_or_else(|_| default_content_type_dir()),
            timezone: env::var("TIMEZONE")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_timezone),
            extension_dir: env::var("EXTENSION_DIR")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_extension_dir),
            protected_tables: env::var("PROTECTED_TABLES")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
                .unwrap_or_else(default_protected_tables),
            storage_driver: env::var("STORAGE_DRIVER")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_storage_driver),
            s3_endpoint: env::var("S3_ENDPOINT").ok().filter(|s| !s.is_empty()),
            s3_access_key: env::var("S3_ACCESS_KEY").ok().filter(|s| !s.is_empty()),
            s3_secret_key: env::var("S3_SECRET_KEY").ok().filter(|s| !s.is_empty()),
            s3_bucket: env::var("S3_BUCKET").unwrap_or_else(|_| default_s3_bucket()),
            s3_region: env::var("S3_REGION").unwrap_or_else(|_| default_s3_region()),
            s3_public_url: env::var("S3_PUBLIC_URL").ok().filter(|s| !s.is_empty()),
            rule_engine: RuleEngineConfig::from_env(),
            graphql_enabled: env::var("GRAPHQL_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            websocket_enabled: env::var("WEBSOCKET_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            oauth: crate::config::oauth::OAuthConfig::from_env(),
            registration_email_enabled: env::var("REGISTRATION_EMAIL_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(true),
            registration_sms_enabled: env::var("REGISTRATION_SMS_ENABLED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            sms_code_expires_in: env::var("SMS_CODE_EXPIRES_IN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_sms_code_expires_in()),
            sms_code_length: env::var("SMS_CODE_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_sms_code_length()),
            sms_rate_limit_secs: env::var("SMS_RATE_LIMIT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_sms_rate_limit_secs()),
            require_email_verification: env::var("REQUIRE_EMAIL_VERIFICATION")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(false),
            email_provider: env::var("EMAIL_PROVIDER")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_email_provider),
            email_smtp_host: env::var("EMAIL_SMTP_HOST").ok().filter(|s| !s.is_empty()),
            email_smtp_port: env::var("EMAIL_SMTP_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_email_smtp_port()),
            email_smtp_user: env::var("EMAIL_SMTP_USER").ok().filter(|s| !s.is_empty()),
            email_smtp_pass: env::var("EMAIL_SMTP_PASS").ok().filter(|s| !s.is_empty()),
            email_from: env::var("EMAIL_FROM").ok().filter(|s| !s.is_empty()),
            email_from_name: env::var("EMAIL_FROM_NAME").ok().filter(|s| !s.is_empty()),
            email_sendgrid_api_key: env::var("EMAIL_SENDGRID_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            email_resend_api_key: env::var("EMAIL_RESEND_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            email_aliyun_access_key_id: env::var("EMAIL_ALIYUN_ACCESS_KEY_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            email_aliyun_access_key_secret: env::var("EMAIL_ALIYUN_ACCESS_KEY_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            email_aliyun_region: env::var("EMAIL_ALIYUN_REGION")
                .ok()
                .filter(|s| !s.is_empty()),
            email_tencent_secret_id: env::var("EMAIL_TENCENT_SECRET_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            email_tencent_secret_key: env::var("EMAIL_TENCENT_SECRET_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
            email_tencent_region: env::var("EMAIL_TENCENT_REGION")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_provider: env::var("SMS_PROVIDER")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_sms_provider),
            sms_aliyun_access_key_id: env::var("SMS_ALIYUN_ACCESS_KEY_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_aliyun_access_key_secret: env::var("SMS_ALIYUN_ACCESS_KEY_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_aliyun_sign_name: env::var("SMS_ALIYUN_SIGN_NAME")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_aliyun_template_code: env::var("SMS_ALIYUN_TEMPLATE_CODE")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_twilio_account_sid: env::var("SMS_TWILIO_ACCOUNT_SID")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_twilio_auth_token: env::var("SMS_TWILIO_AUTH_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            sms_twilio_from: env::var("SMS_TWILIO_FROM").ok().filter(|s| !s.is_empty()),
        }
    }

    /// 创建用于测试的最小配置实例。
    ///
    /// 所有字段使用默认值，调用者可按需覆盖。
    #[must_use]
    pub fn test_defaults() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 3000,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "./test-uploads".into(),
            max_upload_size: 104857600,
            static_dir: "./static".into(),
            base_url: "http://localhost:3000".into(),
            cors_origins: None,
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: default_plugin_max_memory(),
            plugin_default_timeout_ms: default_plugin_timeout(),
            plugin_wasm_pool_size: default_plugin_wasm_pool_size(),
            plugin_lua_pool_size: default_plugin_lua_pool_size(),
            plugin_js_pool_size: default_plugin_js_pool_size(),
            plugin_disabled: vec![],
            plugin_vfs_root: default_plugin_vfs_root(),
            plugin_vfs_max_file_size: default_plugin_vfs_max_file_size(),
            plugin_vfs_max_total_size: default_plugin_vfs_max_total_size(),
            log_dir: "./test-logs".into(),
            log_max_files: 1,
            rate_limit_global_max: default_rate_limit_global_max(),
            rate_limit_global_window: default_rate_limit_global_window(),
            rate_limit_register_max: default_rate_limit_register_max(),
            rate_limit_register_window: default_rate_limit_register_window(),
            rate_limit_login_max: default_rate_limit_login_max(),
            rate_limit_login_window: default_rate_limit_login_window(),
            rate_limit_comment_max: default_rate_limit_comment_max(),
            rate_limit_comment_window: default_rate_limit_comment_window(),
            rate_limit_api_token_max: default_rate_limit_api_token_max(),
            rate_limit_api_token_window: default_rate_limit_api_token_window(),
            worker_enabled: false,
            worker_concurrency: default_worker_concurrency(),
            worker_poll_interval_ms: default_worker_poll_interval_ms(),
            worker_default_max_attempts: default_worker_max_attempts(),
            worker_cron_tick_ms: default_worker_cron_tick_ms(),
            cron_seed_enabled: false,
            cron_schedules: vec![],
            cron_log_retention_days: default_cron_log_retention_days(),
            search_engine: default_search_engine(),
            search_index_dir: default_search_index_dir(),
            content_type_dir: default_content_type_dir(),
            timezone: default_timezone(),
            extension_dir: default_extension_dir(),
            protected_tables: default_protected_tables(),
            storage_driver: default_storage_driver(),
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            s3_bucket: default_s3_bucket(),
            s3_region: default_s3_region(),
            s3_public_url: None,
            rule_engine: RuleEngineConfig::default(),
            graphql_enabled: false,
            websocket_enabled: false,
            oauth: crate::config::oauth::OAuthConfig {
                enabled: false,
                redirect_url: "http://localhost:3000/auth/callback".into(),
                github: None,
                google: None,
                wechat: None,
            },
            registration_email_enabled: true,
            registration_sms_enabled: false,
            sms_code_expires_in: default_sms_code_expires_in(),
            sms_code_length: default_sms_code_length(),
            sms_rate_limit_secs: default_sms_rate_limit_secs(),
            require_email_verification: false,
            email_provider: default_email_provider(),
            email_smtp_host: None,
            email_smtp_port: default_email_smtp_port(),
            email_smtp_user: None,
            email_smtp_pass: None,
            email_from: None,
            email_from_name: None,
            email_sendgrid_api_key: None,
            email_resend_api_key: None,
            email_aliyun_access_key_id: None,
            email_aliyun_access_key_secret: None,
            email_aliyun_region: None,
            email_tencent_secret_id: None,
            email_tencent_secret_key: None,
            email_tencent_region: None,
            sms_provider: default_sms_provider(),
            sms_aliyun_access_key_id: None,
            sms_aliyun_access_key_secret: None,
            sms_aliyun_sign_name: None,
            sms_aliyun_template_code: None,
            sms_twilio_account_sid: None,
            sms_twilio_auth_token: None,
            sms_twilio_from: None,
        }
    }

    /// 初始化应用配置的推荐入口。
    ///
    /// 1. 加载 `.env` 文件中的环境变量
    /// 2. 调用 `from_env()` 构建配置
    /// 3. 校验生产环境安全配置
    /// 4. 打印启动日志
    pub fn init() -> Self {
        if let Err(e) = dotenvy::dotenv() {
            tracing::warn!(".env file not loaded: {}", e);
        }
        let config = Self::from_env();

        if config.env == "production" {
            assert!(
                config.jwt_secret != DEFAULT_JWT_SECRET,
                "FATAL: JWT_SECRET must be set in production. Refusing to start with default secret."
            );
            assert!(
                config.cors_origins.is_some(),
                "FATAL: CORS_ORIGINS must be set in production. \
                 Refusing to start with wildcard CORS."
            );
        }

        tracing::info!(
            "loaded config: env={}, host={}:{}, base_url={}",
            config.env,
            config.host,
            config.port,
            config.base_url
        );
        config
    }
}
