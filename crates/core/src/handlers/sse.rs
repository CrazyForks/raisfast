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
        "system",
        "sse"
    )
    .route("/events/session", axum::routing::get(subscribe_session))
    .route("/presence/heartbeat", axum::routing::post(heartbeat))
    .route("/presence/status", axum::routing::post(set_status))
}

/// POST /api/v1/presence/heartbeat — refresh the caller's presence
/// liveness signal (architecture §5.3). Body is ignored; identity comes
/// from the authenticated user. Returns `{ok: true}` and never produces a
/// presence.* event when the effective status did not change (the 30s
/// heartbeat cadence emits nothing unless the user was offline).
pub async fn heartbeat(
    auth: crate::middleware::auth::AuthUser,
    State(state): State<crate::AppState>,
) -> crate::errors::app_error::AppResult<crate::errors::response::ApiResponse<serde_json::Value>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let tenant_id = auth.tenant_id().unwrap_or(crate::constants::DEFAULT_TENANT);
    if let Some(t) = state.presence.touch(tenant_id, user_id.0) {
        crate::presence::emit_transition(&state.eventbus, &t);
    }
    Ok(crate::errors::response::ApiResponse::success(
        serde_json::json!({ "ok": true }),
    ))
}

/// POST /api/v1/presence/status — set the caller's manual availability wish
/// (e.g. "away"/"busy" while stepping away; architecture §5.3 `set_manual`).
/// Body: `{"status": "away"}` — the only valid values are the Availability
/// strings (`online/busy/away/offline`); a null/absent status clears it.
/// The human wish has highest priority in the effective merge rule.
pub async fn set_status(
    auth: crate::middleware::auth::AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> crate::errors::app_error::AppResult<crate::errors::response::ApiResponse<serde_json::Value>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let tenant_id = auth.tenant_id().unwrap_or(crate::constants::DEFAULT_TENANT);
    let manual = match body.get("status").and_then(serde_json::Value::as_str) {
        Some("online") => Some(crate::presence::Availability::Online),
        Some("busy") => Some(crate::presence::Availability::Busy),
        Some("away") => Some(crate::presence::Availability::Away),
        Some("offline") => Some(crate::presence::Availability::Offline),
        Some(_) => {
            return Err(crate::errors::app_error::AppError::BadRequest(
                "invalid presence status".into(),
            ));
        }
        None => None, // clear manual
    };
    if let Some(t) = state.presence.set_manual(tenant_id, user_id.0, manual) {
        crate::presence::emit_transition(&state.eventbus, &t);
    }
    Ok(crate::errors::response::ApiResponse::success(
        serde_json::json!({ "ok": true }),
    ))
}

/// Extract event type name
pub fn event_type_name(event: &Event) -> Cow<'static, str> {
    event.display_name()
}

struct ActiveGuard(&'static std::sync::atomic::AtomicU64);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Presence-aware SSE guard: on connect, `touch`/`connect` the authenticated
/// user into the presence store; on stream drop, `disconnect` (architecture
/// §5.3). Disconnect is NOT an immediate offline — the reaper converts
/// staleness once the heartbeat TTL passes.
struct PresenceGuard {
    state: crate::AppState,
    tenant_id: String,
    subject_id: i64,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        if let Some(t) = self
            .state
            .presence
            .disconnect(&self.tenant_id, self.subject_id)
        {
            crate::presence::emit_transition(&self.state.eventbus, &t);
        }
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
    auth: crate::middleware::auth::AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<SubscribeQuery>,
) -> crate::errors::app_error::AppResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    // Connection cap (§10.2): refuse beyond the configured concurrent cap.
    static ACTIVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let max_clients = state.config.sse_max_clients;
    let current = ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    if current >= max_clients {
        return Err(crate::errors::app_error::AppError::TooManyRequests(
            "SSE connection cap reached".into(),
        ));
    }
    ACTIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guard = ActiveGuard(&ACTIVE);

    // Presence: live connection counts toward presence (multi-tab = multiple
    // conns). A connect also refreshes last_seen so a reconnect after a
    // network blip resurrects immediately.
    let presence_guard = auth.user_id().map(|uid| {
        let tenant_id = auth
            .tenant_id()
            .unwrap_or(crate::constants::DEFAULT_TENANT)
            .to_string();
        if let Some(t) = state.presence.connect(&tenant_id, uid) {
            crate::presence::emit_transition(&state.eventbus, &t);
        }
        PresenceGuard {
            state: state.clone(),
            tenant_id,
            subject_id: uid,
        }
    });

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

        // `Event::Custom` surfaces its inner event_type as the SSE event name
        // (e.g. `integration.received`, `integration.channel_state`) so filters
        // like `integration.*` work; other events keep their display name.
        let custom_type = match arc_event.as_ref() {
            Event::Custom { event_type, .. } => Some(event_type.clone()),
            _ => None,
        };
        let type_name = custom_type
            .clone()
            .map(std::borrow::Cow::Owned)
            .unwrap_or_else(|| event_type_name(arc_event.as_ref()));

        if !filter_types.is_empty()
            && !filter_types.iter().any(|f| {
                f == type_name.as_ref()
                    || (custom_type.is_some() && f == "Custom")
                    || f.strip_suffix('*')
                        .is_some_and(|prefix| type_name.starts_with(prefix))
            })
        {
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

    let stream = stream.map(move |item| {
        let _hold = &guard;
        let _presence = &presence_guard;
        item
    });
    // Guard released when the stream is dropped (client disconnect).
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(30))
            .text("ping"),
    ))
}

