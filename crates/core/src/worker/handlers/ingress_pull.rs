//! `ingress.pull` job handler — scheduled pull execution for cursor channels
//! (integration.md §5.3). Job claim semantics give single-flight per tick
//! (防重入); payloads carry `channel_key`.

use crate::errors::app_error::AppResult;
use crate::worker::handler::{HandlerMeta, JobHandler};
use crate::worker::Job;

static META: HandlerMeta = HandlerMeta {
    id: "ingress.pull",
    display_name: "集成渠道拉取",
    description: "对 cursor 模式的 pull 渠道执行一轮分页拉取（建议通过渠道的 cron 调度自动运行）",
    category: "集成",
    params_schema: Some(r#"{"type":"object","properties":{"channel_key":{"type":"string","description":"渠道路由键"}},"required":["channel_key"]}"#),
    icon: None,
};

pub struct IngressPullHandler;

#[async_trait::async_trait]
impl JobHandler for IngressPullHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let Some(channel_key) = payload.get("channel_key").and_then(|v| v.as_str()) else {
            tracing::warn!("ingress.pull without channel_key: {payload}");
            return Ok(());
        };
        let Some(plane) = crate::integration::shared_plane() else {
            tracing::warn!(channel_key, "ingress.pull: integration plane not initialized");
            return Ok(());
        };
        let channel = plane
            .channels()
            .get(crate::constants::DEFAULT_TENANT, channel_key)
            .await?;
        if channel.mode != "pull" {
            tracing::warn!(channel_key, "ingress.pull: channel is not pull mode — skipped");
            return Ok(());
        }
        if !channel.enabled || channel.shadow {
            return Ok(());
        }
        let token = crate::integration::connector::http_pull::pull_token(&channel, plane.vault())?;
        let summary = crate::integration::connector::http_pull::run(
            plane.pool(),
            plane.pipeline(),
            &channel,
            token,
        )
        .await?;
        tracing::info!(
            channel_key,
            fetched = summary.fetched,
            delivered = summary.delivered,
            duplicates = summary.duplicates,
            failed = summary.failed,
            pages = summary.pages,
            "ingress.pull run complete"
        );
        Ok(())
    }
}

crate::register_cron_handler!(&META, |_deps| Box::new(IngressPullHandler));
