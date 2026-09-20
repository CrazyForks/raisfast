//! 图像识别公共服务（design: dev-docs/document/service-design.md M2）。
//!
//! 把 VLM caption/OCR 从"文档附图"流程中解耦：接受独立图片，异步 job
//! 返回 `{caption, ocr_text}`。VLM 调用复用 images.rs 的 vlm_call
//! （llm 底座 vision 通道，模型经请求参数或全局默认解析）。

use std::sync::Arc;

use serde_json::json;

use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;
use crate::utils::prompt_file::prompt_file;
use crate::worker::JobQueue as _;

/// 单图上限。
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

fn storage_dir(job_id: &str) -> String {
    format!("kb/recognize/{job_id}")
}

fn meta_key(job_id: &str) -> String {
    format!("{}/meta.json", storage_dir(job_id))
}

fn input_key(job_id: &str, ext: &str) -> String {
    let ext = if ext.is_empty() { "png" } else { ext };
    format!("{}/input.{ext}", storage_dir(job_id))
}

/// 提交识别 job。`model` 缺省时在 run 阶段解析为全局
/// `RAISFAST_KB_IMAGE_MODEL`；两者皆无 → job failed（显式可见）。
#[allow(clippy::too_many_arguments)]
pub async fn submit(
    storage: &Arc<dyn Storage>,
    queue: &crate::worker::DefaultJobQueue,
    tenant: &str,
    filename: &str,
    image: &[u8],
    model: Option<&str>,
    prompt: Option<&str>,
) -> AppResult<String> {
    if image.len() > MAX_IMAGE_BYTES {
        return Err(AppError::BadRequest(format!(
            "image too large: {} bytes (max {MAX_IMAGE_BYTES})",
            image.len()
        )));
    }
    let job_id = crate::utils::id::new_id().to_string();
    let ext = filename
        .rsplit('.')
        .next()
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .unwrap_or("png")
        .to_ascii_lowercase();
    let input = input_key(&job_id, &ext);
    storage
        .put(&input, image, "application/octet-stream")
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("store image: {e}")))?;

    write_meta(
        storage,
        &job_id,
        json!({
            "status": "queued",
            "filename": filename,
            "model": model,
            "input_key": input,
            "created_at": crate::utils::tz::now_utc().to_rfc3339(),
        }),
    )
    .await?;

    queue
        .enqueue(crate::worker::NewJob::from(
            crate::worker::Job::RecognizeImage {
                job_id: job_id.clone(),
                tenant_id: tenant.to_string(),
                model: model.map(str::to_string),
                prompt: prompt.map(str::to_string),
            },
        ))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("enqueue recognize job: {e}")))?;
    Ok(job_id)
}

