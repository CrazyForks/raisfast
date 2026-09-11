//! Cron handler: llm_channel_health — probe channels and auto-recover
//! auto-disabled ones (design §7.5): every auto-disabled channel plus a
//! sampled slice of enabled channels gets a `max_tokens=1` ping on its
//! `test_model`; a success re-enables exactly the tested key (§6.3
//! per-key recovery semantics).

use std::sync::Arc;
use std::time::Instant;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::llm::models::channel::{LlmChannel, LlmChannelStatus, LlmKeyStatus};
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Metadata for the admin task menu.
pub const META: HandlerMeta = HandlerMeta {
    id: "llm_channel_health",
    display_name: "LLM Channel Health Probe",
    description: "Pings auto-disabled + sampled enabled channels (max_tokens=1) and re-enables recovered keys",
    category: "AI / LLM",
    params_schema: None,
    icon: Some("activity"),
};

pub struct LlmHealthHandler {
    pool: crate::db::Pool,
    #[allow(dead_code)]
    config: Arc<AppConfig>,
}

impl LlmHealthHandler {
    /// Creates the handler.
    #[must_use]
    pub fn new(pool: crate::db::Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for LlmHealthHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let _ = job;
        let pool = self.pool.clone();
        let channels = crate::llm::models::channel::list_channels(&pool, None).await?;
        let mut probed = 0usize;
        let mut recovered = 0usize;
        for row in channels {
            let auto_disabled = row.status == LlmChannelStatus::AutoDisabled;
            // Sample: every auto-disabled channel + every 10th enabled one.
            let should_probe = auto_disabled || probed.is_multiple_of(10);
            probed += 1;
            if !should_probe {
                continue;
            }
            match probe_channel(&row).await {
                ProbeOutcome::Skip => continue,
                ProbeOutcome::Failed(detail) => {
                    tracing::debug!(channel = %row.id, %detail, "llm health probe failed");
                }
                ProbeOutcome::Ok => {
                    if auto_disabled {
                        // Re-enable the first disabled key only, then flip the
                        // channel back on (§6.3: only the tested key recovers).
                        let entries = crate::llm::models::channel::parse_keys(&row);
                        if let Some(idx) = entries
                            .iter()
                            .position(|e| e.status == LlmKeyStatus::Disabled)
                        {
                            let _ = crate::llm::models::channel::update_key_status(
                                &pool,
                                None,
                                row.id,
                                idx,
                                LlmKeyStatus::Active,
                                Some("health probe ok"),
                            )
                            .await;
                        }
                        let _ = crate::llm::models::channel::update_status(
                            &pool,
                            None,
                            row.id,
                            LlmChannelStatus::Enabled,
                        )
                        .await;
                        recovered += 1;
                    }
                }
            }
        }
        tracing::info!(probed, recovered, "llm channel health sweep done");
        Ok(())
    }
}

enum ProbeOutcome {
    Ok,
    Failed(String),
    Skip,
}

async fn probe_channel(row: &LlmChannel) -> ProbeOutcome {
    let entries = crate::llm::models::channel::parse_keys(row);
    let Some(entry) = entries
        .iter()
        .find(|e| e.status == LlmKeyStatus::Active)
        .or_else(|| entries.iter().find(|e| e.status == LlmKeyStatus::Disabled))
    else {
        return ProbeOutcome::Skip;
    };
    let Some(key) = crate::llm::crypto::decrypt(&entry.key) else {
        return ProbeOutcome::Skip;
    };
    let model = row
        .test_model
        .clone()
        .or_else(|| {
            row.models
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if model.is_empty() {
        return ProbeOutcome::Skip;
    }
    let started = Instant::now();
    let client = crate::llm::relay::shared_client();
    let url = format!("{}/chat/completions", row.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1
    });
    let resp = client
        .post(&url)
        .headers(crate::llm::relay::OpenaiHeaders::for_key(
            &key,
            row.header_override.as_ref(),
        ))
        .json(&body)
        .send()
        .await;
    let elapsed_ms = started.elapsed().as_millis() as i32;
    let _ = elapsed_ms;
    match resp {
        Ok(r) if r.status().is_success() => ProbeOutcome::Ok,
        Ok(r) => ProbeOutcome::Failed(format!(
            "HTTP {}: {}",
            r.status().as_u16(),
            r.text().await.unwrap_or_default()
        )),
        Err(e) => ProbeOutcome::Failed(format!("transport: {e}")),
    }
}

crate::register_cron_handler!(&META, |deps| {
    Box::new(LlmHealthHandler::new(
        deps.pool.clone(),
        deps.config.clone(),
    ))
});
