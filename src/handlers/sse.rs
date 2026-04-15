//! SSE（Server-Sent Events）实时推送处理器
//!
//! 将 `EventBus` 的事件流转换为 HTTP SSE 推送，供前端实时接收业务事件。
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
        Event::ContentCreating { .. } => "ContentCreating",
        Event::ContentCreated { .. } => "ContentCreated",
        Event::ContentUpdating { .. } => "ContentUpdating",
        Event::ContentUpdated { .. } => "ContentUpdated",
        Event::ContentDeleted { .. } => "ContentDeleted",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventbus::Event;

    #[test]
    fn all_event_types_have_correct_names() {
        let cases: Vec<(Event, &'static str)> = vec![
            (
                Event::PostCreating {
                    id: "1".into(),
                    title: "t".into(),
                },
                "PostCreating",
            ),
            (
                Event::PostCreated {
                    id: "1".into(),
                    slug: "s".into(),
                    title: "t".into(),
                    author_id: "u".into(),
                },
                "PostCreated",
            ),
            (
                Event::PostUpdated {
                    id: "1".into(),
                    slug: "s".into(),
                },
                "PostUpdated",
            ),
            (
                Event::PostDeleted {
                    id: "1".into(),
                    slug: "s".into(),
                },
                "PostDeleted",
            ),
            (
                Event::CommentCreated {
                    id: "1".into(),
                    post_slug: "s".into(),
                    author_name: "a".into(),
                },
                "CommentCreated",
            ),
            (Event::CommentDeleted { id: "1".into() }, "CommentDeleted"),
            (
                Event::UserRegistered {
                    id: "1".into(),
                    username: "u".into(),
                    email: "e".into(),
                },
                "UserRegistered",
            ),
            (
                Event::UserLoggedIn {
                    id: "1".into(),
                    success: true,
                },
                "UserLoggedIn",
            ),
            (
                Event::MediaUploaded {
                    id: "1".into(),
                    filename: "f".into(),
                    uploader_id: "u".into(),
                },
                "MediaUploaded",
            ),
            (Event::MediaDeleted { id: "1".into() }, "MediaDeleted"),
        ];

        assert_eq!(cases.len(), 10, "所有 Event 变体都应有对应名称");
        for (event, expected_name) in &cases {
            assert_eq!(event_type_name(event), *expected_name);
        }
    }

    #[test]
    fn event_serialization_contains_type_tag() {
        let event = Event::PostCreated {
            id: "p1".into(),
            slug: "hello".into(),
            title: "Hello".into(),
            author_id: "u1".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "PostCreated");
        assert_eq!(parsed["data"]["id"], "p1");
        assert_eq!(parsed["data"]["slug"], "hello");
    }

    #[tokio::test]
    async fn broadcast_delivers_events_to_subscribers() {
        let bus = crate::eventbus::EventBus::new(64);
        let bus_emit = bus.clone();

        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus_emit.emit(Event::PostCreated {
            id: "p1".into(),
            slug: "test".into(),
            title: "Test".into(),
            author_id: "u1".into(),
        });

        let e1 = tokio::time::timeout(std::time::Duration::from_millis(100), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        let e2 = tokio::time::timeout(std::time::Duration::from_millis(100), rx2.recv())
            .await
            .unwrap()
            .unwrap();

        assert!(matches!(e1.as_ref(), Event::PostCreated { .. }));
        assert!(matches!(e2.as_ref(), Event::PostCreated { .. }));
    }

    #[tokio::test]
    async fn broadcast_filters_by_type_name() {
        let bus = crate::eventbus::EventBus::new(64);
        let bus_emit = bus.clone();

        let mut rx = bus.subscribe();

        bus_emit.emit(Event::PostCreated {
            id: "p1".into(),
            slug: "test".into(),
            title: "Test".into(),
            author_id: "u1".into(),
        });
        bus_emit.emit(Event::CommentCreated {
            id: "c1".into(),
            post_slug: "test".into(),
            author_name: "alice".into(),
        });
        bus_emit.emit(Event::PostDeleted {
            id: "p1".into(),
            slug: "test".into(),
        });

        let allowed = vec!["PostCreated".to_string()];
        let mut received = Vec::new();
        for _ in 0..3 {
            let event = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let name = event_type_name(event.as_ref());
            if allowed.iter().any(|a| a == name) {
                received.push(name.to_string());
            }
        }

        assert_eq!(received.len(), 1);
        assert_eq!(received[0], "PostCreated");
    }

    #[tokio::test]
    async fn broadcast_filter_multiple_types() {
        let bus = crate::eventbus::EventBus::new(64);
        let bus_emit = bus.clone();

        let mut rx = bus.subscribe();

        bus_emit.emit(Event::PostCreated {
            id: "p1".into(),
            slug: "test".into(),
            title: "Test".into(),
            author_id: "u1".into(),
        });
        bus_emit.emit(Event::CommentCreated {
            id: "c1".into(),
            post_slug: "test".into(),
            author_name: "alice".into(),
        });
        bus_emit.emit(Event::UserLoggedIn {
            id: "u1".into(),
            success: true,
        });

        let allowed = vec!["PostCreated".to_string(), "CommentCreated".to_string()];
        let mut received = Vec::new();
        for _ in 0..3 {
            let event = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let name = event_type_name(event.as_ref());
            if allowed.iter().any(|a| a == name) {
                received.push(name.to_string());
            }
        }

        assert_eq!(received.len(), 2);
        assert!(received.contains(&"PostCreated".to_string()));
        assert!(received.contains(&"CommentCreated".to_string()));
    }

    #[tokio::test]
    async fn no_events_returns_empty_on_timeout() {
        let bus = crate::eventbus::EventBus::new(64);
        let mut rx = bus.subscribe();

        let result = tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv()).await;
        assert!(result.is_err());
    }

    #[test]
    fn subscribe_query_filter_parsing() {
        let q: SubscribeQuery =
            serde_urlencoded::from_str("filter=PostCreated,CommentCreated").unwrap();
        assert_eq!(q.filter.as_deref(), Some("PostCreated,CommentCreated"));

        let q: SubscribeQuery = serde_urlencoded::from_str("").unwrap();
        assert!(q.filter.is_none());
    }
}
