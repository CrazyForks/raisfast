//! 页面相关 Command

pub struct CreatePageCmd {
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: String,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub status: String,
    pub created_by: i64,
    pub updated_by: Option<i64>,
    pub cover_image: Option<String>,
}

pub struct UpdatePageCmd {
    pub id: i64,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub sort_order: Option<i64>,
    pub status: Option<String>,
    pub cover_image: Option<String>,
    pub updated_by: Option<i64>,
}
