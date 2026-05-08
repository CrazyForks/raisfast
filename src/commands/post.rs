//! 文章相关 Command

/// 创建文章
pub struct CreatePostCmd {
    pub title: String,
    pub slug: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: String,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub category_id: Option<i64>,
    pub tag_ids: Option<Vec<i64>>,
}

/// 更新文章
pub struct UpdatePostCmd {
    pub id: i64,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub category_id: Option<i64>,
    pub tag_ids: Option<Vec<i64>>,
    pub updated_by: Option<i64>,
}

/// 查询已发布文章
pub struct FindPublishedQuery {
    pub page: i64,
    pub page_size: i64,
    pub category_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub q: Option<String>,
}
