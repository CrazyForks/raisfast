//! `render` 节点族共享管线（render-node.md）——ffmpeg/ffprobe 工具、画布表、
//! 归一/串联/合成四级管线、资产落地。一节点一文件：`video.rs`（视频段时间线），
//! P1 增 `image.rs`（图片段/推文形态）。

pub mod image;
pub mod video;

pub(crate) use super::validate_value_expr;
pub use video::RenderVideoConfig;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::Pool;
use crate::storage::Storage;

/// 画布分辨率表 [照抄 MPT `VideoAspect::to_resolution`]。
pub(crate) const CANVAS: [(&str, u32, u32); 3] = [
    ("9:16", 1080, 1920),
    ("16:9", 1920, 1080),
    ("1:1", 1080, 1080),
];
pub(crate) const ASPECTS: [&str; 3] = ["9:16", "16:9", "1:1"];
pub(crate) const FIT_MODES: [&str; 2] = ["cover", "contain"];
pub(crate) const SUBTITLE_POSITIONS: [&str; 3] = ["bottom", "top", "center"];
/// libass Alignment（ASS v4+）：2=底部居中 8=顶部居中 5=居中。
pub(crate) const SUBTITLE_ALIGNMENT: [(&str, &str); 3] =
    [("bottom", "2"), ("top", "8"), ("center", "5")];

/// 各 render 节点共享的合成参数（BGM/字幕/画布/字体）。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RenderCommon {
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

impl RenderCommon {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if let Some(bgm) = &self.bgm {
            super::validate_value_expr("render.bgm", bgm)?;
        }
        if let Some(v) = self.bgm_volume
            && !(0.0..=1.0).contains(&v)
        {
            return Err(AppError::BadRequest("render: bgm_volume 须在 [0,1]".into()));
        }
        if let Some(subs) = &self.subtitles {
            super::validate_value_expr("render.subtitles", subs)?;
        }
        if let Some(a) = &self.aspect
            && !ASPECTS.contains(&a.as_str())
        {
            return Err(AppError::BadRequest(format!(
                "render: aspect '{a}' 非法（允许: {}）",
                ASPECTS.join("/")
            )));
        }
        if let Some(f) = &self.fit_mode
            && !FIT_MODES.contains(&f.as_str())
        {
            return Err(AppError::BadRequest(format!(
                "render: fit_mode '{f}' 非法（允许: {}）",
                FIT_MODES.join("/")
            )));
        }
        if self.timeout_ms.is_some_and(|t| t < 1) {
            return Err(AppError::BadRequest(
                "render: timeout_ms 须为 ≥1 的整数".into(),
            ));
        }
        if let Some(pos) = &self.subtitle_position
            && !SUBTITLE_POSITIONS.contains(&pos.as_str())
        {
            return Err(AppError::BadRequest(format!(
                "render: subtitle_position '{pos}' 非法（允许: {}）",
                SUBTITLE_POSITIONS.join("/")
            )));
        }
        Ok(())
    }

    pub(crate) fn canvas(&self) -> (u32, u32) {
        CANVAS
            .iter()
            .find(|(a, _, _)| Some(*a) == self.aspect.as_deref())
            .map(|(_, w, h)| (*w, *h))
            .unwrap_or((1080, 1920))
    }

    pub(crate) fn fit_mode(&self) -> &str {
        self.fit_mode.as_deref().unwrap_or("cover")
    }

    pub(crate) fn subtitle_position(&self) -> &str {
        self.subtitle_position.as_deref().unwrap_or("bottom")
    }

    pub(crate) fn font_size(&self) -> i64 {
        self.font_size.unwrap_or(54)
    }

    pub(crate) fn timeout_ms(&self) -> u64 {
        (self.timeout_ms.filter(|t| *t > 0).unwrap_or(600_000)) as u64
    }
}

// ── ffmpeg/ffprobe 进程工具 ─────────────────────────────────────────────

