//! 媒体文件相关 Command

/// 创建媒体文件记录
pub struct CreateMediaCmd {
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
}
