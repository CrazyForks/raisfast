//! Unified event definitions
//!
//! Single `Event` enum for all hooks — lifecycle, utility, and custom.
//! - `name()` uses `on_` prefix which doubles as the WASM function name.
//! - `display_name()` returns PascalCase for logging.
//! - `event_name()` returns the stable external contract name (e.g., `"post.created"`)
//!   declared via `#[event(event_name = "...")]`. Used by webhooks, SSE, and audit logs.
//! - `table()` returns the DB table if applicable (metadata only).
//! - `audit_info()` auto-derives audit fields from `event_name` + serde payload,
//!   with hand-written overrides for events that need richer detail.
//!
//! Adding a new event: add variant + `#[event(table, event_name)]`.
//! Audit logging is automatic for any event with an `event_name`.

use crate::types::snowflake_id::SnowflakeId;
use serde::{Deserialize, Serialize};

use crate::dto::PostResponse;
use crate::models::category::Category;
use crate::models::comment::Comment;
use crate::models::email_verification::EmailVerificationToken;
use crate::models::media::Media;
use crate::models::order::Order;
use crate::models::page::Page;
use crate::models::password_reset::PasswordResetToken;
use crate::models::payment_order::PaymentOrder;
use crate::models::post::Post;
use crate::models::product::Product;
use crate::models::product_category::ProductCategory;
use crate::models::tag::Tag;
use crate::models::user::User;
use crate::models::wallet_transaction::WalletTransaction;

pub use raisfast_derive::EventMeta;