static FFMPEG_PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Locate ffmpeg once per process (PATH lookup; absence cached so preflight
/// fails fast) [照抄 MPT check_ffmpeg_ready 位].
pub(crate) fn ffmpeg_path() -> Option<&'static PathBuf> {
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
pub(crate) async fn run_ffmpeg(work_dir: &Path, args: &[&str]) -> AppResult<()> {
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
pub(crate) async fn probe_duration(work_dir: &Path, file: &str) -> AppResult<f64> {
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
pub(crate) async fn has_audio_stream(work_dir: &Path, file: &str) -> AppResult<bool> {
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
pub(crate) async fn materialize_ref(
    storage: &Arc<dyn Storage>,
    work_dir: &Path,
    reference: &Value,
    name: &str,
) -> AppResult<String> {
    let raw = match reference {
        Value::String(s) => s.clone(),
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
            .timeout(std::time::Duration::from_secs(120))
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
        // Storage key — copy out of storage into the work dir.
        let bytes = storage
            .get(&raw)
            .await
            .map_err(|e| AppError::BadRequest(format!("render: 读取 {name}（{raw}）失败: {e}")))?;
        std::fs::write(&dest, bytes)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 写入 {name}: {e}")))?;
    }
    Ok(local)
}

/// Resolve the ValueExpr into an ordered list of segment references.
pub(crate) fn resolve_timeline(expr: &Value, pool: &Pool) -> AppResult<Vec<Value>> {
    let resolved = crate::flows::engine::resolve(expr, pool)?;
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

// ── 管线四级（共享）────────────────────────────────────────────────────

/// ① normalize：画布归一 + 统一 fps/编码。
pub(crate) async fn normalize_segment(
    work_dir: &Path,
    input: &str,
    index: usize,
    w: u32,
    h: u32,
    fit_mode: &str,
) -> AppResult<String> {
    let out = format!("seg-{index}.mp4");
    let video_filter = if fit_mode == "contain" {
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30"
        )
    } else {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1,fps=30")
    };
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
pub(crate) async fn concat_segments(work_dir: &Path, segments: &[String]) -> AppResult<String> {
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

/// finalize 参数束（避免 9 参函数）。
///
/// 字段：BGM 引用、音量、SRT 引用、字幕字体/字号/位置。
pub(crate) struct FinalizeStyle<'a> {
    pub(crate) narration: Option<&'a str>,
    pub(crate) bgm: Option<&'a str>,
    pub(crate) bgm_volume: f64,
    pub(crate) subtitles: Option<&'a str>,
    pub(crate) font_name: Option<&'a str>,
    pub(crate) font_size: i64,
    pub(crate) subtitle_position: &'a str,
}

/// ④ finalize：BGM（音量/循环铺满/3s 淡出 [照抄 MPT AudioFadeOut(3)/AudioLoop]）
/// + SRT 烧录，一条命令按需叠加。
pub(crate) async fn finalize(
    work_dir: &Path,
    combined: &str,
    duration: f64,
    style: &FinalizeStyle<'_>,
) -> AppResult<String> {
    let bgm_volume = style.bgm_volume;
    let bgm = style.bgm;
    let narration = style.narration;
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

    // 纯拼接、无任何叠加 → 直接复制成片。
    if narration.is_none() && !has_bgm && subtitles.is_none() {
        let out = "final.mp4";
        std::fs::copy(work_dir.join(combined), work_dir.join(out))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 复制成片: {e}")))?;
        return Ok(out.to_string());
    }

    // 输入装配：combined 恒为 0；旁白=1；BGM=2（循环铺满）。
    let mut args: Vec<&str> = vec!["-y", "-i", combined];
    if let Some(n) = narration {
        args.push("-i");
        args.push(n);
    }
    if has_bgm {
        args.push("-stream_loop");
        args.push("-1");
        args.push("-i");
        args.push(bgm.unwrap());
    }
    let narration_idx = narration.as_ref().map(|_| "1").unwrap_or("1");
    let bgm_idx = if has_bgm {
        if narration.is_some() { "2" } else { "1" }
    } else {
        "1"
    };

    // 统一 filter_complex：视频字幕链 + 音频混音链。
    let mut filter = String::new();
    if subtitles.is_some() {
        filter.push_str(&format!(
            "[0:v]subtitles=sub.srt:force_style='{style_str}'[v];"
        ));
    }

    let has_combined_audio = has_audio_stream(work_dir, combined).await?;
    match (narration, has_bgm) {
        (Some(_), true) => {
            filter.push_str(&format!(
                "[{narration_idx}:a]anull[n];[{bgm_idx}:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[b];[n][b]amix=inputs=2:duration=first[a]"
            ));
        }
        (Some(_), false) => {
            filter.push_str(&format!("[{narration_idx}:a]anull[a]"));
        }
        (None, true) => {
            if has_combined_audio {
                filter.push_str(&format!(
                    "[{bgm_idx}:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[b];[0:a][b]amix=inputs=2:duration=first[a]"
                ));
            } else {
                filter.push_str(&format!(
                    "[{bgm_idx}:a]volume={bgm_volume},afade=t=out:st={fade_start:.3}:d=3[a]"
                ));
            }
        }
        (None, false) => {}
    }

    args.push("-filter_complex");
    args.push(&filter);
    if subtitles.is_some() {
        args.push("-map");
        args.push("[v]");
    }
    if narration.is_some() || has_bgm {
        args.push("-map");
        args.push("[a]");
    } else if has_audio_stream(work_dir, combined).await.unwrap_or(false) {
        // 无叠加但有源音轨：透传。
        args.push("-map");
        args.push("0:a?");
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
    args.push("final.mp4");
    run_ffmpeg(work_dir, &args).await?;
    Ok("final.mp4".into())
}

/// 工作目录：系统临时目录（S3 后端也有本地盘）+ 安全相对名。
pub(crate) fn new_work_dir(run_id: &str) -> AppResult<PathBuf> {
    let work_dir = std::env::temp_dir().join("raisfast-render").join(run_id);
    std::fs::create_dir_all(&work_dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render: 创建工作目录: {e}")))?;
    Ok(work_dir)
}

pub(crate) fn cleanup_work_dir(work_dir: &Path) {
    let _ = std::fs::remove_dir_all(work_dir);
}
