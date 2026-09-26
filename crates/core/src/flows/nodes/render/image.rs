//! `render_image` 节点 executor（render-node.md §7.1）——推文形态：图片段 Ken Burns + 旁白/BGM/字幕合成，管线与 render_video 共享。Ken Burns 3%/s 慢推 [照抄 MPT render_image_zoom_video]。

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::flows::engine::{ExecOutcome, Pool};
use crate::flows::graph::GraphNode;
use crate::storage::Storage;

use super::RenderCommon;

/// `render_image` 节点 config（render-node.md §7.1）。`timeline` 每项
/// `{image: {key|url}, duration: 秒}`——每张图配对旁白时长。
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RenderImageConfig {
    /// 图片段序列 — ValueExpr → 数组。
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub timeline: Value,
    /// 旁白主音轨 — ValueExpr → 音频引用（ speech 节点产物）。
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub narration: Option<Value>,
    #[serde(flatten)]
    pub common: RenderCommon,
}

/// Config validation.
pub(crate) fn validate(config: &Value) -> AppResult<()> {
    let c: RenderImageConfig = serde_json::from_value(config.clone())
        .map_err(|e| AppError::BadRequest(format!("node 'render_image' config invalid: {e}")))?;
    if c.timeline.is_null() {
        return Err(AppError::BadRequest(
            "render_image: timeline 不能为空".into(),
        ));
    }
    super::validate_value_expr("render.timeline", &c.timeline)?;
    if let Some(n) = &c.narration {
        super::validate_value_expr("render.narration", n)?;
    }
    c.common.validate()?;
    Ok(())
}

/// ① image 段生成：Ken Burns 慢推（3%/s，居中锚点 [照抄 MPT
/// `render_image_zoom_video`]），精确帧数控制时长。
async fn image_segment(
    work_dir: &Path,
    input: &str,
    index: usize,
    duration: f64,
    w: u32,
    h: u32,
) -> AppResult<String> {
    let out = format!("seg-{index}.mp4");
    let fps = 30.0;
    let frames = (duration * fps).round().max(1.0) as i64;
    // zoompan：on=输出帧号；30fps 下 on/30=秒 → 3%/s 慢推，封顶 1.15
    // [照抄 MPT 5s→115% 的等效速率]。先归一到画布再推近。
    let video_filter = format!(
        "scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},\
         zoompan=z='min(1+0.03*on/30,1.15)':d=1:x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':s={w}x{h}:fps={fps},setsar=1"
    );
    super::run_ffmpeg(
        work_dir,
        &[
            "-y",
            "-loop",
            "1",
            "-i",
            input,
            "-vf",
            &video_filter,
            "-frames:v",
            &frames.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            &out,
        ],
    )
    .await?;
    Ok(out)
}

