//! 任务 Handler trait 与注册表

use std::collections::HashMap;

use crate::errors::app_error::AppResult;

use super::Job;

/// 任务处理器 trait
#[async_trait::async_trait]
pub trait JobHandler: Send + Sync {
    async fn handle(&self, job: &Job) -> AppResult<()>;
}

/// Handler 注册表
pub struct JobHandlerRegistry {
    handlers: HashMap<String, Box<dyn JobHandler>>,
}

impl JobHandlerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, job_type: &str, handler: Box<dyn JobHandler>) {
        self.handlers.insert(job_type.to_string(), handler);
    }

    /// 检查是否有注册的 Handler
    #[must_use]
    pub fn has_handler(&self, job_type: &str) -> bool {
        self.handlers.contains_key(job_type)
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

/// 日志 Handler — 记录任务执行（默认 handler，用于开发阶段）
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
                user_id: 1,
                email: "a@b.com".into(),
                username: "alice".into(),
            },
            Job::RebuildSearchIndex { post_ids: vec![1] },
            Job::GenerateThumbnail {
                media_id: 1,
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
