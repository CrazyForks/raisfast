//! 分类相关 Command

/// 创建分类
pub struct CreateCategoryCmd {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: i64,
}

/// 更新分类
pub struct UpdateCategoryCmd {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: Option<i64>,
}
