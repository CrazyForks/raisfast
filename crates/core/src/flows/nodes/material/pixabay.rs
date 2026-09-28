//! Pixabay 库存素材源 —— 协议照抄 MPT `search_videos_pixabay:394`。
//!
//! key 走 query 参数（非 header）；`videos` 是按尺寸键的对象（非数组，
//! 如 `large`/`medium`）；方形素材宽松（合成阶段裁剪），横竖屏严格匹配
//! 方向；宽度 ≥ 目标宽即可（候选由合成裁剪）——目标宽由方向派生
//!（portrait 1080 / landscape 1920 / square 1080）。

use serde_json::Value;

use super::{MaterialCandidate, matches_orientation, urlencode};

pub const NAME: &str = "pixabay";

/// GET /api/videos/?q&video_type=all&per_page=50&key（key 在 query，非 header）。
pub fn build_url(
    api_key: &str,
    query: &str,
    _orientation: &str,
    _per_page: u32,
) -> (String, Vec<(&'static str, String)>) {
    // per_page 固定 50 [照抄 MPT]——pixabay 单位是"命中数"非"已过滤候选数"。
    (
        format!(
            "https://pixabay.com/api/videos/?q={}&video_type=all&per_page=50&key={}",
            urlencode(query),
            urlencode(api_key),
        ),
        vec![], // 无鉴权 header
    )
}

pub fn parse_response(
    body: &Value,
    min_duration: u32,
    orientation: &str,
) -> Vec<MaterialCandidate> {
    let min_width = match orientation {
        "landscape" => 1920,
        _ => 1080,
    };
    parse_response_with_min_width(body, min_duration, orientation, min_width)
}

pub fn parse_response_with_min_width(
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
        let out = parse_response(&body, 0, "landscape");
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
        // landscape 严格匹配 → 竖屏文件被拒；square 宽松（方向不筛）
        let body = json!({
            "hits": [
                { "id": 2, "duration": 8,
                  "videos": { "main": { "url": "https://cdn.pixabay.com/t.mp4", "width": "1080", "height": "1920" } } }
            ]
        });
        assert!(
            parse_response(&body, 0, "landscape").is_empty(),
            "非 square orientation 的竖屏文件不应入选"
        );
        assert_eq!(
            parse_response(&body, 0, "square").len(),
            1,
            "square 宽松（方向不筛）"
        );
    }

    #[test]
    fn square_is_lenient_on_orientation_but_min_width_still_applies() {
        let body = json!({
            "hits": [
                { "id": 2, "duration": 8,
                  "videos": { "huge": { "url": "https://cdn.pixabay.com/h.mp4", "width": "1920", "height": "1920" } },
                  "pageURL": "https://pixabay.com/v/2", "user": { "name": "artist" } },
                { "id": 3, "duration": 8,
                  "videos": { "tiny": { "url": "https://cdn.pixabay.com/t.mp4", "width": "640", "height": "640" } },
                  "pageURL": "https://pixabay.com/v/3", "user": { "name": "artist" } }
            ]
        });
        // square 宽松（方向不筛），但 w >= min_width(1080) 仍生效
        let out = parse_response(&body, 0, "square");
        assert_eq!(out.len(), 1, "仅 huge 入选");
        assert_eq!(out[0].url, "https://cdn.pixabay.com/h.mp4");
    }

    #[test]
    fn build_url_puts_key_in_query() {
        let (url, headers) = build_url("px-1", "一只猫", "portrait", 50);
        assert!(url.contains("key=px-1"), "pixabay key 走 query 参数");
        assert!(url.contains("q="));
        assert!(headers.is_empty(), "pixabay 无鉴权 header");
    }
}
