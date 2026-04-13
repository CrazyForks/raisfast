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
/// | `DATABASE_URL` | String | `sqlite:./data/blog.db?mode=rwc` | SQLite 连接字符串 |
/// | `DB_POOL_SIZE` | u32 | `5` | 连接池大小 |
/// | `JWT_SECRET` | String | (内置默认值) | JWT 签名密钥 |
/// | `JWT_ACCESS_EXPIRES` | u64 | `900` (15 分钟) | Access Token 过期时间（秒） |
/// | `JWT_REFRESH_EXPIRES` | u64 | `604800` (7 天) | Refresh Token 过期时间（秒） |
/// | `UPLOAD_DIR` | String | `./uploads` | 上传文件存储目录 |
/// | `MAX_UPLOAD_SIZE` | usize | `5242880` (5 MB) | 上传文件大小上限（字节） |
/// | `BASE_URL` | String | `http://{host}:{port}` | 站点完整 URL（用于生成 RSS/媒体链接） |
/// | `CORS_ORIGINS` | String | (空=允许所有) | CORS 允许的来源，多个用逗号分隔 |
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
    pub base_url: String,
    pub cors_origins: Option<String>,
}

const DEFAULT_JWT_SECRET: &str = "change-me-in-production-at-least-32-chars";

impl AppConfig {
    /// 从环境变量构建配置，缺失变量使用默认值。
    pub fn from_env() -> Self {
        let host = env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".into());
        let port: u16 = env::var("APP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);

        let base_url = env::var("BASE_URL").unwrap_or_else(|_| format!("http://{}:{}", host, port));

        let cors_origins = env::var("CORS_ORIGINS").ok().filter(|s| !s.is_empty());

        Self {
            host,
            port,
            env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:./data/blog.db?mode=rwc".into()),
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
                .unwrap_or(5242880),
            base_url,
            cors_origins,
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
            if config.jwt_secret == DEFAULT_JWT_SECRET {
                panic!(
                    "FATAL: JWT_SECRET must be set in production. Refusing to start with default secret."
                );
            }
            if config.cors_origins.is_none() {
                tracing::warn!(
                    "CORS_ORIGINS not set in production — all origins allowed. This is insecure."
                );
            }
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
