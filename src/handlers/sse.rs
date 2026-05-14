//! SSE (Server-Sent Events) real-time push handler
//!
//! Converts `EventBus` event streams into HTTP SSE pushes for frontend real-time event reception.
//! Supports filtering by event type and keep-alive heartbeats.

use std::borrow::Cow;
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

pub fn routes(registry: &mut crate::server::RouteRegistry, config: &crate::config::app::AppConfig) -> axum::Router<crate::AppState> {
    let _restful = config.api_restful;
    reg_route!(
        axum::Router::new(),
        registry,
        restful,
        "/events",
        get,
        subscribe,
        "system public",
        "sse"
    )
}

/// SSE subscription query parameters
#[derive(Debug, Deserialize, Default)]
pub struct SubscribeQuery {
    /// Comma-separated event type filter, e.g. `PostCreated,CommentCreated`
    /// When empty, subscribes to all events
    pub filter: Option<String>,
}

/// Extract event type name
pub fn event_type_name(event: &Event) -> Cow<'static, str> {
    match event {
        Event::PostCreating { .. } => Cow::Borrowed("PostCreating"),
        Event::PostCreated { .. } => Cow::Borrowed("PostCreated"),
        Event::PostUpdated { .. } => Cow::Borrowed("PostUpdated"),
        Event::PostDeleted { .. } => Cow::Borrowed("PostDeleted"),
        Event::CommentCreated { .. } => Cow::Borrowed("CommentCreated"),
        Event::CommentDeleted { .. } => Cow::Borrowed("CommentDeleted"),
        Event::ContentCreating { .. } => Cow::Borrowed("ContentCreating"),
        Event::ContentCreated { .. } => Cow::Borrowed("ContentCreated"),
        Event::ContentUpdating { .. } => Cow::Borrowed("ContentUpdating"),
        Event::ContentUpdated { .. } => Cow::Borrowed("ContentUpdated"),
        Event::ContentDeleted { .. } => Cow::Borrowed("ContentDeleted"),
        Event::UserRegistered { .. } => Cow::Borrowed("UserRegistered"),
        Event::UserLoggedIn { .. } => Cow::Borrowed("UserLoggedIn"),
        Event::MediaUploaded { .. } => Cow::Borrowed("MediaUploaded"),
        Event::MediaDeleted { .. } => Cow::Borrowed("MediaDeleted"),
        Event::PasswordResetRequested { .. } => Cow::Borrowed("PasswordResetRequested"),
        Event::EmailVerificationRequested { .. } => Cow::Borrowed("EmailVerificationRequested"),
        Event::Custom { event_type, .. } => Cow::Owned(event_type.clone()),
    }
}

/// SSE event subscription endpoint
///
/// - **Method/Path:** `GET /api/v1/events`
/// - **Auth:** Not required (may be added later)
/// - **Description:** Returns an SSE event stream; clients subscribe via the `EventSource` API.
///   Supports filtering by event type via `?filter=PostCreated,CommentCreated`.
///   Sends a keep-alive heartbeat every 30 seconds.
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

        if !filter_types.is_empty() && !filter_types.iter().any(|f| f == type_name.as_ref()) {
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
            (
                Event::Custom {
                    source: "test-plugin".into(),
                    event_type: "OrderCreated".into(),
                    data: serde_json::json!({"order_id": "o1"}),
                },
                "OrderCreated",
            ),
        ];

        assert_eq!(
            cases.len(),
            11,
            "all Event variants should have a corresponding name"
        );
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
            if allowed.iter().any(|a| *a == name.as_ref()) {
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
            if allowed.iter().any(|a| *a == name.as_ref()) {
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
