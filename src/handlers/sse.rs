//! SSE（Server-Sent Events）实时推送处理器
//!
//! 将 EventBus 的事件流转换为 HTTP SSE 推送，供前端实时接收业务事件。
//! 支持按事件类型过滤和心跳保活。

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::sse::Event as SseEvent;
use axum::response::sse::{KeepAlive, Sse};
use futures::stream::Stream;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::eventbus::Event;

/// SSE 订阅查询参数
#[derive(Debug, Deserialize, Default)]
pub struct SubscribeQuery {
    /// 逗号分隔的事件类型过滤，如 `PostCreated,CommentCreated`
    /// 为空时订阅所有事件
    pub filter: Option<String>,
}

/// 提取事件类型名称
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::PostCreating { .. } => "PostCreating",
        Event::PostCreated { .. } => "PostCreated",
        Event::PostUpdated { .. } => "PostUpdated",
        Event::PostDeleted { .. } => "PostDeleted",
        Event::CommentCreated { .. } => "CommentCreated",
        Event::CommentDeleted { .. } => "CommentDeleted",
        Event::UserRegistered { .. } => "UserRegistered",
        Event::UserLoggedIn { .. } => "UserLoggedIn",
        Event::MediaUploaded { .. } => "MediaUploaded",
        Event::MediaDeleted { .. } => "MediaDeleted",
    }
}

/// SSE 事件订阅端点
///
/// - **方法/路径：** `GET /api/v1/events`
/// - **认证：** 无需认证（后续可加）
/// - **说明：** 返回 SSE 事件流，客户端通过 `EventSource` API 订阅。
///   支持通过 `?filter=PostCreated,CommentCreated` 按事件类型过滤。
///   每 30 秒发送心跳保活。
pub async fn subscribe(
    State(state): State<crate::AppState>,
    Query(query): Query<SubscribeQuery>,
) -> crate::errors::app_error::AppResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    let rx = state.eventbus.subscribe();
    let filter_types: Vec<String> = query
        .filter
        .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        let arc_event: Arc<Event> = match result {
            Ok(e) => e,
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!("SSE client lagged, skipped {n} events");
                return None;
            }
        };

        let type_name = event_type_name(arc_event.as_ref());

        if !filter_types.is_empty() && !filter_types.iter().any(|f| f == type_name) {
            return None;
        }

        let data = match serde_json::to_string(arc_event.as_ref()) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("SSE serialize error: {e}");
                return None;
            }
        };

        let sse_event = SseEvent::default().event(type_name).data(data);

        Some(Ok(sse_event))
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(30))
            .text("ping"),
    ))
}
