//! publish 节点 —— 成品视频跨平台发布（Upload-Post API：TikTok / Instagram
//! / YouTube Shorts；协议照抄 MPT `app/services/upload_post.py`）。
//!
//! ⚠️ 发布是**不可逆的公网动作**：节点默认 `enabled: false`，编排上建议
//! 前置 await 审批节点（人工确认后再发布）。
//!
//! 协议要点：
//! - POST {base}/api/upload，`Authorization: Apikey <key>`（非 Bearer）；
//! - multipart/form-data：`video` 文件 + `user` / `title`(≤2200) /
//!   `privacy_level` / `platform[]`(重复) + YouTube 受众声明字段；
//! - 响应 `{success, request_id, message}`；状态查询
//!   GET /api/uploadposts/status?request_id=…。

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool, resolve};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

pub const T_PUBLISH: &str = "publish";

const DEFAULT_API_BASE: &str = "https://api.upload-post.com";
const TITLE_MAX: usize = 2200;
const YT_TITLE_MAX: usize = 100;
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, Deserialize)]
pub struct PublishConfig {
    /// 发布开关（默认关——公网发布不可逆，须显式开启）。
    #[serde(default)]
    pub enabled: bool,
    /// Upload-Post API key。
    pub api_key: String,
    /// Upload-Post 账号名。
    pub user: String,
    /// 发布目标平台（tiktok / instagram / youtube）。
    pub platforms: Vec<String>,
    /// 视频标题模板（≤2200 字符，支持 `{{nodes.x.output}}` 引用）。
    pub title: String,
    /// 成片文件引用 ValueExpr → `{key}`（storage 内部件）。
    pub video: Value,
    #[serde(default = "default_privacy_level")]
    pub privacy_level: String,
    #[serde(default)]
    pub youtube: Option<YouTubeExtra>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_api_base")]
    pub api_base: String,
}

fn default_privacy_level() -> String {
    "PUBLIC_TO_EVERYONE".into()
}

fn default_privacy_status() -> String {
    "public".into()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.into()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct YouTubeExtra {
    /// YouTube 受众声明（COPPA）：不配置视为「非面向儿童」。
    #[serde(default)]
    pub made_for_kids: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_privacy_status")]
    pub privacy_status: String,
}

impl YouTubeExtra {
    fn extra_fields(&self) -> Vec<(String, String)> {
        let mut out = vec![(
            "selfDeclaredMadeForKids".to_string(),
            self.made_for_kids.to_string(),
        )];
        if let Some(t) = &self.title {
            out.push((
                "youtube_title".to_string(),
                t.chars().take(YT_TITLE_MAX).collect(),
            ));
        }
        if let Some(d) = &self.description {
            out.push(("youtube_description".to_string(), d.clone()));
        }
        for tag in &self.tags {
            out.push(("tags[]".to_string(), tag.clone()));
        }
        out.push(("privacyStatus".to_string(), self.privacy_status.clone()));
        out.push(("containsSyntheticMedia".to_string(), "true".into()));
        out
    }
}

/// Config validation。
pub fn validate(config: &Value) -> AppResult<()> {
    let c: PublishConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'publish' config invalid: {e}")))?;
    if c.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("publish: api_key 不能为空".into()));
    }
    if c.user.trim().is_empty() {
        return Err(AppError::BadRequest("publish: user 不能为空".into()));
    }
    if c.platforms.is_empty() {
        return Err(AppError::BadRequest("publish: platforms 不能为空".into()));
    }
    if c.title.trim().is_empty() {
        return Err(AppError::BadRequest("publish: title 不能为空".into()));
    }
    if c.video.is_null() {
        return Err(AppError::BadRequest("publish: video 引用不能为空".into()));
    }
    if c.timeout_ms < 1 {
        return Err(AppError::BadRequest("publish: timeout_ms 须 ≥1".into()));
    }
    Ok(())
}

struct Resolved {
    key: Option<String>,
    url: Option<String>,
}

