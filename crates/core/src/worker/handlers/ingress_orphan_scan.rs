//! `ingress.orphan_scan` — pending-placeholder timeout alert (§10.7 兜底):
//! receipts whose step timeline still contains a `pending` job placeholder
//! older than the threshold (job crashed before its terminal flip). Emits an
//! `integration.alert` event per orphan. Schedule via the admin cron menu.

use crate::db::driver::DbDriver;
use serde_json::Value;

use crate::errors::app_error::AppResult;
use crate::worker::Job;
use crate::worker::handler::{HandlerMeta, JobHandler};

static META: HandlerMeta = HandlerMeta {
    id: "ingress.orphan_scan",
    display_name: "集成孤儿步骤扫描",
    description: "扫描步骤时间线中长期停留在 pending 的异步占位（任务崩溃未回写），发出 integration.alert 告警事件",
    category: "集成",
    params_schema: Some(
        r#"{"type":"object","properties":{"timeout_minutes":{"type":"integer","description":"pending 超时阈值（分钟，默认 10）"}}}"#,
    ),
    icon: None,
};

pub struct IngressOrphanScanHandler;

#[async_trait::async_trait]
impl JobHandler for IngressOrphanScanHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let timeout_minutes = payload
            .get("timeout_minutes")
            .and_then(Value::as_u64)
            .unwrap_or(10);
        let Some(plane) = crate::integration::shared() else {
            return Ok(());
        };
        let pool = plane.pool();

        // Delivered receipts still holding pending placeholders. Delivered_at
        // bounds the age (placeholder written at route time, same tx).
        let sql = format!(
            "SELECT id, steps FROM itg_receipts WHERE status = 'delivered' \
             AND delivered_at IS NOT NULL AND delivered_at < {}",
            crate::db::Driver::ph(1)
        );
        let cutoff = crate::utils::tz::now_utc()
            - chrono::TimeDelta::try_minutes(timeout_minutes as i64).unwrap_or_default();
        let rows: Vec<(i64, Option<Value>)> = sqlx::query_as(crate::db::safe_sql(&sql))
            .bind(cutoff)
            .fetch_all(pool)
            .await?;

        let mut orphans = 0_u64;
        for (trace_id, steps) in rows {
            let Some(arr) = steps.and_then(|v| v.as_array().cloned()) else {
                continue;
            };
            let stuck: Vec<&Value> = arr
                .iter()
                .filter(|s| {
                    s["status"] == "pending"
                        && s["step"].as_str().is_some_and(|n| n.starts_with("job:"))
                })
                .collect();
            if stuck.is_empty() {
                continue;
            }
            orphans += stuck.len() as u64;
            tracing::warn!(
                trace_id,
                stuck = stuck.len(),
                "ingress pending placeholder timed out — job crashed before terminal flip?"
            );
            plane.emit_alert(
                "integration.orphan_step",
                serde_json::json!({
                    "trace_id": trace_id,
                    "stuck": stuck,
                    "timeout_minutes": timeout_minutes,
                }),
            );
        }
        tracing::info!(orphans, "ingress.orphan_scan complete");
        Ok(())
    }
}

crate::register_cron_handler!(&META, |_deps| Box::new(IngressOrphanScanHandler));
