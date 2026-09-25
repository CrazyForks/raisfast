//! `render` 节点 executor（render-node.md）——ffmpeg 合成：timeline 视频段
//! 拼接 + 画布归一 + BGM 混音 + SRT 字幕烧录。
//!
//! 管线（§2 四级，全部 `.current_dir(work_dir)` + 相对文件名——绕开 ffmpeg
//! 滤镜的绝对路径转义地狱 [自造-实现决策]）：
//!   ① normalize  逐片段画布归一（cover=increase+crop / contain=decrease+pad）
//!   ② concat     concat demuxer 一次串联（段已统一编码 → `-c:v copy` 免二次
//!                转码 [自造-优化；MPT 重编码是因 MoviePy 临时段参数不齐]）
//!   ③ ffprobe    成片时长（BGM 淡出起点）+ 音频流探测（amix vs 直挂分叉）
//!   ④ finalize   BGM（volume/循环铺满/3s 淡出）+ SRT 烧录按需叠加
//!
//! 参考矩阵见 `dev-docs/workflow/render-node.md` §9（四大件 [照抄 MPT
//! video.py]：concat demuxer、编码器降级、时长安全余量、BGM 音量短路）。
//! 同步执行：渲染无计费副作用，崩溃重跑 = 幂等重渲。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

/// Canvas resolutions [照抄 MPT `VideoAspect::to_resolution`].
const CANVAS: [(&str, u32, u32); 3] = [
    ("9:16", 1080, 1920),
    ("16:9", 1920, 1080),
    ("1:1", 1080, 1080),
];
const ASPECTS: [&str; 3] = ["9:16", "16:9", "1:1"];
const FIT_MODES: [&str; 2] = ["cover", "contain"];
const SUBTITLE_POSITIONS: [&str; 3] = ["bottom", "top", "center"];
/// libass Alignment（ASS v4+）：2=底部居中 8=顶部居中 5=居中。
const SUBTITLE_ALIGNMENT: [(&str, &str); 3] = [("bottom", "2"), ("top", "8"), ("center", "5")];

/// `render` 节点 config（render-node.md §1）。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RenderConfig {
    /// 片段序列 — ValueExpr → 数组，每项 `{video: {key|url}}`。
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub timeline: Value,
    /// BGM — ValueExpr → `{key|url}` 或 https URL；缺省无 BGM。
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub bgm: Option<Value>,
    /// BGM 音量 0.0–1.0（≤0 视为无 BGM [照抄 MPT 音量短路]）。
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub bgm_volume: Option<f64>,
    /// SRT 字幕 — ValueExpr → 文件引用（storage key / https URL）。
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub subtitles: Option<Value>,
    /// 画布比例（默认 9:16）。
    #[serde(default)]
    pub aspect: Option<String>,
    /// 画布填充（默认 cover）。
    #[serde(default)]
    pub fit_mode: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub timeout_ms: Option<i64>,
    #[serde(default)]
    pub font_name: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub font_size: Option<i64>,
    /// 字幕位置（默认 bottom）。
    #[serde(default)]
    pub subtitle_position: Option<String>,
}