/// ValueExpr → `{key}` 或 `{url}`（storage 内部件 / https 直链）。
fn resolve_video_ref(expr: &Value, variables: &Pool) -> AppResult<Resolved> {
    let v = resolve(expr, variables)?;
    let key = v.get("key").and_then(Value::as_str).map(str::to_string);
    let url = v.get("url").and_then(Value::as_str).map(str::to_string);
    if key.is_none() && url.is_none() {
        return Err(AppError::BadRequest(
            "publish: video 引用解析结果不含 key/url".into(),
        ));
    }
    Ok(Resolved {
        key: key.filter(|k| !k.is_empty()),
        url: url.filter(|u| !u.is_empty()),
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn run_publish(
    node: &GraphNode,
    storage: &Arc<dyn Storage>,
    variables: &Pool,
) -> AppResult<ExecOutcome> {
    let started = std::time::Instant::now();
    let http_client = reqwest::Client::new();
    let cfg: PublishConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("publish config: {e}")))?;

    // 发布开关（默认关——不可逆动作）。
    if !cfg.enabled {
        return Ok(ExecOutcome {
            output: json!({ "posted": false, "skipped": "publish disabled" }),
            usage: None,
            latency_ms: Some(started.elapsed().as_millis() as i64),
        });
    }
    if cfg.platforms.is_empty() {
        return Err(AppError::BadRequest("publish: platforms 不能为空".into()));
    }
    let title_expr = Value::String(cfg.title.clone());
    let title_full = resolve(&title_expr, variables)?;
    let title = match &title_full {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let title: String = title.chars().take(TITLE_MAX).collect();
    if title.trim().is_empty() {
        return Err(AppError::BadRequest("publish: title 不能为空".into()));
    }

    // 成片引用解析：storage key → 读字节；https url → SSRF 守卫下载。
    let resolved = resolve_video_ref(&cfg.video, variables)?;
    let bytes = if let Some(key) = &resolved.key {
        storage.get(key).await?
    } else if let Some(url) = &resolved.url {
        super::download_https(url).await?
    } else {
        return Err(AppError::BadRequest(
            "publish: video 引用解析结果不含 key/url".into(),
        ));
    };
    if bytes.is_empty() {
        return Err(AppError::BadRequest("publish: 视频字节为空".into()));
    }

    // multipart 表单 [照抄 MPT upload_post.py]。
    let mut form = reqwest::multipart::Form::new()
        .text("user", cfg.user.clone())
        .text("title", title.clone())
        .text("privacy_level", cfg.privacy_level.clone());
    for platform in &cfg.platforms {
        form = form.part(
            "platform[]",
            reqwest::multipart::Part::text(platform.clone()),
        );
    }
    if let Some(yt) = &cfg.youtube {
        for (k, v) in yt.extra_fields() {
            form = form.part(k, reqwest::multipart::Part::text(v));
        }
    }
    let video_part = reqwest::multipart::Part::bytes(bytes)
        .file_name("publish.mp4")
        .mime_str("video/mp4")
        .map_err(|e| AppError::Internal(anyhow::anyhow!("publish mime: {e}")))?;
    form = form.part("video", video_part);

    let api_base = if cfg.api_base.is_empty() {
        DEFAULT_API_BASE.to_string()
    } else {
        cfg.api_base.clone()
    };
    let url = format!("{api_base}/api/upload");
    let resp = http_client
        .post(&url)
        .header("Authorization", format!("Apikey {}", cfg.api_key))
        .multipart(form)
        .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("publish upload: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("publish body: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "publish upload failed: HTTP {status}"
        )));
    }
    let parsed: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    let posted = parsed
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let request_id = parsed
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    Ok(ExecOutcome {
        output: json!({
            "posted": posted,
            "requestId": request_id,
            "platforms": cfg.platforms,
        }),
        usage: None,
        latency_ms: Some(started.elapsed().as_millis() as i64),
    })
}
