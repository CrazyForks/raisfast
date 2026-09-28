//! material 节点族共享类型与多源分发（协议参照 MPT material.py）。
//!
//! 一源一文件：`pexels.rs` / `pixabay.rs` / `coverr.rs`（协议逐字段对照
//! MPT search_videos_*；新增素材源继续一源一文件）。编排语义 [照抄 MPT
//! 素材源分治]：本节点族属**库存源**（免费搜索，先取候选再挑选，可缓存
//! 复用）；付费按需生成源由 llm provider 扩展渠道承担（provider-plugins.md）
//! ——两类语义不同，不共享流程。

use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;

pub mod coverr;
pub mod pexels;
pub mod pixabay;

pub const T_MATERIAL: &str = "material";

/// 支持的库存素材源。
pub const PROVIDERS: &[&str] = &["pexels", "pixabay", "coverr"];

const PEXELS_BASE: &str = "https://api.pexels.com";
const PIXABAY_BASE: &str = "https://pixabay.com";
const COVERR_BASE: &str = "https://api.coverr.co";

#[derive(Debug, Clone, Deserialize)]
pub struct MaterialConfig {
    /// 库存素材源（pexels | pixabay | coverr）。
    pub provider: String,
    /// 搜索词，支持模板引用（`{{nodes.x.output}}`，经 render_prompt_text）。
    pub query: String,
    /// 素材源 API key（随 flows 保存——多租户下各租户各用各的配额）。
    pub api_key: String,
    #[serde(default = "default_orientation")]
    pub orientation: String, // portrait | landscape | square
    /// 最小素材时长（秒）——pexels/pixabay 过滤；coverr 照抄 MPT 不过滤。
    #[serde(default)]
    pub min_duration: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

fn default_orientation() -> String {
    "portrait".into()
}

fn default_per_page() -> u32 {
    20
}

/// 候选素材（输出 JSON 形状）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MaterialCandidate {
    pub provider: String,
    pub url: String,
    pub duration: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(rename = "assetId", skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(rename = "sourcePage", skip_serializing_if = "Option::is_none")]
    pub source_page: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 方向匹配 [近似照抄 MPT _matches_video_aspect]。
pub(crate) fn matches_orientation(w: u32, h: u32, orientation: &str) -> bool {
    match orientation {
        "landscape" => w > h,
        "portrait" => h > w,
        "square" => w == h,
        _ => true,
    }
}

/// 多源分发：按 provider 路由到对应 source 模块（每源 parse 各自独立，
/// 本函数只做 HTTP + 分发）。
pub async fn search_material(
    http: &reqwest::Client,
    provider: &str,
    api_key: &str,
    query: &str,
    orientation: &str,
    per_page: u32,
    min_duration: u32,
) -> AppResult<Vec<MaterialCandidate>> {
    match provider {
        "pexels" => {
            let url = format!(
                "{PEXELS_BASE}/v1/videos/search?query={}&per_page={per_page}&orientation={orientation}",
                urlencode(query),
            );
            let body = fetch_json(http, &url, &[("Authorization", api_key)]).await?;
            Ok(pexels::parse_response(&body, min_duration, orientation))
        }
        "pixabay" => {
            let url = format!(
                "{PIXABAY_BASE}/api/videos/?q={}&video_type=all&per_page=50&key={}",
                urlencode(query),
                urlencode(api_key),
            );
            let body = fetch_json(http, &url, &[]).await?;
            let min_width = match orientation {
                "portrait" => 1080,
                "landscape" => 1920,
                _ => 1080,
            };
            Ok(pixabay::parse_response(
                &body,
                min_duration,
                orientation,
                min_width,
            ))
        }
        "coverr" => {
            let filter = match orientation {
                "portrait" => "&filter=is_vertical:true",
                "landscape" => "&filter=is_vertical:false",
                _ => "",
            };
            let url = format!(
                "{COVERR_BASE}/videos?query={}&page_size={per_page}&urls=true&sort=popular{filter}",
                urlencode(query),
            );
            let body = fetch_json(
                http,
                &url,
                &[("Authorization", &format!("Bearer {api_key}"))],
            )
            .await?;
            Ok(coverr::parse_response(&body, orientation))
        }
        other => Err(AppError::BadRequest(format!(
            "material: unsupported provider {other:?} (supported: pexels, pixabay, coverr)"
        ))),
    }
}

async fn fetch_json(
    http: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> AppResult<Value> {
    let mut req = http.get(url).timeout(std::time::Duration::from_secs(60));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("material search: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("material search body: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "material search failed: HTTP {status}"
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("material search parse: {e}")))
}

/// material.search 节点执行（同步——搜索是快请求，无 park/poll）。
pub async fn run_material_search(node: &GraphNode, variables: &Pool) -> AppResult<ExecOutcome> {
    let cfg: MaterialConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("material config: {e}")))?;
    let query = super::render_prompt_text(&cfg.query, variables)?;
    if query.trim().is_empty() {
        return Err(AppError::BadRequest("material: query is empty".into()));
    }

    let http = reqwest::Client::new();
    let started = std::time::Instant::now();
    let videos = search_material(
        &http,
        &cfg.provider,
        &cfg.api_key,
        &query,
        &cfg.orientation,
        cfg.per_page.max(1),
        cfg.min_duration,
    )
    .await?;
    let count = videos.len();

    Ok(ExecOutcome {
        output: json!({ "videos": videos, "count": count, "query": query }),
        usage: None,
        latency_ms: Some(started.elapsed().as_millis() as i64),
    })
}

/// Config validation（节点校验，对照 video.rs validate 模式）。
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: MaterialConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'material' config invalid: {e}")))?;
    if !PROVIDERS.contains(&c.provider.as_str()) {
        return Err(AppError::BadRequest(format!(
            "material: unsupported provider {:?} (supported: {})",
            c.provider,
            PROVIDERS.join(", ")
        )));
    }
    if c.query.trim().is_empty() {
        return Err(AppError::BadRequest("material: query 不能为空".into()));
    }
    if c.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("material: api_key 不能为空".into()));
    }
    if !["portrait", "landscape", "square"].contains(&c.orientation.as_str()) {
        return Err(AppError::BadRequest(
            "material: orientation 须为 portrait/landscape/square".into(),
        ));
    }
    Ok(())
}
