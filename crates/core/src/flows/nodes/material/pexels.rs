//! Pexels 库存素材源 —— 协议照抄 MPT `search_videos_pexels:312`。
//!
//! Authorization 直携裸 key（非 Bearer）；`video_files` 数组取 rendition；
//! 偏差 [自造-放宽]：rendition 取「方向匹配内最大面积」而非精确匹配画幅
//! 分辨率——Pexels rendition 尺寸不保证恰好等于画幅分辨率，精确匹配经常
//! 落空；方向过滤仍保证画幅方向正确。

use serde_json::Value;

use super::{MaterialCandidate, matches_orientation};

pub fn parse_response(
    body: &Value,
    min_duration: u32,
    orientation: &str,
) -> Vec<MaterialCandidate> {
    let mut out = Vec::new();
    let Some(videos) = body.get("videos").and_then(Value::as_array) else {
        return out;
    };
    for v in videos {
        let duration = v.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32;
        if duration < min_duration {
            continue;
        }
        let mut best: Option<(u32, u32, String)> = None;
        for file in v
            .get("video_files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(w), Some(h)) = (
                file.get("width").and_then(Value::as_u64),
                file.get("height").and_then(Value::as_u64),
            ) else {
                continue;
            };
            let (w, h) = (w as u32, h as u32);
            if !matches_orientation(w, h, orientation) {
                continue;
            }
            if best.as_ref().is_none_or(|(bw, bh, _)| w * h > bw * bh)
                && let Some(link) = file.get("link").and_then(Value::as_str)
            {
                best = Some((w, h, link.to_string()));
            }
        }
        let Some((width, height, url)) = best else {
            continue;
        };
        out.push(MaterialCandidate {
            provider: "pexels".into(),
            url,
            duration,
            width: Some(width),
            height: Some(height),
            asset_id: v.get("id").map(|i| i.to_string()),
            source_page: v.get("url").and_then(Value::as_str).map(str::to_string),
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
    fn filters_min_duration_and_picks_largest_orientation_matched() {
        let body = json!({
            "videos": [
                { "id": 1, "duration": 2, "video_files": [
                    { "link": "https://cdn.pexels.com/short.mp4", "width": 1920, "height": 1080 }
                ]},
                { "id": 2, "duration": 10, "video_files": [
                    { "link": "https://cdn.pexels.com/small.mp4", "width": 720, "height": 1280 },
                    { "link": "https://cdn.pexels.com/hd.mp4", "width": 1080, "height": 1920 },
                    { "link": "https://cdn.pexels.com/land.mp4", "width": 1920, "height": 1080 }
                ],
                 "url": "https://pexels.com/v/2", "user": { "name": "creator" } }
            ]
        });
        let out = parse_response(&body, 5, "portrait");
        assert_eq!(out.len(), 1, "短素材被过滤");
        let c = &out[0];
        assert_eq!(
            c.url, "https://cdn.pexels.com/hd.mp4",
            "横屏文件被过滤，竖屏取最大面积"
        );
        assert_eq!(c.width, Some(1080));
        assert_eq!(c.height, Some(1920));
        assert_eq!(c.duration, 10);
        assert_eq!(c.asset_id, Some("2".into()));
        assert_eq!(c.creator, Some("creator".into()));
    }

    #[test]
    fn missing_videos_key_returns_empty() {
        assert!(parse_response(&json!({}), 0, "portrait").is_empty());
    }

    #[test]
    fn landscape_orientation_filters_portrait_files() {
        let body = json!({
            "videos": [
                { "id": 3, "duration": 8, "video_files": [
                    { "link": "https://cdn.pexels.com/portrait.mp4", "width": 720, "height": 1280 },
                    { "link": "https://cdn.pexels.com/land.mp4", "width": 1920, "height": 1080 }
                ]}
            ]
        });
        let out = parse_response(&body, 0, "landscape");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://cdn.pexels.com/land.mp4");
    }
}
