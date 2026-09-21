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
    format!("recognize/{job_id}")
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
    pool: &crate::db::Pool,
    storage: &Arc<dyn Storage>,
    queue: &crate::worker::DefaultJobQueue,
    tenant: &str,
    filename: &str,
    image: &[u8],
    model: Option<&str>,
    prompt: Option<&str>,
    callback_url: Option<&str>,
) -> AppResult<String> {
    if image.len() > MAX_IMAGE_BYTES {
        return Err(AppError::BadRequest(format!(
            "image too large: {} bytes (max {MAX_IMAGE_BYTES})",
            image.len()
        )));
    }
    let job_id_int = crate::utils::id::new_id();
    let job_id = job_id_int.to_string();

    // 用量账本（§7）
    crate::docparse::logs::insert(
        pool,
        job_id_int,
        tenant,
        "recognize",
        "queued",
        None,
        model,
        filename,
        image.len() as i64,
        None,
    )
    .await?;

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
            "callback_url": callback_url,
            "created_at": crate::utils::tz::now_utc().to_rfc3339(),
        }),
    )
    .await?;

    let mut new_job = crate::worker::NewJob::from(crate::worker::Job::RecognizeImage {
        job_id: job_id.clone(),
        tenant_id: tenant.to_string(),
        model: model.map(str::to_string),
        prompt: prompt.map(str::to_string),
    });
    new_job.timeout_secs = Some(crate::worker::LONG_JOB_TIMEOUT_SECS);
    queue
        .enqueue(new_job)
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

/// Vision 调用：经 llm 底座 router（日志记 Kb 源——识别能力自 KB 演化，
/// M3 可增设独立 LogSource）[抄RF:kb/images.rs vlm_call，依赖反转]。
async fn vlm_call(
    router: &Arc<crate::llm::service::LlmRouter>,
    tenant: &str,
    model: &str,
    system: &str,
    user: &str,
    image_ref: &str,
) -> AppResult<String> {
    let messages = [
        raisfast_agent::messages::ChatMessage {
            role: raisfast_agent::messages::ChatRole::System,
            content: Some(system.to_string()),
            images: Vec::new(),
            tool_calls: None,
            tool_call_id: None,
        },
        raisfast_agent::messages::ChatMessage {
            role: raisfast_agent::messages::ChatRole::User,
            content: Some(user.to_string()),
            images: vec![image_ref.to_string()],
            tool_calls: None,
            tool_call_id: None,
        },
    ];
    let request = raisfast_agent::provider::ChatRequest {
        messages: &messages,
        tools: None,
        temperature: Some(0.0),
        max_tokens: None,
        stop: None,
    };
    let resp = router
        .call(tenant, crate::llm::models::log::LogSource::Kb)
        .chat(Some(model), &request)
        .await?;
    Ok(resp.text.unwrap_or_default())
}

/// worker 执行体：running → VLM caption + OCR → completed/failed。
/// （参数 8 个系依赖显式化的代价——底座模块不持 KbDeps，见 §3.0.3。）
#[allow(clippy::too_many_arguments)]
pub async fn run(
    pool: &crate::db::Pool,
    router: &Arc<crate::llm::service::LlmRouter>,
    image_model_default: Option<&str>,
    storage: &Arc<dyn Storage>,
    job_id: &str,
    tenant: &str,
    model: Option<&str>,
    prompt: Option<&str>,
) -> AppResult<()> {
    let started = std::time::Instant::now();
    let raw = storage.get(&meta_key(job_id)).await?;
    let mut meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta decode: {e}")))?;
    meta["status"] = json!("running");
    write_meta(storage, job_id, meta.clone()).await?;

    let resolved_model = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let result = recognize_inner(
        router,
        image_model_default,
        storage,
        tenant,
        &mut meta,
        model,
        prompt,
    )
    .await;
    match result {
        Ok(()) => {
            let _ = crate::docparse::logs::finish(
                pool,
                job_id.parse().unwrap_or(0),
                "completed",
                resolved_model.as_deref(),
                None,
                Some(meta["caption"].as_str().map_or(0, |s| s.chars().count()) as i64),
                Some(started.elapsed().as_millis() as i64),
                None,
            )
            .await;
            meta["status"] = json!("completed");
            meta["finished_at"] = json!(crate::utils::tz::now_utc().to_rfc3339());
            write_meta(storage, job_id, meta.clone()).await?;
            fire_callback(job_id, &meta).await;
            Ok(())
        }
        Err(e) => {
            let _ = crate::docparse::logs::finish(
                pool,
                job_id.parse().unwrap_or(0),
                "failed",
                resolved_model.as_deref(),
                None,
                None,
                Some(started.elapsed().as_millis() as i64),
                Some(&e.to_string()),
            )
            .await;
            meta["status"] = json!("failed");
            meta["error"] = json!(e.to_string());
            meta["finished_at"] = json!(crate::utils::tz::now_utc().to_rfc3339());
            write_meta(storage, job_id, meta.clone()).await?;
            fire_callback(job_id, &meta).await;
            Err(e)
        }
    }
}

/// M3 webhook：job 终态回调（meta.callback_url，best-effort）。
async fn fire_callback(job_id: &str, meta: &serde_json::Value) {
    if let Some(cb) = meta["callback_url"].as_str() {
        crate::docparse::webhook::fire(
            cb,
            &json!({
                "job_id": job_id,
                "kind": "recognize",
                "status": meta["status"],
                "caption": meta["caption"],
                "ocr_text": meta["ocr_text"],
                "error": meta["error"],
            }),
        )
        .await;
    }
}

async fn recognize_inner(
    router: &Arc<crate::llm::service::LlmRouter>,
    image_model_default: Option<&str>,
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
        .or_else(|| image_model_default.map(str::to_string))
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

    let caption = vlm_call(
        router,
        tenant,
        &model,
        &caption_system,
        &format!("Describe this image in Chinese.{user_note}"),
        &image_ref,
    )
    .await?;
    let ocr = vlm_call(
        router,
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
    use crate::config::app::AppConfig;
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

    async fn deps(
        pool: crate::db::Pool,
    ) -> (
        Arc<crate::llm::service::LlmRouter>,
        Arc<AppConfig>,
        Arc<dyn Storage>,
    ) {
        let router = crate::llm::service::LlmRouter::with_provider_for_test(
            Some(pool),
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
        (router, Arc::new(config), storage)
    }

    #[tokio::test]
    async fn recognize_job_lifecycle() {
        let pool = crate::test_pool!();
        let (router, _config, storage) = deps(pool.clone()).await;
        let queue = crate::worker::DefaultJobQueue::new(pool.clone());

        let job_id = submit(
            &pool,
            &storage,
            &queue,
            "default",
            "photo.png",
            b"PNGDATA",
            Some("kb-test-model"),
            None,
            None,
        )
        .await
        .unwrap();

        // worker 执行体（等价 RecognizeImageHandler）
        run(
            &pool,
            &router,
            Some("kb-test-model"),
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
        let pool = crate::test_pool!();
        let (router, _config, storage) = deps(pool.clone()).await;
        let queue = crate::worker::DefaultJobQueue::new(pool.clone());
        let job_id = submit(
            &pool, &storage, &queue, "default", "a.png", b"X", None, None, None,
        )
        .await
        .unwrap();
        // 无模型（参数与全局皆无）→ job 显式 failed，不静默
        let _ = run(
            &pool, &router, None, &storage, &job_id, "default", None, None,
        )
        .await;
        let meta = read_status(&storage, &job_id).await.unwrap().unwrap();
        assert_eq!(meta["status"], "failed");
    }
}
