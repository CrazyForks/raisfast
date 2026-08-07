//! Job Handler trait and registry

use std::collections::HashMap;

#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;

use super::Job;

/// Self-describing metadata for a cron handler — powers the admin task menu.
///
/// Handlers registered via [`JobHandlerRegistry::register_with_meta`] attach a
/// `&'static HandlerMeta`. The admin `GET /admin/cron-handlers` endpoint lists
/// all metas so the frontend can render a task picker + dynamic parameter form.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "export-types", derive(TS))]
pub struct HandlerMeta {
    /// Unique identifier (snake_case), same as the registered `job_type`.
    pub id: &'static str,
    /// Display name shown in the admin UI.
    pub display_name: &'static str,
    /// One-line description of what this task does.
    pub description: &'static str,
    /// Category for grouping in the task menu (e.g. "系统维护", "内容").
    pub category: &'static str,
    /// JSON Schema (draft-07) as a raw string. Parsed to `Value` at runtime.
    /// `None` = no params.
    #[cfg_attr(feature = "export-types", ts(type = "string | null"))]
    pub params_schema: Option<&'static str>,
    /// Optional icon identifier for the admin UI.
    pub icon: Option<&'static str>,
}

/// Job handler trait
#[async_trait::async_trait]
pub trait JobHandler: Send + Sync {
    async fn handle(&self, job: &Job) -> AppResult<()>;

    /// Return a coalesce key for this job. If two or more jobs in a batch share
    /// the same key, they are merged via [`JobHandler::coalesce`] and executed
    /// once instead of N times.
    #[allow(unused_variables)]
    fn coalesce_key(&self, job: &Job) -> Option<String> {
        None
    }

    /// Merge multiple jobs with the same coalesce key into a single job.
    #[allow(unused_variables)]
    fn coalesce(&self, jobs: Vec<Job>) -> Option<Job> {
        None
    }
}

/// Handler registry
pub struct JobHandlerRegistry {
    handlers: HashMap<String, Box<dyn JobHandler>>,
    metas: HashMap<String, &'static HandlerMeta>,
}

impl JobHandlerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            metas: HashMap::new(),
        }
    }

    /// Registers a handler without metadata (invisible in the cron task menu).
    pub fn register(&mut self, job_type: &str, handler: Box<dyn JobHandler>) {
        self.handlers.insert(job_type.to_string(), handler);
    }

    /// Registers a handler **with** metadata — makes it appear in the cron task menu.
    pub fn register_with_meta(
        &mut self,
        job_type: &str,
        handler: Box<dyn JobHandler>,
        meta: &'static HandlerMeta,
    ) {
        self.handlers.insert(job_type.to_string(), handler);
        self.metas.insert(job_type.to_string(), meta);
    }

    /// Checks if a handler is registered
    #[must_use]
    pub fn has_handler(&self, job_type: &str) -> bool {
        self.handlers.contains_key(job_type)
    }

    /// Returns a reference to the registered handler, if any.
    #[must_use]
    pub fn get_handler(&self, job_type: &str) -> Option<&dyn JobHandler> {
        self.handlers.get(job_type).map(|b| b.as_ref())
    }

    /// Returns the metadata for a registered handler, if it has one.
    #[must_use]
    pub fn get_meta(&self, job_type: &str) -> Option<&'static HandlerMeta> {
        self.metas.get(job_type).copied()
    }

    /// Lists all handler metas that have been registered with metadata.
    pub fn list_meta(&self) -> Vec<&'static HandlerMeta> {
        self.metas.values().copied().collect()
    }

    pub async fn handle(&self, job: &Job) -> AppResult<()> {
        let job_type = job.job_type();
        if let Some(handler) = self.handlers.get(job_type) {
            handler.handle(job).await
        } else {
            tracing::warn!("no handler registered for job type: {job_type}");
            Ok(())
        }
    }
}

impl Default for JobHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Log handler — records job execution (default handler, used during development)
pub struct LogJobHandler;

#[async_trait::async_trait]
impl JobHandler for LogJobHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        tracing::info!("[worker] executing job: {}", job.job_type());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    struct FailHandler;

    #[async_trait::async_trait]
    impl JobHandler for FailHandler {
        async fn handle(&self, _job: &Job) -> AppResult<()> {
            Err(crate::errors::app_error::AppError::BadRequest(
                "forced failure".into(),
            ))
        }
    }

    #[tokio::test]
    async fn registry_dispatches_to_registered_handler() {
        let mut registry = JobHandlerRegistry::new();
        registry.register("generate_sitemap", Box::new(LogJobHandler));

        let job = Job::GenerateSitemap;
        let result = registry.handle(&job).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn registry_returns_ok_for_unregistered_type() {
        let registry = JobHandlerRegistry::new();
        let job = Job::GenerateSitemap;
        let result = registry.handle(&job).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn registry_propagates_handler_error() {
        let mut registry = JobHandlerRegistry::new();
        registry.register("generate_sitemap", Box::new(FailHandler));

        let job = Job::GenerateSitemap;
        let result = registry.handle(&job).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn log_job_handler_succeeds() {
        let handler = LogJobHandler;
        let job = Job::GenerateSitemap;
        let result = handler.handle(&job).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn log_job_handler_succeeds_for_various_jobs() {
        let handler = LogJobHandler;
        let jobs = vec![
            Job::SendWelcomeEmail {
                user_id: SnowflakeId(1),
                email: "a@b.com".into(),
                username: "alice".into(),
            },
            Job::RebuildSearchIndex { post_ids: vec![1] },
            Job::GenerateThumbnail {
                media_id: SnowflakeId(1),
                size: 300,
            },
        ];
        for job in &jobs {
            assert!(handler.handle(job).await.is_ok());
        }
    }

    #[test]
    fn registry_default_is_new() {
        let registry = JobHandlerRegistry::default();
        assert!(registry.handlers.is_empty());
    }

    #[test]
    fn has_handler_checks_registration() {
        let mut registry = JobHandlerRegistry::new();
        assert!(!registry.has_handler("generate_sitemap"));
        registry.register("generate_sitemap", Box::new(LogJobHandler));
        assert!(registry.has_handler("generate_sitemap"));
        assert!(!registry.has_handler("unknown_type"));
    }
}
