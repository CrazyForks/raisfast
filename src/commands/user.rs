//! 用户相关 Command

use crate::models::user::{RegisteredVia, SocialLinks, UserMetadata};

/// 创建用户
pub struct CreateUserCmd {
    pub username: String,
    pub registered_via: RegisteredVia,
}

/// 更新用户资料
pub struct UpdateProfileCmd {
    pub id: i64,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
    pub social_links: Option<SocialLinks>,
    pub metadata: Option<UserMetadata>,
}
