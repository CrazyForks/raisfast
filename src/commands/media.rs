//! Media file-related commands

/// Create a media file record
pub struct CreateMediaCmd {
    pub user_id: i64,
    pub filename: String,
    pub filepath: String,
    pub mimetype: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
}
