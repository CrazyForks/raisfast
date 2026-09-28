//! Pixabay 库存素材源 —— 协议照抄 MPT `search_videos_pixabay:394`。
//!
//! key 走 query 参数（非 header）；`videos` 是按尺寸键的对象（非数组，
//! 如 `large`/`medium`）；方形素材宽松（合成阶段裁剪），横竖屏严格匹配
//! 方向；宽度 ≥ 目标宽即可（候选由合成裁剪）。

use serde_json::Value;

use super::{MaterialCandidate, matches_orientation};

pub fn parse_response(
    body: &Value,
    min_duration: u32,
    orientation: &str,
    min_width: u32,
) -> Vec<MaterialCandidate> {
    let mut out = Vec::new();
    let Some(hits) = body.get("hits").and_then(Value::as_array) else {
        return out;
    };
    for v in hits {
        let duration = v.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32;
        if duration < min_duration {
            continue;
        }
        let Some(sizes) = v.get("videos").and_then(Value::as_object) else {
            continue;
        };
        let mut best: Option<(u32, u32, String)> = None;
        for video in sizes.values() {
            let (Ok(w), Ok(h)) = (
                video
                    .get("width")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .parse::<u32>(),
                video
                    .get("height")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .parse::<u32>(),
            ) else {
                continue;
            };
            // 方形输出宽松（合成裁剪）；横竖屏严格 [照抄 MPT 注释]。
            let orientation_matches =
                orientation == "square" || matches_orientation(w, h, orientation);
            if orientation_matches
                && w >= min_width
                && best.as_ref().is_none_or(|(bw, bh, _)| w * h > bw * bh)
                && let Some(link) = video.get("url").and_then(Value::as_str)
            {
                best = Some((w, h, link.to_string()));
            }
        }
        let Some((width, height, url)) = best else {
            continue;
        };
        out.push(MaterialCandidate {
            provider: "pixabay".into(),
            url,
            duration,
            width: Some(width),
            height: Some(height),
            asset_id: v.get("id").map(|i| i.to_string()),
            source_page: v.get("pageURL").and_then(Value::as_str).map(str::to_string),
            creator: v
                .pointer("/user/name")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_object_shaped_sizes_with_min_width() {
        let body = json!({
            "hits": [
                { "id": 1, "duration": 12,
                  "videos": {
                      "small": { "url": "https://cdn.pixabay.com/s.mp4", "width": "960", "height": "540" },
                      "large": { "url": "https://cdn.pixabay.com/l.mp4", "width": "1920", "height": "1080" }
                  },
                  "pageURL": "https://pixabay.com/v/1", "user": { "name": "artist" } }
            ]
        });
        let out = parse_response(&body, 0, "landscape", 1920);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].url, "https://cdn.pixabay.com/l.mp4",
            "取 ≥min_width 中面积最大"
        );
        assert_eq!(out[0].width, Some(1920));
        assert_eq!(out[0].creator, Some("artist".into()));
    }

    #[test]
    fn square_is_lenient_on_orientation() {
        let body = json!({
            "hits": [
                { "id": 2, "duration": 8,
                  "videos": { "tiny": { "url": "https://cdn.pixabay.com/t.mp4", "width": "640", "height": "1280" } } }
            ]
        });
        let out = parse_response(&body, 0, "landscape", 0);
        // landscape 严格匹配 → 竖屏文件被拒
        assert!(out.is_empty(), "非 square orientation 的竖屏文件不应入选");
        let square = parse_response(&body, 0, "square", 0);
        assert_eq!(square.len(), 1, "square 宽松（合成裁剪）");
    }

    #[test]
    fn below_min_width_or_min_duration_filtered() {
        let body = json!({
            "hits": [
                { "id": 3, "duration": 2, "videos": { "tiny": { "url": "https://cdn.pixabay.com/s.mp4", "width": "640", "height": "1280" } } },
                { "id": 4, "duration": 9, "videos": { "tiny": { "url": "https://cdn.pixabay.com/s.mp4", "width": "640", "height": "1280" } } }
            ]
        });
        let out = parse_response(&body, 5, "landscape", 1080);
        assert!(out.is_empty(), "短时长 + 低于目标宽都被过滤");
    }
}