pub struct AuditInfo {
    pub action: String,
    pub subject: String,
    pub subject_id: String,
    pub actor_id: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, EventMeta)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum Event {
    // ── Post lifecycle ──
    #[event(table = "posts")]
    PostCreating,
    #[event(table = "posts", event_name = "post.created")]
    PostCreated(PostResponse),
    #[event(table = "posts")]
    PostUpdating,
    #[event(table = "posts", event_name = "post.updated")]
    PostUpdated(Post),
    #[event(table = "posts", event_name = "post.deleted")]
    PostDeleted(Post),

    // ── Comment lifecycle ──
    #[event(table = "comments")]
    CommentCreating,
    #[event(table = "comments", event_name = "comment.created")]
    CommentCreated(Comment),
    #[event(table = "comments", event_name = "comment.updated")]
    CommentUpdated(Comment),
    #[event(table = "comments", event_name = "comment.deleted")]
    CommentDeleted(Comment),

    // ── Tag lifecycle ──
    #[event(table = "tags", event_name = "tag.created")]
    TagCreated(Tag),
    #[event(table = "tags", event_name = "tag.updated")]
    TagUpdated(Tag),
    #[event(table = "tags", event_name = "tag.deleted")]
    TagDeleted(Tag),

    // ── Category lifecycle ──
    #[event(table = "categories", event_name = "category.created")]
    CategoryCreated(Category),
    #[event(table = "categories", event_name = "category.updated")]
    CategoryUpdated(Category),
    #[event(table = "categories", event_name = "category.deleted")]
    CategoryDeleted(Category),

    // ── Page lifecycle ──
    #[event(table = "pages", event_name = "page.created")]
    PageCreated(Page),
    #[event(table = "pages", event_name = "page.updated")]
    PageUpdated(Page),
    #[event(table = "pages", event_name = "page.deleted")]
    PageDeleted(Page),

    // ── Product lifecycle ──
    #[event(table = "products", event_name = "product.created")]
    ProductCreated(Product),
    #[event(table = "products", event_name = "product.updated")]
    ProductUpdated(Product),
    #[event(table = "products", event_name = "product.deleted")]
    ProductDeleted(Product),

    // ── Product Category lifecycle ──
    #[event(table = "product_categories", event_name = "product_category.created")]
    ProductCategoryCreated(ProductCategory),
    #[event(table = "product_categories", event_name = "product_category.updated")]
    ProductCategoryUpdated(ProductCategory),
    #[event(table = "product_categories", event_name = "product_category.deleted")]
    ProductCategoryDeleted(ProductCategory),

    // ── Product Comment lifecycle ──
    #[event(table = "product_comments", event_name = "product_comment.created")]
    ProductCommentCreated(crate::models::product_comment::ProductComment),
    #[event(table = "product_comments", event_name = "product_comment.updated")]
    ProductCommentUpdated(crate::models::product_comment::ProductComment),
    #[event(table = "product_comments", event_name = "product_comment.deleted")]
    ProductCommentDeleted(crate::models::product_comment::ProductComment),

    // ── Order lifecycle ──
    #[event(table = "orders", event_name = "order.created")]
    OrderCreated(Order),
    #[event(table = "orders", event_name = "order.paid")]
    OrderPaid(Order),
    #[event(table = "orders", event_name = "order.shipped")]
    OrderShipped(Order),
    #[event(table = "orders", event_name = "order.completed")]
    OrderCompleted(Order),
    #[event(table = "orders", event_name = "order.cancelled")]
    OrderCancelled(Order),

    // ── Payment lifecycle ──
    #[event(table = "payment_orders", event_name = "payment_order.created")]
    PaymentOrderCreated(PaymentOrder),
    #[event(table = "payment_orders", event_name = "payment_order.paid")]
    PaymentPaid(PaymentOrder),
    #[event(table = "payment_orders", event_name = "payment_order.refunded")]
    PaymentRefunded(PaymentOrder),

    // ── Wallet lifecycle ──
    #[event(
        table = "wallet_transactions",
        event_name = "wallet_transaction.credited"
    )]
    WalletCredited(WalletTransaction),
    #[event(
        table = "wallet_transactions",
        event_name = "wallet_transaction.debited"
    )]
    WalletDebited(WalletTransaction),

    // ── Generic CMS content lifecycle ──
    ContentCreating,
    ContentCreated,
    ContentUpdating,
    ContentUpdated,
    ContentDeleted,
    ContentViewed,

    // ── User ──
    #[event(table = "users", event_name = "user.registered")]
    UserRegistered(User),
    #[event(table = "users", event_name = "user.loggedIn")]
    UserLoggedIn {
        user: User,
        success: bool,
    },

    // ── Media ──
    #[event(table = "media", event_name = "media.uploaded")]
    MediaUploaded(Media),
    #[event(table = "media", event_name = "media.deleted")]
    MediaDeleted(Media),

    // ── Auth ──
    #[event(table = "users", event_name = "user.password_reset_requested")]
    PasswordResetRequested {
        user: User,
        token: PasswordResetToken,
    },
    #[event(table = "users", event_name = "user.email_verification_requested")]
    EmailVerificationRequested {
        user_id: SnowflakeId,
        email: String,
        token: EmailVerificationToken,
    },

    // ── Plugin custom ──
    #[event(dynamic)]
    Custom {
        source: String,
        event_type: String,
        data: serde_json::Value,
    },
}

