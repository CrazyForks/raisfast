//! User-related commands

use crate::models::user::{RegisteredVia, SocialLinks, UserMetadata};

/// Create a user
pub struct CreateUserCmd {
    pub username: String,
    pub registered_via: RegisteredVia,
}

/// Update user profile
pub struct UpdateProfileCmd {
    pub id: i64,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
    pub social_links: Option<SocialLinks>,
    pub metadata: Option<UserMetadata>,
}
