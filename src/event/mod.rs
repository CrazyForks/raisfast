//! Unified event definitions
//!
//! Event variants carry full model/dto objects as payload — type-safe, zero-maintenance.
//! Adding a field to a model automatically makes it available to all event consumers.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::dto::PostResponse;
use crate::models::comment::Comment;
use crate::models::email_verification::EmailVerificationToken;
use crate::models::media::Media;
use crate::models::password_reset::PasswordResetToken;
use crate::models::post::Post;
use crate::models::user::User;

/// Whether this event fires before or after the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Phase {
    Before,
    After,
}

/// Unified system event.
///
/// Each variant carries the full model/dto object as payload — no hand-picked fields.
/// Consumers use `serde_json::to_value(&event)` to extract what they need.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", content = "data")]
pub enum Event {
    // ── Post lifecycle ──
    PostCreated(PostResponse),
    PostUpdated(Post),
    PostDeleted(Post),

    // ── Comment lifecycle ──
    CommentCreated(Comment),

    // ── User ──
    UserRegistered(User),
    UserLoggedIn {
        user: User,
        success: bool,
    },

    // ── Media ──
    MediaUploaded(Media),
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

    // ── Plugin custom ──
    Custom {
        source: String,
        event_type: String,
        data: serde_json::Value,
    },
}

impl Event {
    pub fn name(&self) -> Cow<'static, str> {
        match self {
            Self::PostCreated(_) => Cow::Borrowed("post_created"),
            Self::PostUpdated(_) => Cow::Borrowed("post_updated"),
            Self::PostDeleted(_) => Cow::Borrowed("post_deleted"),
            Self::CommentCreated(_) => Cow::Borrowed("comment_created"),
            Self::UserRegistered(_) => Cow::Borrowed("user_registered"),
            Self::UserLoggedIn { .. } => Cow::Borrowed("user_logged_in"),
            Self::MediaUploaded(_) => Cow::Borrowed("media_uploaded"),
            Self::MediaDeleted(_) => Cow::Borrowed("media_deleted"),
            Self::PasswordResetRequested { .. } => Cow::Borrowed("password_reset_requested"),
            Self::EmailVerificationRequested { .. } => {
                Cow::Borrowed("email_verification_requested")
            }
            Self::Custom { event_type, .. } => Cow::Owned(event_type.clone()),
        }
    }

    pub fn table(&self) -> Option<&'static str> {
        match self {
            Self::PostCreated(_) | Self::PostUpdated(_) | Self::PostDeleted(_) => Some("posts"),
            Self::CommentCreated(_) => Some("comments"),
            Self::UserRegistered(_) | Self::UserLoggedIn { .. } => Some("users"),
            Self::MediaUploaded(_) | Self::MediaDeleted(_) => Some("media"),
            Self::PasswordResetRequested { .. }
            | Self::EmailVerificationRequested { .. }
            | Self::Custom { .. } => None,
        }
    }

    pub fn phase(&self) -> Phase {
        Phase::After
    }

    pub fn display_name(&self) -> Cow<'static, str> {
        match self {
            Self::PostCreated(_) => Cow::Borrowed("PostCreated"),
            Self::PostUpdated(_) => Cow::Borrowed("PostUpdated"),
            Self::PostDeleted(_) => Cow::Borrowed("PostDeleted"),
            Self::CommentCreated(_) => Cow::Borrowed("CommentCreated"),
            Self::UserRegistered(_) => Cow::Borrowed("UserRegistered"),
            Self::UserLoggedIn { .. } => Cow::Borrowed("UserLoggedIn"),
            Self::MediaUploaded(_) => Cow::Borrowed("MediaUploaded"),
            Self::MediaDeleted(_) => Cow::Borrowed("MediaDeleted"),
            Self::PasswordResetRequested { .. } => Cow::Borrowed("PasswordResetRequested"),
            Self::EmailVerificationRequested { .. } => Cow::Borrowed("EmailVerificationRequested"),
            Self::Custom { event_type, .. } => Cow::Owned(event_type.clone()),
        }
    }
}