/// ② concat（image 段已统一编码 → copy）。
async fn concat_segments(work_dir: &Path, segments: &[String]) -> AppResult<String> {
    let list = segments
        .iter()
        .map(|s| format!("file '{s}'"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(work_dir.join("concat-list.txt"), list)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("render_image: 写 concat list: {e}")))?;
    super::run_ffmpeg(
        work_dir,
        &[
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            "concat-list.txt",
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

/// Execute the `render_image` node.
///
/// # Errors
/// `BadRequest` on missing template refs / missing ffmpeg; `Internal` on
/// ffmpeg failures or timeouts. Crash re-run = idempotent re-render.
pub async fn run_render_image(
    storage: &Arc<dyn Storage>,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: RenderImageConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("render_image config: {e}")))?;
    let started = Instant::now();

    if super::ffmpeg_path().is_none() {
        return Err(AppError::BadRequest(
            "render_image: ffmpeg 不可用（安装 ffmpeg 或配置 PATH 后重试）".into(),
        ));
    }

    let segments = super::resolve_timeline(&cfg.timeline, pool)?;
    if segments.is_empty() {
        return Err(AppError::BadRequest(
            "render_image: timeline 为空数组".into(),
        ));
    }
    let (w, h) = cfg.common.canvas();
    let bgm_volume = cfg.common.bgm_volume.unwrap_or(0.2);
    let timeout_ms = cfg.common.timeout_ms();

    let run_id = crate::utils::id::new_id().to_string();
    let work_dir = super::new_work_dir(&run_id)?;

    let pipeline = async {
        // ① materialize images + generate Ken Burns segments
        let mut seg_files = Vec::with_capacity(segments.len());
        let mut total = 0.0f64;
        for (i, seg) in segments.iter().enumerate() {
            let image_ref = seg.get("image").cloned().unwrap_or_else(|| seg.clone());
            let input =
                super::materialize_ref(storage, &work_dir, &image_ref, &format!("in-{i}")).await?;
            let duration = seg
                .get("duration")
                .and_then(Value::as_f64)
                .unwrap_or(5.0)
                .clamp(0.5, 60.0);
            let f = image_segment(&work_dir, &input, i + 1, duration, w, h);
            let f = f.await?;
            seg_files.push(f);
            total += duration;
        }
        // ② concat
        let combined = concat_segments(&work_dir, &seg_files).await?;
        let duration = super::probe_duration(&work_dir, &combined).await?;
        let _ = total;
        // ④ finalize：旁白主音轨 + BGM 垫底 + 字幕
        let narration = match &cfg.narration {
            Some(expr) => {
                let resolved = crate::flows::engine::resolve(expr, pool)?;
                if resolved.is_null() {
                    None
                } else {
                    Some(super::materialize_ref(storage, &work_dir, &resolved, "narration").await?)
                }
            }
            None => None,
        };
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
                bgm: bgm.as_deref(),
                bgm_volume,
                narration: narration.as_deref(),
                subtitles: subtitles.as_deref(),
                font_name: cfg.common.font_name.as_deref(),
                font_size: cfg.common.font_size(),
                subtitle_position: cfg.common.subtitle_position(),
            },
        )
        .await?;
        let bytes = std::fs::read(work_dir.join(&final_file))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("render_image: 读取成片: {e}")))?;
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
        .map_err(|_| AppError::Internal(anyhow::anyhow!("render_image 超时 {timeout_ms}ms")));

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
        #[allow(clippy::new_ret_no_self)]
        fn new() -> Arc<dyn crate::storage::Storage> {
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

    fn image_node(config: Value) -> GraphNode {
        GraphNode {
            id: "ri".into(),
            data: NodeData {
                kind: "render_image".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[test]
    fn render_image_config_validation() {
        assert!(
            super::validate(&json!({"timeline": {"literal": []}})).is_ok(),
            "最小配置"
        );
        assert!(super::validate(&json!({})).is_err(), "缺 timeline");
        assert!(
            super::validate(
                &json!({"timeline": {"literal": []}, "narration": {"ref": ["s", "audio"]}})
            )
            .is_ok(),
            "narration 合法"
        );
    }

    fn ffmpeg_available() -> bool {
        super::super::ffmpeg_path().is_some()
    }

    /// 真实渲染冒烟（ffmpeg 在场才跑）：2 张测试图（各 1.5s Ken Burns）
    /// → render_image → ffprobe 断言成片 ≈3s 且已落 storage。
    #[tokio::test]
    async fn smoke_images_render_to_storage() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let storage = MemStorage::new();
        let img_dir =
            std::env::temp_dir().join(format!("raisfast-img-test-{}", crate::utils::id::new_id()));
        std::fs::create_dir_all(&img_dir).unwrap();
        let ff = super::super::ffmpeg_path().unwrap();

        for (i, color) in ["red", "blue"].iter().enumerate() {
            let out = img_dir.join(format!("img-{i}.png"));
            let st = tokio::process::Command::new(ff)
                .args([
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color={color}:size=1080x1920"),
                    "-frames:v",
                    "1",
                ])
                .arg(&out)
                .output()
                .await
                .unwrap();
            assert!(
                st.status.success(),
                "img {i} gen failed: {}",
                String::from_utf8_lossy(&st.stderr)
            );
            storage
                .put(
                    &format!("img-{i}.png"),
                    &std::fs::read(&out).unwrap(),
                    "image/png",
                )
                .await
                .unwrap();
        }
        let _ = std::fs::remove_dir_all(&img_dir);

        let node = image_node(json!({
            "timeline": {"literal": [
                {"image": {"key": "img-0.png"}, "duration": 1.5},
                {"image": {"key": "img-1.png"}, "duration": 1.5}
            ]},
            "aspect": "9:16"
        }));
        let out = run_render_image(&storage, &node, &Pool::new())
            .await
            .unwrap();
        let key = out.output["video"]["key"].as_str().unwrap().to_string();
        assert!(key.starts_with("gen/flows/"));
        let bytes = storage.get(&key).await.unwrap();
        assert!(bytes.len() > 1000, "成片过小: {}", bytes.len());
        let duration = out.output["duration_s"].as_f64().unwrap();
        assert!((duration - 3.0).abs() < 0.5, "duration {duration}");
    }
}
