//! User-related commands

use crate::models::user::{RegisteredVia, SocialLinks, UserMetadata};
use crate::types::snowflake_id::SnowflakeId;

/// Create a user
pub struct CreateUserCmd {
    pub username: String,
    pub registered_via: RegisteredVia,
}

impl CreateUserCmd {
    pub fn new(username: impl Into<String>, registered_via: RegisteredVia) -> Self {
        Self {
            username: username.into(),
            registered_via,
        }
    }
}

/// Update user profile
pub struct UpdateProfileCmd {
    pub id: SnowflakeId,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
    pub social_links: Option<SocialLinks>,
    pub metadata: Option<UserMetadata>,
}