impl Event {
    /// Generic audit info derived from `event_name` + serde payload.
    ///
    /// Used as a fallback for events that don't have a hand-written `audit_info` arm.
    /// Derives:
    /// - `subject` / `action` from `event_name` (e.g., `"order.paid"` → subject=`"order"`, action=`"paid"`)
    /// - `subject_id` from the payload's `id` field
    /// - `actor_id` from the payload's `user_id` or `created_by` field
    fn generic_audit_info(&self) -> Option<AuditInfo> {
        let event_name = self.event_name()?;
        let (subject, action) = event_name.split_once('.')?;

        // Extract the inner data payload (serde tag = "type", content = "data")
        let value = serde_json::to_value(self).ok()?;
        let data = value.get("data")?;

        let subject_id = data
            .get("id")
            .and_then(|v| {
                v.as_i64()
                    .map(|i| i.to_string())
                    .or_else(|| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default();

        let actor_id = data
            .get("user_id")
            .and_then(|v| v.as_i64())
            .or_else(|| data.get("created_by").and_then(|v| v.as_i64()));

        Some(AuditInfo {
            action: action.to_string(),
            subject: subject.to_string(),
            subject_id,
            actor_id,
            detail: None,
        })
    }

    pub fn audit_info(&self) -> Option<AuditInfo> {
        match self {
            Event::PostCreated(data) => Some(AuditInfo {
                action: "create".into(),
                subject: "post".into(),
                subject_id: data.id.clone(),
                actor_id: data.created_by.as_deref().and_then(|s| s.parse().ok()),
                detail: Some(format!("title={}", data.title)),
            }),
            Event::PostUpdated(data) => Some(AuditInfo {
                action: "update".into(),
                subject: "post".into(),
                subject_id: data.id.to_string(),
                actor_id: None,
                detail: Some(format!("slug={}", data.slug)),
            }),
            Event::PostDeleted(data) => Some(AuditInfo {
                action: "delete".into(),
                subject: "post".into(),
                subject_id: data.id.to_string(),
                actor_id: None,
                detail: Some(format!("slug={}", data.slug)),
            }),
            Event::CommentCreated(data) => Some(AuditInfo {
                action: "create".into(),
                subject: "comment".into(),
                subject_id: data.id.to_string(),
                actor_id: None,
                detail: Some(format!(
                    "author={}",
                    data.nickname.clone().unwrap_or_default()
                )),
            }),
            Event::UserRegistered(data) => Some(AuditInfo {
                action: "register".into(),
                subject: "user".into(),
                subject_id: data.id.to_string(),
                actor_id: None,
                detail: Some(format!("username={}", data.username)),
            }),
            Event::UserLoggedIn { user, success } => Some(AuditInfo {
                action: "login".into(),
                subject: "user".into(),
                subject_id: user.id.to_string(),
                actor_id: Some(*user.id),
                detail: Some(format!("success={}", success)),
            }),
            Event::MediaUploaded(data) => Some(AuditInfo {
                action: "upload".into(),
                subject: "media".into(),
                subject_id: data.id.to_string(),
                actor_id: Some(*data.user_id),
                detail: Some(format!("filename={}", data.filename)),
            }),
            Event::MediaDeleted(data) => Some(AuditInfo {
                action: "delete".into(),
                subject: "media".into(),
                subject_id: data.id.to_string(),
                actor_id: None,
                detail: None,
            }),
            Event::PasswordResetRequested { user, token: _ } => Some(AuditInfo {
                action: "password_reset_request".into(),
                subject: "user".into(),
                subject_id: user.id.to_string(),
                actor_id: None,
                detail: Some(format!("username={}", user.username)),
            }),
            Event::EmailVerificationRequested { user_id, email, .. } => Some(AuditInfo {
                action: "email_verification_request".into(),
                subject: "user".into(),
                subject_id: user_id.to_string(),
                actor_id: None,
                detail: Some(format!("email={}", email)),
            }),
            _ => self.generic_audit_info(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_post_response(id: &str, slug: &str, title: &str) -> PostResponse {
        PostResponse {
            id: id.into(),
            slug: slug.into(),
            title: title.into(),
            content: String::new(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: crate::models::post::PostStatus::Draft,
            created_by: None,
            author_name: None,
            category_id: None,
            category_name: None,
            tags: vec![],
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: crate::models::post::CommentOpenStatus::Open,
            format: String::new(),
            template: String::new(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: Default::default(),
            updated_at: Default::default(),
            published_at: None,
            title_highlight: None,
            excerpt_highlight: None,
        }
    }

    #[test]
    fn name_auto() {
        assert_eq!(Event::PostCreating.name(), "on_post_creating");
        assert_eq!(
            Event::PostCreated(make_post_response("1", "s", "t")).name(),
            "on_post_created"
        );
    }

    #[test]
    fn name_dynamic() {
        let e = Event::Custom {
            source: "test".into(),
            event_type: "my_event".into(),
            data: serde_json::Value::Null,
        };
        assert_eq!(e.name(), "my_event");
    }

    #[test]
    fn display_name_auto() {
        assert_eq!(Event::PostCreating.display_name(), "PostCreating");
    }

    #[test]
    fn table_mapping() {
        assert_eq!(Event::PostCreating.table(), Some("posts"));
        assert_eq!(Event::CommentCreating.table(), Some("comments"));
        assert_eq!(Event::ContentCreating.table(), None);
    }
}
