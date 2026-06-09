//! Sitemap generation Handler
//!
//! Queries all published posts, generates a sitemap.xml and writes it to `{static_dir}/sitemap.xml`.
//! Follows the [sitemaps.org protocol](https://www.sitemaps.org/protocol.html).

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// Sitemap generation handler
pub struct GenerateSitemapHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl GenerateSitemapHandler {
    /// Creates a new sitemap generation handler
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }

    fn build_xml(base_url: &str, posts: &[crate::models::post::Post]) -> String {
        let mut urls = Vec::new();

        urls.push(xml_url(base_url, None, None));
        for p in posts {
            let loc = format!("{}/posts/{}", base_url, p.slug);
            let lastmod = p.updated_at.to_rfc3339();
            urls.push(xml_url(&loc, Some(&lastmod), None));
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

        let (posts, _) =
            crate::models::post::find_published(&self.pool, 1, 50000, None, None, None, None)
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
    use crate::types::snowflake_id::SnowflakeId;

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
            id: crate::types::snowflake_id::SnowflakeId(1i64),
            tenant_id: Some(crate::constants::DEFAULT_TENANT.to_string()),
            title: "Hello".into(),
            slug: "hello".into(),
            content: "".into(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: crate::models::post::PostStatus::Published,
            created_by: crate::types::snowflake_id::SnowflakeId(1i64),
            updated_by: Some(crate::types::snowflake_id::SnowflakeId(1i64)),
            category_id: None,
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: crate::models::post::CommentOpenStatus::Open,
            format: "standard".into(),
            template: "default".into(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2025-01-02T00:00:00Z".parse().unwrap(),
            published_at: Some("2025-01-01T00:00:00Z".parse().unwrap()),
        }];
        let xml = GenerateSitemapHandler::build_xml("http://example.com", &posts);
        assert!(xml.contains("<loc>http://example.com/posts/hello</loc>"));
        assert!(xml.contains("<lastmod>2025-01-02T00:00:00+00:00</lastmod>"));
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let config = Arc::new(test_config());
        let handler = GenerateSitemapHandler::new(pool, config);
        let job = Job::SendWelcomeEmail {
            user_id: SnowflakeId(1),
            email: "a@b.com".into(),
            username: "alice".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    fn test_config() -> crate::config::app::AppConfig {
        crate::config::app::AppConfig::test_defaults()
    }
}