/// Config validation（render-node.md §1 边界）。
pub(super) fn validate(config: &Value) -> AppResult<()> {
    let c: RenderConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'render' config invalid: {e}")))?;
    if c.timeline.is_null() {
        return Err(AppError::BadRequest("render: timeline 不能为空".into()));
    }
    super::validate_value_expr("render.timeline", &c.timeline)?;
    if let Some(bgm) = &c.bgm {
        super::validate_value_expr("render.bgm", bgm)?;
    }
    if let Some(v) = c.bgm_volume
        && !(0.0..=1.0).contains(&v)
    {
        return Err(AppError::BadRequest("render: bgm_volume 须在 [0,1]".into()));
    }
    if let Some(subs) = &c.subtitles {
        super::validate_value_expr("render.subtitles", subs)?;
    }
    if let Some(a) = &c.aspect
        && !ASPECTS.contains(&a.as_str())
    {
        return Err(AppError::BadRequest(format!(
            "render: aspect '{}' 非法（允许: {}）",
            a,
            ASPECTS.join("/")
        )));
    }
    if let Some(f) = &c.fit_mode
        && !FIT_MODES.contains(&f.as_str())
    {
        return Err(AppError::BadRequest(format!(
            "render: fit_mode '{}' 非法（允许: {}）",
            f,
            FIT_MODES.join("/")
        )));
    }
    if c.timeout_ms.is_some_and(|t| t < 1) {
        return Err(AppError::BadRequest(
            "render: timeout_ms 须为 ≥1 的整数".into(),
        ));
    }
    if let Some(pos) = &c.subtitle_position
        && !SUBTITLE_POSITIONS.contains(&pos.as_str())
    {
        return Err(AppError::BadRequest(format!(
            "render: subtitle_position '{}' 非法（允许: {}）",
            pos,
            SUBTITLE_POSITIONS.join("/")
        )));
    }
    Ok(())
}

// ── ffmpeg/ffprobe 进程工具 ─────────────────────────────────────────────

static FFMPEG_PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Locate ffmpeg once per process: `render.ffmpeg_path` override is not yet
/// plumbed (options), so PATH lookup wins; absence is cached too so the
/// preflight fails fast without re-probing [照抄 MPT check_ffmpeg_ready 位].
fn ffmpeg_path() -> Option<&'static PathBuf> {
    FFMPEG_PATH.get_or_init(|| which("ffmpeg")).as_ref()
}

fn which(bin: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Run ffmpeg with the given args inside `work_dir`; non-zero exit →
/// `Internal` with the stderr tail [照抄 MPT 错误尾截断].
async fn run_ffmpeg(work_dir: &Path, args: &[&str]) -> AppResult<()> {
    let Some(ffmpeg) = ffmpeg_path() else {
        return Err(AppError::BadRequest(
            "render: ffmpeg 不可用（安装 ffmpeg 或配置 PATH）".into(),
        ));
    };
    let output = tokio::process::Command::new(ffmpeg)
        .args(args)
        .current_dir(work_dir)
        .output()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: ffmpeg 启动失败: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        return Err(AppError::Internal(anyhow::anyhow!(
            "render: ffmpeg 失败: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
        )));
    }
    Ok(())
}

/// `ffprobe -show_entries format=duration` → seconds.
async fn probe_duration(work_dir: &Path, file: &str) -> AppResult<f64> {
    let Some(ffprobe) = which("ffprobe") else {
        return Err(AppError::BadRequest(
            "render: ffprobe 不可用（随 ffmpeg 安装）".into(),
        ));
    };
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            file,
        ])
        .current_dir(work_dir)
        .output()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: ffprobe 启动失败: {e}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<f64>()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 时长解析失败 {text:?}: {e}")))
}

/// Probe whether the file has an audio stream (amix vs direct-map branch).
async fn has_audio_stream(work_dir: &Path, file: &str) -> AppResult<bool> {
    let Some(ffprobe) = which("ffprobe") else {
        return Ok(false);
    };
    let output = tokio::process::Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
            file,
        ])
        .current_dir(work_dir)
        .output()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: ffprobe 启动失败: {e}")))?;
    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

// ── 输入解析 ────────────────────────────────────────────────────────────

