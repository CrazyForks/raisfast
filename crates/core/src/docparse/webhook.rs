//! job 终态 webhook 回调（M3）——best-effort：失败仅记日志，
//! 不重试（调用方可在结果保留期内自行拉取）。

use std::time::Duration;

use serde_json::Value;

/// 向 `url` POST 终态载荷。任何失败只记 warn。
pub async fn fire(url: &str, payload: &Value) {
    let client = reqwest::Client::new();
    let result = client
        .post(url)
        .timeout(Duration::from_secs(10))
        .json(payload)
        .send()
        .await;
    match result {
        Ok(r) => tracing::info!(status = %r.status(), url, "docparse webhook fired"),
        Err(e) => tracing::warn!(error = %e, url, "docparse webhook fire failed"),
    }
}
