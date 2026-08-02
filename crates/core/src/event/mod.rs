//! Unified event definitions
//!
//! Single `Event` enum for all hooks — lifecycle, utility, and custom.
//! `name()` uses `on_` prefix which doubles as the WASM function name.
//! `display_name()` returns PascalCase for SSE/frontend.
//! `table()` returns the DB table if applicable.
//! `audit_info()` auto-derives audit fields from table + display_name + serde payload.
//!
//! Adding a new event: add variant + optional `#[event(table = "...", name = "...")]`.
//! Audit logging is automatic for any event with a `table` attribute.

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
    #[event(table = "posts")]
    PostCreated(PostResponse),
    #[event(table = "posts")]
    PostUpdating,
    #[event(table = "posts")]
    PostUpdated(Post),
    #[event(table = "posts")]
    PostDeleted(Post),

    // ── Comment lifecycle ──
    #[event(table = "comments")]
    CommentCreating,
    #[event(table = "comments")]
    CommentCreated(Comment),
    #[event(table = "comments")]
    CommentUpdated(Comment),
    #[event(table = "comments")]
    CommentDeleted(Comment),

    // ── Tag lifecycle ──
    #[event(table = "tags")]
    TagCreated(Tag),
    #[event(table = "tags")]
    TagUpdated(Tag),
    #[event(table = "tags")]
    TagDeleted(Tag),

    // ── Category lifecycle ──
    #[event(table = "categories")]
    CategoryCreated(Category),
    #[event(table = "categories")]
    CategoryUpdated(Category),
    #[event(table = "categories")]
    CategoryDeleted(Category),

    // ── Page lifecycle ──
    #[event(table = "pages")]
    PageCreated(Page),
    #[event(table = "pages")]
    PageUpdated(Page),
    #[event(table = "pages")]
    PageDeleted(Page),

    // ── Product lifecycle ──
    #[event(table = "products")]
    ProductCreated(Product),
    #[event(table = "products")]
    ProductUpdated(Product),
    #[event(table = "products")]
    ProductDeleted(Product),

    // ── Product Category lifecycle ──
    #[event(table = "product_categories")]
    ProductCategoryCreated(ProductCategory),
    #[event(table = "product_categories")]
    ProductCategoryUpdated(ProductCategory),
    #[event(table = "product_categories")]
    ProductCategoryDeleted(ProductCategory),

    // ── Product Comment lifecycle ──
    #[event(table = "product_comments")]
    ProductCommentCreated(crate::models::product_comment::ProductComment),
    #[event(table = "product_comments")]
    ProductCommentUpdated(crate::models::product_comment::ProductComment),
    #[event(table = "product_comments")]
    ProductCommentDeleted(crate::models::product_comment::ProductComment),

    // ── Order lifecycle ──
    #[event(table = "orders")]
    OrderCreated(Order),
    #[event(table = "orders")]
    OrderPaid(Order),
    #[event(table = "orders")]
    OrderShipped(Order),
    #[event(table = "orders")]
    OrderCompleted(Order),
    #[event(table = "orders")]
    OrderCancelled(Order),

    // ── Payment lifecycle ──
    #[event(table = "payment_orders")]
    PaymentOrderCreated(PaymentOrder),
    #[event(table = "payment_orders")]
    PaymentPaid(PaymentOrder),
    #[event(table = "payment_orders")]
    PaymentRefunded(PaymentOrder),

    // ── Wallet lifecycle ──
    #[event(table = "wallet_transactions")]
    WalletCredited(WalletTransaction),
    #[event(table = "wallet_transactions")]
    WalletDebited(WalletTransaction),

    // ── Generic CMS content lifecycle ──
    ContentCreating,
    ContentCreated,
    ContentUpdating,
    ContentUpdated,
    ContentDeleted,
    ContentViewed,

    // ── User ──
    #[event(table = "users")]
    UserRegistered(User),
    #[event(table = "users")]
    UserLoggedIn {
        user: User,
        success: bool,
    },

    // ── Media ──
    #[event(table = "media")]
    MediaUploaded(Media),
    #[event(table = "media")]
    MediaDeleted(Media),

    // ── Auth ──
    #[event(table = "users")]
    PasswordResetRequested {
        user: User,
        token: PasswordResetToken,
    },
    #[event(table = "users")]
    EmailVerificationRequested {
        user_id: SnowflakeId,
        email: String,
        token: EmailVerificationToken,
    },

    // ── Utility ──
    #[event(name = "render_markdown")]
    RenderMarkdown,
    #[event(name = "filter_html")]
    FilterHtml,
    #[event(name = "on_login")]
    OnLogin,
    CronTick,

    // ── Plugin custom ──
    #[event(dynamic)]
    Custom {
        source: String,
        event_type: String,
        data: serde_json::Value,
    },
}

impl Event {
    fn snake_to_pascal(s: &str) -> String {
        s.split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect()
    }

    fn generic_audit_info(&self) -> Option<AuditInfo> {
        let table = self.table()?;
        let singular = table.strip_suffix('s').unwrap_or(table);
        let display = self.display_name();
        let prefix = Self::snake_to_pascal(singular);
        let remainder = display.strip_prefix(&prefix)?;
        let action = remainder[..1].to_ascii_lowercase() + &remainder[1..];
        let subject = singular.to_string();

        let value = serde_json::to_value(self).ok()?;
        let data = value.get("data")?;

        let subject_id = data
            .get("id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default();

        let actor_id = data
            .get("user_id")
            .and_then(|v| v.as_i64())
            .or_else(|| data.get("created_by").and_then(|v| v.as_i64()));

        Some(AuditInfo {
            action,
            subject,
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
        assert_eq!(Event::OnLogin.name(), "on_login");
        assert_eq!(Event::CronTick.name(), "on_cron_tick");
    }

    #[test]
    fn name_custom() {
        assert_eq!(Event::RenderMarkdown.name(), "render_markdown");
        assert_eq!(Event::FilterHtml.name(), "filter_html");
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
        assert_eq!(Event::RenderMarkdown.display_name(), "RenderMarkdown");
    }

    #[test]
    fn table_mapping() {
        assert_eq!(Event::PostCreating.table(), Some("posts"));
        assert_eq!(Event::CommentCreating.table(), Some("comments"));
        assert_eq!(Event::ContentCreating.table(), None);
        assert_eq!(Event::RenderMarkdown.table(), None);
    }
}
