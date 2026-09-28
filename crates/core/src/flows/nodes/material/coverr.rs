//! Coverr 库存素材源 —— 协议照抄 MPT `search_videos_coverr:520`。
//!
//! Bearer 鉴权；`urls=true` 让搜索响应直含 mp4_download 直链（GET 该 URL
//! 即计入 Coverr 下载统计，无需再调 stats 端点）；duration 为 number/
//! string 双形态；方向经服务端 `filter=is_vertical` 预筛 + 本地
//! is_vertical/max_width 复核（方形无对应布尔筛选，依赖本地宽高复核）。

use serde_json::Value;

use super::{MaterialCandidate, urlencode};

pub const NAME: &str = "coverr";

/// GET /videos?query&page_size&urls=true&sort=popular[&filter=is_vertical]，
/// Authorization: Bearer。
pub fn build_url(
    api_key: &str,
    query: &str,
    orientation: &str,
    per_page: u32,
) -> (String, Vec<(&'static str, String)>) {
    // 服务端方向筛选直接返回目标素材，避免先取热门再本地过滤导致竖屏
    // 候选为空 [照抄 MPT]；方形无对应布尔条件，靠本地宽高复核。
    let filter = match orientation {
        "portrait" => "&filter=is_vertical:true",
        "landscape" => "&filter=is_vertical:false",
        _ => "",
    };
    (
        format!(
            "https://api.coverr.co/videos?query={}&page_size={per_page}&urls=true&sort=popular{filter}",
            urlencode(query),
        ),
        vec![("Authorization", format!("Bearer {api_key}"))],
    )
}

pub fn parse_response(
    body: &Value,
    _min_duration: u32,
    orientation: &str,
) -> Vec<MaterialCandidate> {
    // min_duration 忽略 [照抄 MPT：coverr 不做时长过滤]。
    let _ = _min_duration;
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
        let out = parse_response(&body, 0, "portrait");
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
        assert!(parse_response(&landscape, 0, "portrait").is_empty());
        assert_eq!(parse_response(&landscape, 0, "landscape").len(), 1);
        // square 无服务端筛选，本地也不做方向校验 → 放行
        assert_eq!(parse_response(&landscape, 0, "square").len(), 1);
    }

    #[test]
    fn entries_without_mp4_download_are_skipped() {
        let body = json!({
            "hits": [
                { "id": "c3", "duration": 10, "urls": {} }
            ]
        });
        assert!(parse_response(&body, 0, "portrait").is_empty());
    }

    #[test]
    fn build_url_sets_bearer_and_vertical_filter() {
        let (url, headers) = build_url("cv-1", "城市", "portrait", 20);
        assert!(url.contains("query="));
        assert!(url.contains("urls=true"));
        assert!(url.contains("filter=is_vertical:true"));
        assert!(
            headers
                .iter()
                .any(|(k, v)| *k == "Authorization" && v == "Bearer cv-1"),
            "{headers:?}"
        );
    }
}