/// Public session SSE — widget visitors subscribe with their short-session JWT
/// (`Bearer <widget token>`). Events are filtered server-side to the token's
/// claims (widget.md §3.3): only `chat.*` events whose payload `contact_id`
/// matches `claims.sub` (and, when present, matching the token's channel) are
/// pushed — a cross-session subscription sees no data.
pub async fn subscribe_session(
    State(state): State<crate::AppState>,
    headers: axum::http::HeaderMap,
) -> crate::errors::app_error::AppResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    // Connection cap (widget.md §3.3): separate budget from the authed stream.
    static ACTIVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let max_session_clients = state.config.sse_max_session_clients;
    let current = ACTIVE.load(std::sync::atomic::Ordering::Relaxed);
    if current >= max_session_clients {
        return Err(crate::errors::app_error::AppError::TooManyRequests(
            "SSE session connection cap reached".into(),
        ));
    }
    ACTIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guard = ActiveGuard(&ACTIVE);

    let auth = headers
        .get(crate::constants::HEADER_AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(auth) = auth else {
        return Err(crate::errors::app_error::AppError::Unauthorized);
    };
    let Some(token) = crate::utils::widget_token::bearer_token(auth) else {
        return Err(crate::errors::app_error::AppError::Unauthorized);
    };
    let Some(claims) =
        crate::utils::widget_token::verify_widget_token(&state.config.jwt_secret, token)
    else {
        return Err(crate::errors::app_error::AppError::Unauthorized);
    };
    let contact_id = claims.sub;
    let channel_key = claims.ch;

    let rx = state.eventbus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        let arc_event: Arc<Event> = match result {
            Ok(e) => e,
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!("SSE session client lagged, skipped {n} events");
                return None;
            }
        };

        let (event_type, data) = match arc_event.as_ref() {
            Event::Custom {
                event_type, data, ..
            } => (event_type.as_str(), data),
            _ => return None,
        };
        if !event_type.starts_with("chat.") {
            return None;
        }
        // Claims-mandated filter: only this contact's events (cross-session = ∅).
        let ev_contact = data
            .get("contact_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !ev_contact.is_empty() && ev_contact != contact_id {
            return None;
        }
        if let Some(ch) = data.get("channel").and_then(serde_json::Value::as_str)
            && !ch.is_empty()
            && ch != channel_key
        {
            return None;
        }

        let json = serde_json::to_string(arc_event.as_ref()).unwrap_or_default();
        Some(Ok(SseEvent::default().event(event_type).data(json)))
    });

    let stream = stream.map(move |item| {
        let _hold = &guard;
        item
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
    use crate::models::user::{RegisteredVia, User, UserStatus};
    use crate::types::snowflake_id::SnowflakeId;

    fn ts() -> crate::utils::tz::Timestamp {
        "2025-01-01T00:00:00Z".parse().unwrap()
    }

    fn make_post_response(id: &str, slug: &str, title: &str) -> PostResponse {
        PostResponse {
            id: SnowflakeId::new(id.parse().unwrap_or(0)),
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
            tenant_id: None,
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
                    tenant_id: None,
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
        assert!(parsed["data"]["id"].is_string());
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
