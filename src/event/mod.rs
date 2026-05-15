//! Unified event definitions
//!
//! Single `Event` enum for all hooks — lifecycle, utility, and custom.
//! `name()` uses `on_` prefix which doubles as the WASM function name.
//! `display_name()` returns PascalCase for SSE/frontend.
//! `table()` returns the DB table if applicable.
//!
//! Adding a new event: add variant + optional `#[event(table = "...", name = "...")]`.

use serde::{Deserialize, Serialize};

use crate::dto::PostResponse;
use crate::models::comment::Comment;
use crate::models::email_verification::EmailVerificationToken;
use crate::models::media::Media;
use crate::models::password_reset::PasswordResetToken;
use crate::models::post::Post;
use crate::models::user::User;

pub use raisfast_derive::EventMeta;

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
    PasswordResetRequested {
        user: User,
        token: PasswordResetToken,
    },
    EmailVerificationRequested {
        user_id: i64,
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
            status: crate::models::post::PostStatus::Draft,
            created_by: 0,
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
