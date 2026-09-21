//! 文档转换公共服务（design: dev-docs/document/service-design.md M1）。
//!
//! 把"传文件拿 markdown"从 KB 入库流程中解耦成独立服务：外部 API
//! (`POST /v1/document-conversions`) 与 KB 入库 (`process_document`) 共用
//! 同一转换内核；区别仅在等待方式——外部走异步 job，入库同步等结果。
//!
//! Job 生命周期：meta.json 是状态文档（submit 写 queued，worker 写
//! running/completed/failed），GET 端点只读 meta + storage URL，
//! 不回查 worker 的 jobs 行。

use std::sync::Arc;

use serde_json::json;

use crate::docparse::ParserRegistry;
use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;
use crate::worker::JobQueue as _;

/// 单文件上限（design §6：CPU 引擎大文件会拖垮 worker）。
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;

/// 结果存储键前缀（挂 /uploads 挂载点，可经 web 直接访问）。
pub fn storage_dir(job_id: &str) -> String {
    format!("parse/{job_id}")
}

pub fn meta_key(job_id: &str) -> String {
    format!("{}/meta.json", storage_dir(job_id))
}

fn input_key(job_id: &str, filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .filter(|e| !e.is_empty())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    format!("{}/input{ext}", storage_dir(job_id))
}

/// 提交参数 → 校验 → 输入落存储 → meta(queued) → 入队。
#[allow(clippy::too_many_arguments)]
pub async fn submit(
    pool: &crate::db::Pool,
    storage: &Arc<dyn Storage>,
    parsers: &ParserRegistry,
    queue: &crate::worker::DefaultJobQueue,
    tenant: &str,
    filename: &str,
    bytes: &[u8],
    engine: Option<&str>,
    extract_images: bool,
    callback_url: Option<&str>,
) -> AppResult<String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(AppError::BadRequest(format!(
            "file too large: {} bytes (max {MAX_INPUT_BYTES})",
            bytes.len()
        )));
    }
    if let Some(name) = engine
        && parsers.get(name).is_none()
    {
        return Err(AppError::BadRequest(format!(
            "unknown engine '{name}' (known: {:?})",
            parsers.names()
        )));
    }
    let job_id_int = crate::utils::id::new_id();
    let job_id = job_id_int.to_string();

    // 用量账本（§7）：提交即落行，终态回填计费字段。
    crate::docparse::logs::insert(
        pool,
        job_id_int,
        tenant,
        "convert",
        "queued",
        engine,
        None,
        filename,
        bytes.len() as i64,
        None,
    )
    .await?;

    // 输入先行落存储（worker 只拿路径，不背文件字节）。
    let input = input_key(&job_id, filename);
    storage
        .put(&input, bytes, "application/octet-stream")
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("store input: {e}")))?;

    write_meta(
        storage,
        &job_id,
        json!({
            "status": "queued",
            "filename": filename,
            "engine": engine,
            "extract_images": extract_images,
            "callback_url": callback_url,
            "input_key": input,
            "created_at": crate::utils::tz::now_utc().to_rfc3339(),
        }),
    )
    .await?;

    let mut new_job = crate::worker::NewJob::from(crate::worker::Job::ConvertDocument {
        job_id: job_id.clone(),
        tenant_id: tenant.to_string(),
        filename: filename.to_string(),
        engine: engine.map(str::to_string),
        extract_images,
    });
    new_job.timeout_secs = Some(crate::worker::LONG_JOB_TIMEOUT_SECS);
    queue
        .enqueue(new_job)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("enqueue convert job: {e}")))?;
    Ok(job_id)
}

/// 读 job 状态文档（GET 端点）。返回 (meta, 结果 markdown 可选)。
pub async fn read_status(
    storage: &Arc<dyn Storage>,
    job_id: &str,
) -> AppResult<Option<(serde_json::Value, Option<String>)>> {
    let raw = match storage.get(&meta_key(job_id)).await {
        Ok(raw) => raw,
        Err(_) => return Ok(None), // 无 meta = job 不存在
    };
    let mut meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta decode: {e}")))?;
    let mut markdown = None;
    if meta["status"] == "completed"
        && let Some(key) = meta["result_md_key"].as_str()
    {
        markdown = storage
            .get(key)
            .await
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
    }
    if let Some(url) = meta["result_md_url"].as_str() {
        meta["result_md_url"] = json!(url);
    }
    Ok(Some((meta, markdown)))
}

