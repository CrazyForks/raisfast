//! 全局事件总线
//!
//! 基于 `tokio::sync::broadcast` 的发布-订阅事件系统。
//! 所有业务事件（文章创建、评论、用户注册等）通过 `EventBus` 广播，
//! 各子系统（插件、通知、审计、缓存）订阅感兴趣的事件。

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::broadcast;

/// 业务事件
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum Event {
    // ── 内容生命周期（泛化，兼容旧事件） ──
    PostCreating {
        id: String,
        title: String,
    },
    PostCreated {
        id: String,
        slug: String,
        title: String,
        author_id: String,
    },
    PostUpdated {
        id: String,
        slug: String,
    },
    PostDeleted {
        id: String,
        slug: String,
    },
    CommentCreated {
        id: String,
        post_slug: String,
        author_name: String,
    },
    CommentDeleted {
        id: String,
    },

    // ── 通用内容事件（Phase 10 新增） ──
    ContentCreating {
        content_type: String,
        id: String,
        data: serde_json::Value,
    },
    ContentCreated {
        content_type: String,
        id: String,
        slug: Option<String>,
    },
    ContentUpdating {
        content_type: String,
        id: String,
        data: serde_json::Value,
    },
    ContentUpdated {
        content_type: String,
        id: String,
    },
    ContentDeleted {
        content_type: String,
        id: String,
    },

    // ── 用户/媒体 ──
    UserRegistered {
        id: String,
        username: String,
        email: String,
    },
    UserLoggedIn {
        id: String,
        success: bool,
    },
    MediaUploaded {
        id: String,
        filename: String,
        uploader_id: String,
    },
    MediaDeleted {
        id: String,
    },
}

/// 事件订阅者
///
/// 每个 subscriber 独立消费事件，慢消费者会收到 `RecvError::Lagged`。
pub type EventReceiver = broadcast::Receiver<Arc<Event>>;

/// 事件总线
///
/// 线程安全，可在 `AppState` 中通过 `Arc` 共享。
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
}

impl EventBus {
    /// 创建指定容量的 `EventBus`
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// 发布事件，所有订阅者都会收到
    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(Arc::new(event));
    }

    /// 订阅事件流
    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.tx.subscribe()
    }

    /// 当前订阅者数量
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_and_receive() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(Event::PostCreated {
            id: "test-1".into(),
            slug: "hello".into(),
            title: "Hello".into(),
            author_id: "u1".into(),
        });

        let event = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { rx.recv().await.unwrap() });

        match event.as_ref() {
            Event::PostCreated { id, slug, .. } => {
                assert_eq!(id, "test-1");
                assert_eq!(slug, "hello");
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(Event::UserRegistered {
            id: "u1".into(),
            username: "alice".into(),
            email: "a@b.com".into(),
        });

        let rt = tokio::runtime::Runtime::new().unwrap();
        let e1 = rt.block_on(async { rx1.recv().await.unwrap() });
        let e2 = rt.block_on(async { rx2.recv().await.unwrap() });
        assert!(matches!(e1.as_ref(), Event::UserRegistered { .. }));
        assert!(matches!(e2.as_ref(), Event::UserRegistered { .. }));
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
        bus.emit(Event::PostDeleted {
            id: "x".into(),
            slug: "y".into(),
        });
    }
}