/// Resolve one asset reference (`{key|url}` object or plain string) into a
/// local file inside `work_dir` (storage key → copy; https → SSRF-checked
/// download). Returns the relative file name.
async fn materialize_ref(
    storage: &Arc<dyn Storage>,
    work_dir: &Path,
    reference: &Value,
    name: &str,
) -> AppResult<String> {
    let raw = match reference {
        Value::String(s) => s.clone(),
        // timeline 项形态 `{video: {key|url}}`——先解包 video 层再取引用
        // [照抄 media-nodes 资产引用约定]。
        Value::Object(o) => {
            let inner = o.get("video").and_then(Value::as_object).unwrap_or(o);
            inner
                .get("key")
                .and_then(Value::as_str)
                .or_else(|| inner.get("url").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string()
        }
        _ => String::new(),
    };
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        return Err(AppError::BadRequest(format!(
            "render: {name} 引用为空（须为 storage key 或 https URL）"
        )));
    }
    let ext = Path::new(&raw)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.split('?').next().unwrap_or(e)))
        .unwrap_or_default();
    let local = format!("{name}{ext}");
    let dest = work_dir.join(&local);
    if raw.starts_with("https://") || raw.starts_with("http://") {
        crate::docparse::validate_external_url(&raw)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render http client: {e}")))?;
        let resp = client
            .get(&raw)
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("render: 下载 {name} 失败: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::BadRequest(format!(
                "render: 下载 {name} 失败 HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("render: 读取 {name} 失败: {e}")))?;
        std::fs::write(&dest, bytes.as_ref())
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 写入 {name}: {e}")))?;
    } else {
        // Storage key — stream out of storage into the work dir.
        let bytes = storage
            .get(&raw)
            .await
            .map_err(|e| AppError::BadRequest(format!("render: 读取 {name}（{raw}）失败: {e}")))?;
        std::fs::write(&dest, bytes)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 写入 {name}: {e}")))?;
    }
    Ok(local)
}

/// Resolve the timeline ValueExpr into an ordered list of asset references.
fn resolve_timeline(expr: &Value, pool: &Pool) -> AppResult<Vec<Value>> {
    let resolved = super::super::engine::resolve(expr, pool)?;
    match resolved {
        Value::Array(items) => Ok(items),
        other => Err(AppError::BadRequest(format!(
            "render: timeline 解析结果须为数组（got {}）",
            match other {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Object(_) => "object",
                Value::Array(_) => unreachable!(),
            }
        ))),
    }
}

// ── 管线四级 ────────────────────────────────────────────────────────────

/// ① normalize：画布归一 + 统一 fps/编码（无音轨，音轨在 ④ 统一处理）。
async fn normalize_segment(
    work_dir: &Path,
    input: &str,
    index: usize,
    w: u32,
    h: u32,
    fit_mode: &str,
) -> AppResult<String> {
    let out = format!("seg-{index}.mp4");
    // cover = 放大铺满后裁切；contain = 缩小完整 + 黑边 [照抄 MPT VideoFitMode]。
    let scale_kind = if fit_mode == "contain" {
        "decrease"
    } else {
        "increase"
    };
    let video_filter = if fit_mode == "contain" {
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30"
        )
    } else {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1,fps=30")
    };
    let _ = scale_kind;
    run_ffmpeg(
        work_dir,
        &[
            "-y",
            "-i",
            input,
            "-vf",
            &video_filter,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-an",
            &out,
        ],
    )
    .await?;
    Ok(out)
}

