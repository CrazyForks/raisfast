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
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::dto::SubscribeQuery;
use crate::eventbus::Event;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
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

/// Extract event type name
pub fn event_type_name(event: &Event) -> Cow<'static, str> {
    event.display_name()
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
    use crate::dto::PostResponse;
    use crate::models::comment::{Comment, CommentStatus};
    use crate::models::email_verification::EmailVerificationToken;
    use crate::models::media::Media;
    use crate::models::password_reset::PasswordResetToken;
    use crate::models::post::{CommentOpenStatus, Post, PostStatus};
    use crate::models::user::{RegisteredVia, User, UserRole, UserStatus};
    use crate::types::snowflake_id::SnowflakeId;

    fn ts() -> crate::utils::tz::Timestamp {
        "2025-01-01T00:00:00Z".parse().unwrap()
    }

    fn make_post_response(id: &str, slug: &str, title: &str) -> PostResponse {
        PostResponse {
            id: id.into(),
            title: title.into(),
            slug: slug.into(),
            content: String::new(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: PostStatus::Published,
            created_by: None,
            author_name: None,
            category_id: None,
            category_name: None,
            tags: vec![],
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: CommentOpenStatus::Open,
            format: String::new(),
            template: String::new(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: ts(),
            updated_at: ts(),
            published_at: None,
            title_highlight: None,
            excerpt_highlight: None,
        }
    }

    fn make_post(id: i64, slug: &str) -> Post {
        Post {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            title: String::new(),
            slug: slug.into(),
            content: String::new(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: PostStatus::Published,
            created_by: crate::types::snowflake_id::SnowflakeId(0),
            updated_by: None,
            category_id: None,
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: CommentOpenStatus::Open,
            format: String::new(),
            template: String::new(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: ts(),
            updated_at: ts(),
            published_at: None,
        }
    }

    fn make_comment(id: i64) -> Comment {
        Comment {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            post_id: crate::types::snowflake_id::SnowflakeId(0),
            created_by: None,
            updated_by: None,
            nickname: None,
            email: None,
            content: String::new(),
            parent_id: None,
            author_ip: None,
            author_url: None,
            status: CommentStatus::Approved,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn make_user(id: i64, username: &str) -> User {
        User {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            username: username.into(),
            role: UserRole::Reader,
            status: UserStatus::Active,
            registered_via: RegisteredVia::Email,
            avatar: None,
            bio: None,
            website: None,
            display_name: None,
            slug: None,
            locale: None,
            social_links: None,
            metadata: None,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn make_media(id: i64, filename: &str) -> Media {
        Media {
            id: crate::types::snowflake_id::SnowflakeId(id),
            tenant_id: None,
            user_id: crate::types::snowflake_id::SnowflakeId(1),
            filename: filename.into(),
            filepath: String::new(),
            mimetype: String::new(),
            size: 0,
            width: None,
            height: None,
            title: None,
            alt_text: None,
            caption: None,
            description: None,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn make_password_reset_token(user_id: i64) -> PasswordResetToken {
        PasswordResetToken {
            id: crate::types::snowflake_id::SnowflakeId(1),
            user_id: crate::types::snowflake_id::SnowflakeId(user_id),
            token: "reset-token".into(),
            expires_at: ts(),
            used_at: None,
            created_at: ts(),
        }
    }

    fn make_email_verification_token(user_id: i64, email: &str) -> EmailVerificationToken {
        EmailVerificationToken {
            id: crate::types::snowflake_id::SnowflakeId(1),
            user_id: crate::types::snowflake_id::SnowflakeId(user_id),
            token: "verify-token".into(),
            email: email.into(),
            expires_at: ts(),
            verified_at: None,
            created_at: ts(),
        }
    }

    #[test]
    fn all_event_types_have_correct_names() {
        let cases: Vec<(Event, &'static str)> = vec![
            (
                Event::PostCreated(make_post_response("1", "s", "t")),
                "PostCreated",
            ),
            (Event::PostUpdated(make_post(1, "s")), "PostUpdated"),
            (Event::PostDeleted(make_post(1, "s")), "PostDeleted"),
            (Event::CommentCreated(make_comment(1)), "CommentCreated"),
            (Event::UserRegistered(make_user(1, "u")), "UserRegistered"),
            (
                Event::UserLoggedIn {
                    user: make_user(1, "u"),
                    success: true,
                },
                "UserLoggedIn",
            ),
            (Event::MediaUploaded(make_media(1, "f")), "MediaUploaded"),
            (Event::MediaDeleted(make_media(1, "f")), "MediaDeleted"),
            (
                Event::PasswordResetRequested {
                    user: make_user(1, "u"),
                    token: make_password_reset_token(1),
                },
                "PasswordResetRequested",
            ),
            (
                Event::EmailVerificationRequested {
                    user_id: SnowflakeId(1),
                    email: "e".into(),
                    token: make_email_verification_token(1, "e"),
                },
                "EmailVerificationRequested",
            ),
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
        let event = Event::PostCreated(make_post_response("p1", "hello", "Hello"));
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

        bus_emit.emit(Event::PostCreated(make_post_response("p1", "test", "Test")));

        let e1 = tokio::time::timeout(std::time::Duration::from_millis(100), rx1.recv())
            .await
            .unwrap()
            .unwrap();
        let e2 = tokio::time::timeout(std::time::Duration::from_millis(100), rx2.recv())
            .await
            .unwrap()
            .unwrap();

        assert!(matches!(e1.as_ref(), Event::PostCreated(..)));
        assert!(matches!(e2.as_ref(), Event::PostCreated(..)));
    }

    #[tokio::test]
    async fn broadcast_filters_by_type_name() {
        let bus = crate::eventbus::EventBus::new(64);
        let bus_emit = bus.clone();

        let mut rx = bus.subscribe();

        bus_emit.emit(Event::PostCreated(make_post_response("p1", "test", "Test")));
        bus_emit.emit(Event::CommentCreated(make_comment(1)));
        bus_emit.emit(Event::PostDeleted(make_post(1, "test")));

        let allowed = ["PostCreated".to_string()];
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

        bus_emit.emit(Event::PostCreated(make_post_response("p1", "test", "Test")));
        bus_emit.emit(Event::CommentCreated(make_comment(1)));
        bus_emit.emit(Event::UserLoggedIn {
            user: make_user(1, "u"),
            success: true,
        });

        let allowed = ["PostCreated".to_string(), "CommentCreated".to_string()];
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
