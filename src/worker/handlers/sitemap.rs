//! Sitemap 生成 Handler
//!
//! 查询所有已发布文章，生成 sitemap.xml 写入 `{static_dir}/sitemap.xml`。
//! 遵循 [sitemaps.org 协议](https://www.sitemaps.org/protocol.html)。

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// Sitemap 生成处理器
pub struct GenerateSitemapHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl GenerateSitemapHandler {
    /// 创建新的 Sitemap 生成处理器
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }

    fn build_xml(base_url: &str, posts: &[crate::models::post::Post]) -> String {
        let mut urls = Vec::new();

        urls.push(xml_url(base_url, None, None));
        for p in posts {
            let loc = format!("{}/posts/{}", base_url, p.slug);
            let lastmod = p.updated_at.as_str();
            urls.push(xml_url(&loc, Some(lastmod), None));
        }

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\
             {}\n\
             </urlset>",
            urls.join("\n")
        )
    }
}

fn xml_url(loc: &str, lastmod: Option<&str>, changefreq: Option<&str>) -> String {
    let mut s = format!("  <url>\n    <loc>{loc}</loc>");
    if let Some(lm) = lastmod {
        s.push_str(&format!("\n    <lastmod>{lm}</lastmod>"));
    }
    if let Some(cf) = changefreq {
        s.push_str(&format!("\n    <changefreq>{cf}</changefreq>"));
    }
    s.push_str("\n  </url>");
    s
}

#[async_trait::async_trait]
impl JobHandler for GenerateSitemapHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::GenerateSitemap = job else {
            return Ok(());
        };

        let (posts, _) = crate::models::post::find_published(
            &self.pool,
            1,
            50000,
            None,
            None,
            None,
            Some(crate::db::tenant::DEFAULT_TENANT),
        )
        .await?;

        let xml = Self::build_xml(&self.config.base_url, &posts);
        let path = std::path::PathBuf::from(&self.config.static_dir).join("sitemap.xml");

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "create dir {parent:?}: {e}"
                ))
            })?;
        }

        tokio::fs::write(&path, &xml).await.map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "write sitemap {path:?}: {e}"
            ))
        })?;

        tracing::info!("[sitemap] generated sitemap.xml with {} URLs", posts.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_xml_empty_posts() {
        let xml = GenerateSitemapHandler::build_xml("http://example.com", &[]);
        assert!(xml.contains("<loc>http://example.com</loc>"));
        assert!(xml.contains("<urlset"));
    }

    #[test]
    fn build_xml_with_posts() {
        use crate::models::post::Post;
        let posts = vec![Post {
            id: "p1".into(),
            tenant_id: crate::db::tenant::DEFAULT_TENANT.into(),
            title: "Hello".into(),
            slug: "hello".into(),
            content: "".into(),
            excerpt: None,
            cover_image: None,
            status: "published".into(),
            author_id: "u1".into(),
            category_id: None,
            view_count: 0,
            is_pinned: false,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-02T00:00:00Z".into(),
            published_at: Some("2025-01-01T00:00:00Z".into()),
        }];
        let xml = GenerateSitemapHandler::build_xml("http://example.com", &posts);
        assert!(xml.contains("<loc>http://example.com/posts/hello</loc>"));
        assert!(xml.contains("<lastmod>2025-01-02T00:00:00Z</lastmod>"));
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let config = Arc::new(test_config());
        let handler = GenerateSitemapHandler::new(pool, config);
        let job = Job::SendWelcomeEmail {
            user_id: "u1".into(),
            email: "a@b.com".into(),
            username: "alice".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    fn test_config() -> crate::config::app::AppConfig {
        crate::config::app::AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "/tmp/test-uploads".into(),
            max_upload_size: 5242880,
            static_dir: "/tmp/test-static".into(),
            base_url: "http://localhost:9000".into(),
            cors_origins: None,
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 32,
            plugin_default_timeout_ms: 5000,
            plugin_disabled: vec![],
            plugin_vfs_root: "./plugins-data".into(),
            plugin_vfs_max_file_size: 1048576,
            plugin_vfs_max_total_size: 10485760,
            log_dir: "./logs".into(),
            log_max_files: 7,
            rate_limit_global_max: 60,
            rate_limit_global_window: 60,
            rate_limit_register_max: 5,
            rate_limit_register_window: 3600,
            rate_limit_login_max: 10,
            rate_limit_login_window: 60,
            rate_limit_comment_max: 3,
            rate_limit_comment_window: 60,
            worker_enabled: false,
            worker_concurrency: 1,
            worker_poll_interval_ms: 500,
            worker_default_max_attempts: 3,
            worker_cron_tick_ms: 60000,
            cron_seed_enabled: false,
            cron_schedules: vec![],
            cron_log_retention_days: 30,
            search_engine: "none".into(),
            search_index_dir: "./data/search_index".into(),
            content_type_dir: "./content_types".into(),
            timezone: "UTC".into(),
            extension_dir: "./extensions".into(),
            protected_tables: vec![],
        }
    }
}
