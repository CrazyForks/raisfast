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
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub status: String,
    pub author_id: String,
    pub cover_image: Option<String>,
}

pub struct UpdatePageCmd {
    pub id: String,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
    pub blocks: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_image: Option<String>,
    pub template: Option<String>,
    pub parent_id: Option<Option<String>>,
    pub sort_order: Option<i64>,
    pub status: Option<String>,
    pub cover_image: Option<String>,
}
