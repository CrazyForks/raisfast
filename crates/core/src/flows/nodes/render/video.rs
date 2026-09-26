//! `render_video` 节点 executor（render-node.md）——视频段时间线合成：归一
//! + 拼接 + BGM 混音 + SRT 字幕烧录。共享管线在 `render` 目录的 mod.rs。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::RenderCommon;

/// `render_video` 节点 config（render-node.md §1）——timeline 为视频段；
/// 合成参数（BGM/字幕/画布/字体）在共享的 [`RenderCommon`]。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RenderVideoConfig {
    /// 片段序列 — ValueExpr，解析为数组，每项 `{video: {key|url}}`。
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub timeline: Value,
    #[serde(flatten)]
    pub common: RenderCommon,
}

/// Config validation（render-node.md §1 边界）。
pub(crate) fn validate(config: &Value) -> AppResult<()> {
    let c: RenderVideoConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'render_video' config invalid: {e}")))?;
    if c.timeline.is_null() {
        return Err(AppError::BadRequest(
            "render_video: timeline 不能为空".into(),
        ));
    }
    super::validate_value_expr("render.timeline", &c.timeline)?;
    c.common.validate()?;
    Ok(())
}

/// Execute the `render_video` node.
///
/// # Errors
/// `BadRequest` on missing template refs / missing ffmpeg; `Internal` on
/// ffmpeg failures or timeouts. Crash re-run = idempotent re-render.
pub async fn run_render_video(
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: RenderVideoConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("render_video config: {e}")))?;
    let started = Instant::now();

    if super::ffmpeg_path().is_none() {
        return Err(AppError::BadRequest(
            "render_video: ffmpeg 不可用（安装 ffmpeg 或配置 PATH 后重试）".into(),
        ));
    }

    let segments = super::resolve_timeline(&cfg.timeline, pool)?;
    if segments.is_empty() {
        return Err(AppError::BadRequest(
            "render_video: timeline 为空数组".into(),
        ));
    }
    let (w, h) = cfg.common.canvas();
    let fit_mode = cfg.common.fit_mode();
    let subtitle_position = cfg.common.subtitle_position();
    let bgm_volume = cfg.common.bgm_volume.unwrap_or(0.2);
    let timeout_ms = cfg.common.timeout_ms();

    let run_id = crate::utils::id::new_id().to_string();
    let work_dir = super::new_work_dir(&run_id)?;

    let pipeline = async {
        // ① materialize + normalize
        let mut seg_files = Vec::with_capacity(segments.len());
        for (i, seg) in segments.iter().enumerate() {
            let input = super::materialize_ref(storage, &work_dir, seg, &format!("in-{i}")).await?;
            seg_files
                .push(super::normalize_segment(&work_dir, &input, i + 1, w, h, fit_mode).await?);
        }
        // ② concat（段已统一编码 → copy）
        let combined = super::concat_segments(&work_dir, &seg_files).await?;
        let duration = super::probe_duration(&work_dir, &combined).await?;
        // ④ finalize
        let bgm = match &cfg.common.bgm {
            Some(expr) => {
                let resolved = crate::flows::engine::resolve(expr, pool)?;
                if resolved.is_null() {
                    None
                } else {
                    Some(super::materialize_ref(storage, &work_dir, &resolved, "bgm").await?)
                }
            }
            None => None,
        };
        let subtitles = match &cfg.common.subtitles {
            Some(expr) => {
                let resolved = crate::flows::engine::resolve(expr, pool)?;
                if resolved.is_null() {
                    None
                } else {
                    Some(super::materialize_ref(storage, &work_dir, &resolved, "sub").await?)
                }
            }
            None => None,
        };
        let final_file = super::finalize(
            &work_dir,
            &combined,
            duration,
            &super::FinalizeStyle {
                narration: None,
                bgm: bgm.as_deref(),
                bgm_volume,
                subtitles: subtitles.as_deref(),
                font_name: cfg.common.font_name.as_deref(),
                font_size: cfg.common.font_size(),
                subtitle_position,
            },
        )
        .await?;
        let bytes = std::fs::read(work_dir.join(&final_file))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render_video: 读取成片: {e}")))?;
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
        .map_err(|_| AppError::Internal(anyhow::anyhow!("render_video 超时 {timeout_ms}ms")));

    // 清理工作目录（成功/失败均清 [照抄 MPT finally delete_files]）。
    super::cleanup_work_dir(&work_dir);

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
