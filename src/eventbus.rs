//! Global event bus
//!
//! A publish-subscribe event system based on `tokio::sync::broadcast`.
//! All business events are broadcast via `EventBus`, with each subsystem subscribing to events of interest.

use std::sync::Arc;

use tokio::sync::broadcast;

pub use crate::event::Event;

/// Event subscriber
///
/// Each subscriber independently consumes events. Slow consumers will receive `RecvError::Lagged`.
pub type EventReceiver = broadcast::Receiver<Arc<Event>>;

/// Event bus
///
/// Thread-safe, can be shared via `Arc` in `AppState`.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
}

impl EventBus {
    /// Create an `EventBus` with the specified capacity
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event; all subscribers will receive it
    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(Arc::new(event));
    }

    /// Subscribe to the event stream
    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.tx.subscribe()
    }

    /// Current subscriber count
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::PostResponse;
    use crate::models::post::{CommentOpenStatus, Post, PostStatus};
    use crate::models::user::{RegisteredVia, User, UserRole, UserStatus};

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

    #[test]
    fn emit_and_receive() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(Event::PostCreated(make_post_response(
            "test-1", "hello", "Hello",
        )));

        let event = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { rx.recv().await.unwrap() });

        match event.as_ref() {
            Event::PostCreated(data) => {
                assert_eq!(data.id, "test-1");
                assert_eq!(data.slug, "hello");
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(Event::UserRegistered(make_user(1, "alice")));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let e1 = rt.block_on(async { rx1.recv().await.unwrap() });
        let e2 = rt.block_on(async { rx2.recv().await.unwrap() });
        assert!(matches!(e1.as_ref(), Event::UserRegistered(..)));
        assert!(matches!(e2.as_ref(), Event::UserRegistered(..)));
    }

    #[test]
    fn subscriber_count() {
        let bus = EventBus::new(16);
        assert_eq!(bus.subscriber_count(), 0);
        let rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        drop(rx1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[test]
    fn no_subscribers_emit_does_not_panic() {
        let bus = EventBus::new(16);
        bus.emit(Event::PostDeleted(make_post(0, "y")));
    }

    #[test]
    fn event_name_and_metadata() {
        let e = Event::PostCreated(make_post_response("1", "s", "t"));
        assert_eq!(e.name(), "on_post_created");
        assert_eq!(e.table(), Some("posts"));
        assert_eq!(e.display_name(), "PostCreated");
    }
}
