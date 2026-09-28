//! Coverr 库存素材源 —— 协议照抄 MPT `search_videos_coverr:520`。
//!
//! Bearer 鉴权；`urls=true` 让搜索响应直含 mp4_download 直链（GET 该 URL
//! 即计入 Coverr 下载统计，无需再调 stats 端点）；duration 为 number/
//! string 双形态；方向经服务端 `filter=is_vertical` 预筛 + 本地
//! is_vertical/max_width 复核（方形无对应布尔筛选，依赖本地宽高复核）。

use serde_json::Value;

use super::MaterialCandidate;

pub fn parse_response(body: &Value, orientation: &str) -> Vec<MaterialCandidate> {
    let mut out = Vec::new();
    let Some(hits) = body.get("hits").and_then(Value::as_array) else {
        return out;
    };
    for v in hits {
        let video_id = v.get("id").map(|i| i.to_string()).unwrap_or_default();
        let mp4 = v
            .get("urls")
            .and_then(|u| u.get("mp4_download"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if mp4.is_empty() {
            continue;
        }
        // 方形无对应布尔筛选，依赖本地宽高复核 [照抄 MPT]。
        if orientation != "square" {
            let is_vertical = v.get("is_vertical").and_then(Value::as_bool);
            let vertical_match = match orientation {
                "portrait" => is_vertical.unwrap_or_else(|| {
                    match (
                        v.get("max_width").and_then(Value::as_u64),
                        v.get("max_height").and_then(Value::as_u64),
                    ) {
                        (Some(w), Some(h)) => h > w,
                        _ => true,
                    }
                }),
                _ => !is_vertical.unwrap_or(false),
            };
            if !vertical_match {
                continue;
            }
        }
        out.push(MaterialCandidate {
            provider: "coverr".into(),
            url: mp4,
            duration: v
                .get("duration")
                .and_then(|d| {
                    d.as_u64()
                        .map(|n| n as u32)
                        .or_else(|| d.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0),
            width: v.get("max_width").and_then(Value::as_u64).map(|n| n as u32),
            height: v
                .get("max_height")
                .and_then(Value::as_u64)
                .map(|n| n as u32),
            asset_id: (!video_id.is_empty()).then_some(video_id),
            source_page: v
                .get("canonical_url")
                .or_else(|| v.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string),
            creator: v
                .get("creator")
                .or_else(|| v.get("author"))
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
    fn parses_mp4_download_with_string_duration() {
        let body = json!({
            "hits": [
                { "id": "c1", "duration": "12", "is_vertical": true,
                  "urls": { "mp4_download": "https://coverr.co/d/c1.mp4" },
                  "max_width": 1080, "max_height": 1920,
                  "canonical_url": "https://coverr.co/v/c1", "creator": "vid" }
            ]
        });
        let out = parse_response(&body, "portrait");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://coverr.co/d/c1.mp4");
        assert_eq!(out[0].duration, 12, "duration 字符串形态");
        assert_eq!(out[0].creator, Some("vid".into()));
    }

    #[test]
    fn portrait_filter_rejects_landscape_and_square_accepts_any() {
        let landscape = json!({
            "hits": [
                { "id": "c2", "duration": 10, "is_vertical": false,
                  "urls": { "mp4_download": "https://coverr.co/d/c2.mp4" } }
            ]
        });
        assert!(parse_response(&landscape, "portrait").is_empty());
        assert_eq!(parse_response(&landscape, "landscape").len(), 1);
        // square 无服务端筛选，本地也不做方向校验 → 放行
        assert_eq!(parse_response(&landscape, "square").len(), 1);
    }

    #[test]
    fn entries_without_mp4_download_are_skipped() {
        let body = json!({
            "hits": [
                { "id": "c3", "duration": 10, "urls": {} }
            ]
        });
        assert!(parse_response(&body, "portrait").is_empty());
    }
}
