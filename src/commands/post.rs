//! 文章相关 Command

/// 创建文章
pub struct CreatePostCmd {
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub created_by: String,
    pub updated_by: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
}

/// 更新文章
pub struct UpdatePostCmd {
    pub id: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_ids: Option<Vec<String>>,
    pub updated_by: Option<String>,
}

/// 查询已发布文章
pub struct FindPublishedQuery {
    pub page: i64,
    pub page_size: i64,
    pub category_id: Option<String>,
    pub tag_id: Option<String>,
    pub q: Option<String>,
}
