//! 评论相关 Command

/// 创建评论
pub struct CreateCommentCmd {
    pub post_id: String,
    pub created_by: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub content: String,
    pub parent_id: Option<String>,
}