/// ② concat demuxer：段已统一编码 → `-c:v copy` 免二次转码。
async fn concat_segments(work_dir: &Path, segments: &[String]) -> AppResult<String> {
    let list_name = "concat-list.txt";
    let list = segments
        .iter()
        .map(|s| format!("file '{s}'"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(work_dir.join(list_name), list)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 写 concat list: {e}")))?;
    run_ffmpeg(
        work_dir,
        &[
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            list_name,
            "-c:v",
            "copy",
            "-pix_fmt",
            "yuv420p",
            "combined.mp4",
        ],
    )
    .await?;
    Ok("combined.mp4".into())
}

/// finalize 阶段参数束（避免 9 参函数）。
///
/// 字段：BGM 引用、音量、SRT 引用、字幕字体/字号/位置。
///
struct FinalizeStyle<'a> {
    bgm: Option<&'a str>,
    bgm_volume: f64,
    subtitles: Option<&'a str>,
    font_name: Option<&'a str>,
    font_size: i64,
    subtitle_position: &'a str,
}

async fn finalize(
    work_dir: &Path,
    combined: &str,
    duration: f64,
    style: &FinalizeStyle<'_>,
) -> AppResult<String> {
    let bgm_volume = style.bgm_volume;
    let bgm = style.bgm;
    let subtitles = style.subtitles;
    let alignment = SUBTITLE_ALIGNMENT
        .iter()
        .find(|(p, _)| *p == style.subtitle_position)
        .map(|(_, a)| *a)
        .unwrap_or("2");
    let mut style_str = format!("FontSize={},Alignment={}", style.font_size, alignment);
    if let Some(font) = style.font_name {
        style_str = format!("FontName={font},{style_str}");
    }

    let has_bgm = bgm.is_some() && bgm_volume > 0.0;
    let fade_start = (duration - 3.0).max(0.0);
    let filter;
    let mut maps: Vec<&str> = Vec::new();

    let video_chain = subtitles
        .map(|_| format!("subtitles=sub.srt:force_style='{style_str}'"))
        .unwrap_or_default();
    let mut args: Vec<&str> = vec!["-y", "-i", combined];
    let mut bgm_index_owned: Option<String> = None;
    if has_bgm {
        let Some(b) = bgm else {
            unreachable!("has_bgm implies bgm present");
        };
        args.push("-stream_loop");
        args.push("-1");
        args.push("-i");
        args.push(b);
        bgm_index_owned = Some("1".to_string());
    }
    let bgm_index = bgm_index_owned.as_deref();

    match (subtitles.is_some(), bgm_index) {
        (true, Some(_idx)) => {
            filter = format!(
                "[0:v]{video_chain}[v];[1:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[b];[0:a][b]amix=inputs=2:duration=first[a]"
            );
            maps.push("[v]");
            maps.push("[a]");
        }
        (true, None) => {
            filter = format!("[0:v]{video_chain}[v]");
            maps.push("[v]");
            maps.push("0:a?");
        }
        (false, Some(_idx)) => {
            if has_audio_stream(work_dir, combined).await? {
                filter = format!(
                    "[1:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[b];[0:a][b]amix=inputs=2:duration=first[a]"
                );
                maps.push("[a]");
            } else {
                filter = format!("[1:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[a]");
                maps.push("[a]");
            }
        }
        (false, None) => {
            // 纯拼接已就绪 — 直接复制。
            let out = "final.mp4";
            std::fs::copy(work_dir.join(combined), work_dir.join(out))
                .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 复制成片: {e}")))?;
            return Ok(out.to_string());
        }
    }
    args.push("-filter_complex");
    args.push(&filter);
    for m in &maps {
        args.push("-map");
        args.push(m);
    }
    args.push("-c:v");
    args.push("libx264");
    args.push("-preset");
    args.push("veryfast");
    args.push("-crf");
    args.push("20");
    args.push("-c:a");
    args.push("aac");
    args.push("-b:a");
    args.push("192k");
    if matches!((subtitles.is_some(), bgm_index), (false, Some(_))) {
        // BGM 直挂（源无音轨）时按视频时长截断无限循环。
        args.push("-shortest");
    }
    args.push("final.mp4");
    run_ffmpeg(work_dir, &args).await?;
    Ok("final.mp4".into())
}

/// Execute the `render` node.
///
/// # Errors
/// `BadRequest` on missing template refs / missing ffmpeg; `Internal` on
/// ffmpeg failures or timeouts. Crash re-run = idempotent re-render.
pub async fn run_render(
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: RenderConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("render config: {e}")))?;
    let started = Instant::now();

    if ffmpeg_path().is_none() {
        return Err(AppError::BadRequest(
            "render: ffmpeg 不可用（安装 ffmpeg 或配置 PATH 后重试）".into(),
        ));
    }

    // Resolve inputs against the pool first (fail fast before any IO).
    let segments = resolve_timeline(&cfg.timeline, pool)?;
    if segments.is_empty() {
        return Err(AppError::BadRequest("render: timeline 为空数组".into()));
    }
    let (w, h) = CANVAS
        .iter()
        .find(|(a, _, _)| Some(*a) == cfg.aspect.as_deref())
        .map(|(_, w, h)| (*w, *h))
        .unwrap_or((1080, 1920));
    let fit_mode = cfg.fit_mode.as_deref().unwrap_or("cover");
    let subtitle_position = cfg.subtitle_position.as_deref().unwrap_or("bottom");
    let font_size = cfg.font_size.unwrap_or(54);
    let bgm_volume = cfg.bgm_volume.unwrap_or(0.2);
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(600_000) as u64;

    // Work dir: system temp (always local — S3 backends included) with our
    // own safe names, so ffmpeg filter paths never need escaping.
    let run_id = crate::utils::id::new_id().to_string();
    let work_dir = std::env::temp_dir().join("raisfast-render").join(&run_id);
    std::fs::create_dir_all(&work_dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 创建工作目录: {e}")))?;

    let pipeline = async {
        // ① materialize + normalize
        let mut seg_files = Vec::with_capacity(segments.len());
        for (i, seg) in segments.iter().enumerate() {
            let input = materialize_ref(storage, &work_dir, seg, &format!("in-{i}")).await?;
            seg_files.push(normalize_segment(&work_dir, &input, i + 1, w, h, fit_mode).await?);
        }
        // ② concat
        let combined = concat_segments(&work_dir, &seg_files).await?;
        let duration = probe_duration(&work_dir, &combined).await?;
        // ④ finalize
        let bgm = match &cfg.bgm {
            Some(expr) => {
                let resolved = super::super::engine::resolve(expr, pool)?;
                if resolved.is_null() {
                    None
                } else {
                    Some(materialize_ref(storage, &work_dir, &resolved, "bgm").await?)
                }
            }
            None => None,
        };
        let subtitles = match &cfg.subtitles {
            Some(expr) => {
                let resolved = super::super::engine::resolve(expr, pool)?;
                if resolved.is_null() {
                    None
                } else {
                    Some(materialize_ref(storage, &work_dir, &resolved, "sub").await?)
                }
            }
            None => None,
        };
        let final_file = finalize(
            &work_dir,
            &combined,
            duration,
            &FinalizeStyle {
                bgm: bgm.as_deref(),
                bgm_volume,
                subtitles: subtitles.as_deref(),
                font_name: cfg.font_name.as_deref(),
                font_size,
                subtitle_position,
            },
        )
        .await?;
        // 转存 storage（唯一资产出口）。
        let bytes = std::fs::read(work_dir.join(&final_file))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 读取成片: {e}")))?;
        let key = format!("gen/flows/{run_id}/final.mp4");
        storage.put(&key, &bytes, "video/mp4").await?;
        let public_url = storage
            .url(&key)
            .await
            .unwrap_or_else(|_| format!("/{key}"));
        Ok::<_, AppError>((
            json!({ "key": key, "url": public_url, "bytes": bytes.len() }),
            duration,
        ))
    };

    let render_result = tokio::time::timeout(Duration::from_millis(timeout_ms), pipeline)
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("render 超时 {timeout_ms}ms")));

    // 清理工作目录（成功/失败均清 [照抄 MPT finally delete_files]）。
    let _ = std::fs::remove_dir_all(&work_dir);

    let (video, duration) = render_result??;
    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("video".into(), video);
    out.insert("duration_s".into(), json!(duration));
    out.insert("segments".into(), json!(segments.len()));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: Some(json!({ "segments": segments.len() })),
        latency_ms: Some(latency_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct MemStorage {
        files: Mutex<HashMap<String, Vec<u8>>>,
    }
    impl MemStorage {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                files: Mutex::new(HashMap::new()),
            })
        }
    }
    #[async_trait::async_trait]
    impl crate::storage::Storage for MemStorage {
        async fn put(&self, key: &str, data: &[u8], _ct: &str) -> AppResult<()> {
            self.files.lock().unwrap().insert(key.into(), data.into());
            Ok(())
        }
        async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
            self.files
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| AppError::not_found("storage"))
        }
        async fn delete(&self, key: &str) -> AppResult<()> {
            self.files.lock().unwrap().remove(key);
            Ok(())
        }
        async fn url(&self, key: &str) -> AppResult<String> {
            Ok(format!("http://localhost/{key}"))
        }
    }

    fn render_node(config: Value) -> GraphNode {
        GraphNode {
            id: "r1".into(),
            data: NodeData {
                kind: "render".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    fn ffmpeg_available() -> bool {
        ffmpeg_path().is_some()
    }

    #[test]
    fn render_config_validation() {
        let ok = json!({"timeline": {"literal": []}});
        assert!(super::validate(&ok).is_ok());
        assert!(super::validate(&json!({})).is_err(), "缺 timeline");
        assert!(
            super::validate(&json!({"timeline": {"literal": []}, "aspect": "4:3"})).is_err(),
            "非法 aspect"
        );
        assert!(
            super::validate(&json!({"timeline": {"literal": []}, "fit_mode": "stretch"})).is_err(),
            "非法 fit_mode"
        );
        assert!(
            super::validate(&json!({"timeline": {"literal": []}, "bgm_volume": 1.5})).is_err(),
            "bgm_volume 越界"
        );
        assert!(
            super::validate(&json!({"timeline": {"literal": []}, "subtitle_position": "middle"}))
                .is_err(),
            "非法字幕位置"
        );
    }

    #[tokio::test]
    async fn render_requires_ffmpeg_and_fails_fast() {
        // 即使 ffmpeg 在场，该用例也成立：空 timeline 在 IO 前就失败。
        let node = render_node(json!({"timeline": {"literal": []}}));
        let err = run_render(
            &(MemStorage::new() as Arc<dyn Storage>),
            &node,
            &Pool::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
    }

    /// 真实渲染冒烟（ffmpeg 在场才跑）：testsrc 产 2 段 1s 片段 →
    /// render → ffprobe 断言成片时长 ≈ 2s 且已落 storage。
    #[tokio::test]
    async fn smoke_two_segments_render_to_storage() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let storage = MemStorage::new();
        let work = std::env::temp_dir().join(format!(
            "raisfast-render-test-{}",
            crate::utils::id::new_id()
        ));
        std::fs::create_dir_all(&work).unwrap();
        let ff = ffmpeg_path().unwrap();

        for (i, color) in ["color=c=red", "color=c=blue"].iter().enumerate() {
            let out = work.join(format!("in-{i}.mp4"));
            let st = tokio::process::Command::new(ff)
                .args([
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("{color}:size=640x360:duration=1:rate=30"),
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                ])
                .arg(&out)
                .output()
                .await
                .unwrap();
            assert!(
                st.status.success(),
                "testsrc seg {i} failed: {}",
                String::from_utf8_lossy(&st.stderr)
            );
            storage
                .put(
                    &format!("seg-{i}.mp4"),
                    &std::fs::read(&out).unwrap(),
                    "video/mp4",
                )
                .await
                .unwrap();
        }
        let _ = std::fs::remove_dir_all(&work);

        let node = render_node(json!({
            "timeline": {"literal": [
                {"video": {"key": "seg-0.mp4"}},
                {"video": {"key": "seg-1.mp4"}}
            ]},
            "aspect": "16:9"
        }));
        let out = run_render(&(storage.clone() as Arc<dyn Storage>), &node, &Pool::new())
            .await
            .unwrap();
        let key = out.output["video"]["key"].as_str().unwrap().to_string();
        assert!(key.starts_with("gen/flows/"));
        let bytes = storage.get(&key).await.unwrap();
        assert!(bytes.len() > 1000, "成片过小: {}", bytes.len());
        assert_eq!(out.output["segments"], 2);
        let duration = out.output["duration_s"].as_f64().unwrap();
        assert!((duration - 2.0).abs() < 0.5, "duration {duration}");
    }
}