/// worker 执行体：running → 引擎解析 → 图片/结果落存储 → completed/failed。
/// `route` 语义：显式 engine 优先，缺省走全局默认引擎，均未命中回 builtin。
#[allow(clippy::too_many_arguments)]
pub async fn run(
    pool: &crate::db::Pool,
    storage: &Arc<dyn Storage>,
    parsers: &ParserRegistry,
    global_default: Option<&str>,
    job_id: &str,
    _tenant: &str,
    filename: &str,
    engine: Option<&str>,
    extract_images: bool,
) -> AppResult<()> {
    let started = std::time::Instant::now();
    set_status(storage, job_id, "running").await?;

    let result = convert_inner(
        storage,
        parsers,
        global_default,
        job_id,
        filename,
        engine,
        extract_images,
    )
    .await;

    match result {
        Ok(outcome) => {
            let _ = crate::docparse::logs::finish(
                pool,
                job_id.parse().unwrap_or(0),
                "completed",
                Some(&outcome.engine),
                outcome.pages.map(i64::from),
                Some(i64::try_from(outcome.markdown.chars().count()).unwrap_or(0)),
                Some(started.elapsed().as_millis() as i64),
                None,
            )
            .await;
            write_meta(
                storage,
                job_id,
                json!({
                    "status": "completed",
                    "filename": filename,
                    "engine": outcome.engine,
                    "pages": outcome.pages,
                    "warnings": outcome.warnings,
                    "images": outcome.images,
                    "result_md_key": format!("{}/result.md", storage_dir(job_id)),
                    "result_md_url": outcome.result_md_url,
                    "finished_at": crate::utils::tz::now_utc().to_rfc3339(),
                }),
            )
            .await?;
        }
        Err(e) => {
            let _ = crate::docparse::logs::finish(
                pool,
                job_id.parse().unwrap_or(0),
                "failed",
                None,
                None,
                None,
                Some(started.elapsed().as_millis() as i64),
                Some(&e.to_string()),
            )
            .await;
            write_meta(
                storage,
                job_id,
                json!({
                    "status": "failed",
                    "filename": filename,
                    "engine": engine,
                    "error": e.to_string(),
                    "finished_at": crate::utils::tz::now_utc().to_rfc3339(),
                }),
            )
            .await?;
            return Err(e);
        }
    }

    // M3 webhook：job 终态回调（best-effort，10s 超时，不阻塞结果本身）。
    let final_meta: serde_json::Value =
        serde_json::from_slice(&storage.get(&meta_key(job_id)).await.unwrap_or_default())
            .unwrap_or(serde_json::Value::Null);
    if let Some(cb) = final_meta["callback_url"].as_str() {
        crate::docparse::webhook::fire(
            cb,
            &json!({
                "job_id": job_id,
                "kind": "convert",
                "status": final_meta["status"],
                "result_md_url": final_meta["result_md_url"],
                "error": final_meta["error"],
            }),
        )
        .await;
    }
    Ok(())
}

async fn convert_inner(
    storage: &Arc<dyn Storage>,
    parsers: &ParserRegistry,
    global_default: Option<&str>,
    job_id: &str,
    filename: &str,
    engine: Option<&str>,
    extract_images: bool,
) -> AppResult<Converted> {
    let input = input_key(job_id, filename);
    let bytes = storage
        .get(&input)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read input: {e}")))?;

    // 引擎选择：显式 engine > 全局默认 > builtin（KB 无关，无 KB 规则层）。
    let requested = engine
        .or(global_default)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("builtin")
        .to_string();
    let mut warnings: Vec<String> = Vec::new();
    let eng = match parsers.get(&requested) {
        Some(e) if e.probe().await => e,
        _ => {
            warnings.push(format!(
                "engine '{requested}' unavailable → falling back to builtin"
            ));
            parsers
                .get("builtin")
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("builtin engine missing")))?
        }
    };
    let mime = mime_guess::from_path(filename).first_or_octet_stream();
    let opts = crate::docparse::ParseOpts { extract_images };
    let parsed = eng
        .parse(&bytes, mime.essence_str(), filename, &opts)
        .await?;

    // 图片落存储（挂 /uploads 挂载点 → 可直接访问），改写 markdown 引用。
    let mut stored_images: Vec<serde_json::Value> = Vec::new();
    let mut markdown = parsed.markdown;
    for img in &parsed.images {
        let key = format!(
            "{}/images/{}",
            storage_dir(job_id),
            img.ref_name.trim_start_matches("./")
        );
        storage.put(&key, &img.bytes, &img.mime_type).await?;
        let url = storage
            .url(&key)
            .await
            .unwrap_or_else(|_| format!("/{key}"));
        markdown = markdown.replace(&format!("]({})", img.ref_name), &format!("]({url})"));
        stored_images.push(json!({ "name": img.ref_name, "key": key, "url": url }));
    }

    let result_md_key = format!("{}/result.md", storage_dir(job_id));
    storage
        .put(&result_md_key, markdown.as_bytes(), "text/markdown")
        .await?;
    let result_md_url = storage.url(&result_md_key).await.unwrap_or_default();

    Ok(Converted {
        markdown,
        images: stored_images,
        result_md_url,
        engine: requested,
        pages: parsed.pages,
        warnings,
    })
}

