//! material 节点族共享类型与多源分发（协议参照 MPT material.py）。
//!
//! 一源一文件：`pexels.rs` / `pixabay.rs` / `coverr.rs`——每文件自带
//! `NAME` + `build_url` + `parse_response`，自包含全部协议逻辑；新增素材源
//! = 一个新文件 + `SOURCES` 表一行注册。
//!
//! 编排语义 [照抄 MPT 素材源分治]：本节点族属**库存源**（免费搜索，先取
//! 候选再挑选，可缓存复用）；付费按需生成源由 llm provider 扩展渠道承担
//！（provider-plugins.md）——两类语义不同，不共享流程。

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::ExecOutcome;
use crate::flows::graph::GraphNode;

pub mod coverr;
pub mod pexels;
pub mod pixabay;

pub use coverr as coverr_source;
pub use pexels as pexels_source;
pub use pixabay as pixabay_source;

pub const T_MATERIAL: &str = "material";

/// 构建请求 URL 与鉴权 header：(url, headers)。
pub type BuildUrlFn = fn(
    api_key: &str,
    query: &str,
    orientation: &str,
    per_page: u32,
) -> (String, Vec<(&'static str, String)>);
/// 解析响应 → 候选（源内自查方向/时长/宽度）。
pub type ParseFn = fn(&Value, u32, &str) -> Vec<MaterialCandidate>;

/// 素材源注册表：name + build_url + parse（新增源 = 一行注册）。
pub struct StockSource {
    pub name: &'static str,
    pub build_url: BuildUrlFn,
    pub parse: ParseFn,
}

pub const SOURCES: &[StockSource] = &[
    StockSource {
        name: "pexels",
        build_url: pexels::build_url,
        parse: pexels::parse_response,
    },
    StockSource {
        name: "pixabay",
        build_url: pixabay::build_url,
        parse: pixabay::parse_response,
    },
    StockSource {
        name: "coverr",
        build_url: coverr::build_url,
        parse: coverr::parse_response,
    },
];

/// 支持的素材源名（管理台提示/校验用）。
pub fn provider_names() -> Vec<&'static str> {
    SOURCES.iter().map(|s| s.name).collect()
}

pub(crate) fn urlencode(s: &str) -> String {
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

#[derive(Debug, Clone, Deserialize)]
pub struct MaterialConfig {
    /// 库存素材源（见 SOURCES）。
    pub provider: String,
    /// 搜索词，支持模板引用（`{{nodes.x.output}}`，经 render_prompt_text）。
    pub query: String,
    /// 素材源 API key（随 flows 保存——多租户下各租户各用各的配额）。
    pub api_key: String,
    #[serde(default = "default_orientation")]
    pub orientation: String, // portrait | landscape | square
    /// 最小素材时长（秒）——各源 parse 内自查（coverr 照抄 MPT 不过滤）。
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

/// 多源分发：查表 → build_url → fetch → parse（统一入口，节点/测试共用）。
pub async fn search_material(
    http: &reqwest::Client,
    provider: &str,
    api_key: &str,
    query: &str,
    orientation: &str,
    per_page: u32,
    min_duration: u32,
) -> AppResult<Vec<MaterialCandidate>> {
    let Some(src) = SOURCES.iter().find(|s| s.name == provider) else {
        return Err(AppError::BadRequest(format!(
            "material: unsupported provider {provider:?} (supported: {})",
            provider_names().join(", ")
        )));
    };
    let (url, headers) = (src.build_url)(api_key, query, orientation, per_page);
    let mut req = http.get(&url).timeout(std::time::Duration::from_secs(60));
    for (k, v) in &headers {
        req = req.header(*k, v.as_str());
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
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("material search parse: {e}")))?;
    Ok((src.parse)(&body, min_duration, orientation))
}

/// material.search 节点执行（同步——搜索是快请求，无 park/poll）。
pub async fn run_material_search(
    node: &GraphNode,
    variables: &HashMap<String, HashMap<String, Value>>,
) -> AppResult<ExecOutcome> {
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
    if !SOURCES.iter().any(|s| s.name == c.provider) {
        return Err(AppError::BadRequest(format!(
            "material: unsupported provider {:?} (supported: {})",
            c.provider,
            provider_names().join(", ")
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
