//! 全局事件总线
//!
//! 基于 `tokio::sync::broadcast` 的发布-订阅事件系统。
//! 所有业务事件通过 `EventBus` 广播，各子系统订阅感兴趣的事件。
//! `emit_with_aspects()` 在发布前/后经过 AOP Event Layer 拦截。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::aspects::engine::AspectEngine;
use crate::aspects::{BaseContext, EventContext};

/// 业务事件
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    // ── 认证 ──
    PasswordResetRequested {
        user_id: String,
        email: String,
        reset_token: String,
    },
    EmailVerificationRequested {
        user_id: String,
        email: String,
        verify_token: String,
    },

    // ── 插件自定义事件 ──
    Custom {
        source: String,
        event_type: String,
        data: serde_json::Value,
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

    /// 带 AOP Event Layer 拦截的事件发布
    ///
    /// 流程：dispatch_before → 广播 → dispatch_after
    /// 如果 before 返回 false（被 aspect 阻止），事件不会发布。
    /// dispatch_after 的错误仅记录日志，不影响发布结果。
    pub async fn emit_with_aspects(
        &self,
        event: Event,
        engine: &AspectEngine,
        base_ctx: BaseContext,
    ) {
        let event_type = event_label(&event);
        let payload = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);

        let mut ctx = EventContext {
            base: base_ctx,
            event_type: event_type.to_string(),
            payload,
            table: event_table(&event),
        };

        match engine
            .dispatch_event_before_publish(&event_type, &mut ctx)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!("event {event_type} blocked by aspect");
                return;
            }
            Err(e) => {
                tracing::warn!("event {event_type} before_dispatch error: {e}");
                return;
            }
        }

        let publish_payload = ctx.payload.clone();
        let event = match serde_json::from_value::<Event>(publish_payload) {
            Ok(e) => e,
            Err(_) => event,
        };
        self.emit(event);

        if let Err(e) = engine
            .dispatch_event_after_publish(&event_type, &mut ctx)
            .await
        {
            tracing::warn!("event {event_type} after_dispatch error: {e}");
        }
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

fn event_label(event: &Event) -> String {
    match event {
        Event::PostCreating { .. } => "post_creating".to_string(),
        Event::PostCreated { .. } => "post_created".to_string(),
        Event::PostUpdated { .. } => "post_updated".to_string(),
        Event::PostDeleted { .. } => "post_deleted".to_string(),
        Event::CommentCreated { .. } => "comment_created".to_string(),
        Event::CommentDeleted { .. } => "comment_deleted".to_string(),
        Event::ContentCreating { .. } => "content_creating".to_string(),
        Event::ContentCreated { .. } => "content_created".to_string(),
        Event::ContentUpdating { .. } => "content_updating".to_string(),
        Event::ContentUpdated { .. } => "content_updated".to_string(),
        Event::ContentDeleted { .. } => "content_deleted".to_string(),
        Event::UserRegistered { .. } => "user_registered".to_string(),
        Event::UserLoggedIn { .. } => "user_logged_in".to_string(),
        Event::MediaUploaded { .. } => "media_uploaded".to_string(),
        Event::MediaDeleted { .. } => "media_deleted".to_string(),
        Event::PasswordResetRequested { .. } => "password_reset_requested".to_string(),
        Event::EmailVerificationRequested { .. } => "email_verification_requested".to_string(),
        Event::Custom { event_type, .. } => event_type.clone(),
    }
}

fn event_table(event: &Event) -> Option<String> {
    match event {
        Event::PostCreating { .. }
        | Event::PostCreated { .. }
        | Event::PostUpdated { .. }
        | Event::PostDeleted { .. } => Some("posts".to_string()),
        Event::CommentCreated { .. } | Event::CommentDeleted { .. } => {
            Some("comments".to_string())
        }
        Event::ContentCreating { content_type, .. }
        | Event::ContentCreated { content_type, .. }
        | Event::ContentUpdating { content_type, .. }
        | Event::ContentUpdated { content_type, .. }
        | Event::ContentDeleted { content_type, .. } => Some(content_type.clone()),
        Event::MediaUploaded { .. } | Event::MediaDeleted { .. } => Some("media".to_string()),
        Event::UserRegistered { .. } | Event::UserLoggedIn { .. } => {
            Some("users".to_string())
        }
        _ => None,
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

    #[tokio::test]
    async fn emit_with_aspects_publishes_when_no_aspects() {
        let bus = EventBus::new(16);
        let engine = AspectEngine::new();
        let mut rx = bus.subscribe();
        let base_ctx = BaseContext::new(None, "default".into(), "now".into());

        bus.emit_with_aspects(
            Event::PostCreated {
                id: "1".into(),
                slug: "test".into(),
                title: "Test".into(),
                author_id: "u1".into(),
            },
            &engine,
            base_ctx,
        )
        .await;

        let event = rx.recv().await.unwrap();
        match event.as_ref() {
            Event::PostCreated { id, .. } => assert_eq!(id, "1"),
            _ => panic!("wrong event type"),
        }
    }

    #[tokio::test]
    async fn emit_with_aspects_blocked_by_aspect() {
        use crate::aspects::{Advice, Aspect, AspectResult, Layer, Operation, Pointcut, TargetMatcher, When};

        struct BlockAllEvents;
        #[async_trait::async_trait]
        impl Aspect for BlockAllEvents {
            fn name(&self) -> &str {
                "block_events"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::Before,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
                Ok(Advice::Skip)
            }
        }

        let bus = EventBus::new(16);
        let engine = AspectEngine::new();
        engine.register(BlockAllEvents);
        let base_ctx = BaseContext::new(None, "default".into(), "now".into());

        bus.emit_with_aspects(
            Event::PostCreated {
                id: "1".into(),
                slug: "test".into(),
                title: "Test".into(),
                author_id: "u1".into(),
            },
            &engine,
            base_ctx,
        )
        .await;

        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn emit_with_aspects_runs_after_dispatch() {
        use std::sync::{Arc, Mutex};
        use crate::aspects::{Advice, Aspect, AspectResult, Layer, Operation, Pointcut, TargetMatcher, When};

        let flag: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        struct AfterAspect {
            flag: Arc<Mutex<bool>>,
        }
        #[async_trait::async_trait]
        impl Aspect for AfterAspect {
            fn name(&self) -> &str {
                "after_flag"
            }
            fn pointcuts(&self) -> Vec<Pointcut> {
                vec![Pointcut {
                    layer: Layer::Event,
                    operation: Operation::Publish,
                    when: When::After,
                    target: TargetMatcher::All,
                }]
            }
            async fn on_event_after_publish(&self, _ctx: &mut EventContext) -> AspectResult {
                *self.flag.lock().unwrap() = true;
                Ok(Advice::Continue)
            }
        }

        let bus = EventBus::new(16);
        let engine = AspectEngine::new();
        engine.register(AfterAspect { flag: flag.clone() });
        let base_ctx = BaseContext::new(None, "default".into(), "now".into());

        bus.emit_with_aspects(
            Event::PostCreated {
                id: "1".into(),
                slug: "test".into(),
                title: "Test".into(),
                author_id: "u1".into(),
            },
            &engine,
            base_ctx,
        )
        .await;

        assert!(*flag.lock().unwrap());
    }
}