pub struct Converted {
    pub markdown: String,
    pub images: Vec<serde_json::Value>,
    pub result_md_url: String,
    pub engine: String,
    pub pages: Option<u32>,
    pub warnings: Vec<String>,
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

async fn set_status(storage: &Arc<dyn Storage>, job_id: &str, status: &str) -> AppResult<()> {
    let raw = storage.get(&meta_key(job_id)).await?;
    let mut meta: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("meta decode: {e}")))?;
    meta["status"] = json!(status);
    write_meta(storage, job_id, meta).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docparse::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};

    struct FakeEngine;

    #[async_trait::async_trait]
    impl ParseEngine for FakeEngine {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn supports(&self, _mime: &str, _filename: &str) -> bool {
            true
        }
        async fn probe(&self) -> bool {
            true
        }
        async fn parse(
            &self,
            _bytes: &[u8],
            _mime: &str,
            _filename: &str,
            _opts: &ParseOpts,
        ) -> AppResult<ParseOutcome> {
            Ok(ParseOutcome {
                markdown: "# FAKE_TEST_OUTPUT\n\n![img](images/pic.png)".into(),
                images: vec![ParsedImage {
                    ref_name: "images/pic.png".into(),
                    mime_type: "image/png".into(),
                    bytes: b"PNGDATA".to_vec(),
                }],
                engine: "fake".into(),
                pages: Some(1),
                scanned_pages: Vec::new(),
            })
        }
    }

    fn deps_with_fake() -> (Arc<dyn Storage>, Arc<ParserRegistry>, Arc<dyn Storage>) {
        let storage: Arc<dyn Storage> = Arc::new(
            crate::storage::local::LocalStorage::new(
                "/tmp/kb-parse-service-test/uploads",
                "/uploads",
            )
            .unwrap(),
        );
        let parsers = Arc::new(ParserRegistry::new(vec![Arc::new(FakeEngine)]));
        (storage.clone(), parsers, storage.clone())
    }

    #[tokio::test]
    async fn conversion_job_lifecycle() {
        let (storage, parsers, storage_assert) = deps_with_fake();
        let pool = crate::test_pool!();
        let queue = crate::worker::DefaultJobQueue::new(pool.clone());
        let job_id = submit(
            &pool,
            &storage,
            &parsers,
            &queue,
            "default",
            "hello.txt",
            b"hello conversion",
            Some("fake"),
            true,
            None,
        )
        .await
        .unwrap();

        // 未执行前：queued
        let (meta, md) = read_status(&storage, &job_id).await.unwrap().unwrap();
        assert_eq!(meta["status"], "queued");
        assert!(md.is_none());

        // worker 执行体（直接调用，等价 ConvertDocumentHandler）
        run(
            &pool,
            &storage,
            &parsers,
            None,
            &job_id,
            "default",
            "hello.txt",
            Some("fake"),
            true,
        )
        .await
        .unwrap();

        let (meta, md) = read_status(&storage, &job_id).await.unwrap().unwrap();
        assert_eq!(meta["status"], "completed");
        assert_eq!(meta["engine"], "fake");
        let md = md.expect("completed job carries markdown");
        assert!(md.contains("FAKE_TEST_OUTPUT"));
        assert!(
            md.contains("/uploads/parse/") || md.contains("parse/"),
            "image ref must be rewritten: {md}"
        );
        // 图片确实落了存储（key 以 meta 记录为准）
        let img_key = meta["images"][0]["key"].as_str().unwrap().to_string();
        let img = storage_assert.get(&img_key).await.unwrap();
        assert_eq!(img, b"PNGDATA");
    }

    #[tokio::test]
    async fn unknown_engine_rejected() {
        let (storage, parsers, _sa) = deps_with_fake();
        let queue = crate::worker::DefaultJobQueue::new(crate::test_pool!().clone());
        let pool = crate::test_pool!();
        let err = submit(
            &pool,
            &storage,
            &parsers,
            &queue,
            "default",
            "x.txt",
            b"x",
            Some("nope"),
            true,
            None,
        )
        .await;
        assert!(err.is_err(), "unknown engine must be rejected");
    }
}