/// 读 job 状态；completed 时返回 `{caption, ocr_text}`。
pub async fn read_status(
    storage: &Arc<dyn Storage>,
    job_id: &str,
) -> AppResult<Option<serde_json::Value>> {
    let raw = match storage.get(&meta_key(job_id)).await {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta decode: {e}")))?;
    Ok(Some(meta))
}

async fn write_meta(
    storage: &Arc<dyn Storage>,
    job_id: &str,
    meta: serde_json::Value,
) -> AppResult<()> {
    let payload = serde_json::to_vec_pretty(&meta)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta encode: {e}")))?;
    storage
        .put(&meta_key(job_id), &payload, "application/json")
        .await
}

/// worker 执行体：running → VLM caption + OCR → completed/failed。
pub async fn run(
    deps: &crate::kb::service::KbDeps,
    storage: &Arc<dyn Storage>,
    job_id: &str,
    tenant: &str,
    model: Option<&str>,
    prompt: Option<&str>,
) -> AppResult<()> {
    let raw = storage.get(&meta_key(job_id)).await?;
    let mut meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta decode: {e}")))?;
    meta["status"] = json!("running");
    write_meta(storage, job_id, meta.clone()).await?;

    let result = recognize_inner(deps, storage, tenant, &mut meta, model, prompt).await;
    match result {
        Ok(()) => {
            meta["status"] = json!("completed");
            meta["finished_at"] = json!(crate::utils::tz::now_utc().to_rfc3339());
            write_meta(storage, job_id, meta).await?;
            Ok(())
        }
        Err(e) => {
            meta["status"] = json!("failed");
            meta["error"] = json!(e.to_string());
            meta["finished_at"] = json!(crate::utils::tz::now_utc().to_rfc3339());
            write_meta(storage, job_id, meta).await?;
            Err(e)
        }
    }
}

async fn recognize_inner(
    deps: &crate::kb::service::KbDeps,
    storage: &Arc<dyn Storage>,
    tenant: &str,
    meta: &mut serde_json::Value,
    model: Option<&str>,
    prompt: Option<&str>,
) -> AppResult<()> {
    let model = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| deps.config.kb.image_model.clone())
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| {
            AppError::BadRequest(
                "no recognition model: pass model or set RAISFAST_KB_IMAGE_MODEL".into(),
            )
        })?;
    meta["model"] = json!(model);

    let input_key = meta["input_key"]
        .as_str()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("meta missing input_key")))?
        .to_string();
    let bytes = storage.get(&input_key).await?;
    let mime = if input_key.ends_with(".png") {
        "image/png"
    } else if input_key.ends_with(".webp") {
        "image/webp"
    } else if input_key.ends_with(".gif") {
        "image/gif"
    } else {
        "image/jpeg"
    };
    use base64::Engine as _;
    let image_ref = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );

    let caption_system = prompt_file!("src/kb/prompts/image_caption.md");
    let ocr_system = prompt_file!("src/kb/prompts/image_ocr.md");
    let user_note = prompt.unwrap_or_default();

    let caption = crate::kb::images::vlm_call(
        deps,
        tenant,
        &model,
        &caption_system,
        &format!("Describe this image in Chinese.{user_note}"),
        &image_ref,
    )
    .await?;
    let ocr = crate::kb::images::vlm_call(
        deps,
        tenant,
        &model,
        &ocr_system,
        "Extract the text content of this image.",
        &image_ref,
    )
    .await?;

    meta["caption"] = json!(caption.trim());
    meta["ocr_text"] = json!(ocr.trim());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::service::KbDeps;
    use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

    struct MockVlm;

    #[async_trait::async_trait]
    impl ModelProvider for MockVlm {
        fn name(&self) -> &str {
            "mock-vlm"
        }
        async fn chat(
            &self,
            _r: &ChatRequest<'_>,
            _m: &str,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse::text_only("MOCK_VLM_TEXT"))
        }
    }

    struct NoopEmbedder;

    #[async_trait::async_trait]
    impl crate::kb::service::KbEmbedder for NoopEmbedder {
        async fn embed(&self, _tenant: &str, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.0_f32; 4]).collect())
        }
    }

    async fn deps() -> (KbDeps, Arc<dyn Storage>) {
        let pool = crate::test_pool!();
        let router = crate::llm::service::LlmRouter::with_provider_for_test(
            Some(pool.clone()),
            Arc::new(MockVlm),
            &["kb-test-model"],
            Some("kb-test-model"),
        )
        .await;
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        let storage: Arc<dyn Storage> = Arc::new(
            crate::storage::local::LocalStorage::new("/tmp/kb-recognize-test/uploads", "/uploads")
                .unwrap(),
        );
        let deps = KbDeps {
            pool: pool.clone(),
            config: Arc::new(config),
            storage: storage.clone(),
            vector: Arc::new(crate::kb::vectors::BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(NoopEmbedder),
            reranker: None,
            parsers: Arc::new(crate::kb::parser::ParserRegistry::new(Vec::new())),
            router,
            emitter: crate::event::EventEmitter::eventbus_only(crate::eventbus::EventBus::new(16)),
        };
        (deps, storage)
    }

    #[tokio::test]
    async fn recognize_job_lifecycle() {
        let (deps, storage) = deps().await;
        let queue = crate::worker::DefaultJobQueue::new(deps.pool.clone());

        let job_id = submit(
            &storage,
            &queue,
            "default",
            "photo.png",
            b"PNGDATA",
            Some("kb-test-model"),
            None,
        )
        .await
        .unwrap();

        // worker 执行体（等价 RecognizeImageHandler）
        run(
            &deps,
            &storage,
            &job_id,
            "default",
            Some("kb-test-model"),
            None,
        )
        .await
        .unwrap();

        let meta = read_status(&storage, &job_id).await.unwrap().unwrap();
        assert_eq!(meta["status"], "completed");
        assert_eq!(meta["caption"], "MOCK_VLM_TEXT");
        assert_eq!(meta["ocr_text"], "MOCK_VLM_TEXT");
    }

    #[tokio::test]
    async fn recognize_without_model_fails_visible() {
        let (deps, storage) = deps().await;
        let queue = crate::worker::DefaultJobQueue::new(deps.pool.clone());
        let job_id = submit(&storage, &queue, "default", "a.png", b"X", None, None)
            .await
            .unwrap();
        // 无模型（参数与全局皆无）→ job 显式 failed，不静默
        let _ = run(&deps, &storage, &job_id, "default", None, None).await;
        let meta = read_status(&storage, &job_id).await.unwrap().unwrap();
        assert_eq!(meta["status"], "failed");
    }
}
