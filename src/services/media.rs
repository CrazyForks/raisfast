//! 媒体文件服务。
//!
//! 处理文件上传、列表查询和删除等操作，包括类型校验、大小限制和磁盘 I/O。

use std::path::Path;

use crate::errors::app_error::{AppError, AppResult};
use crate::models::media;

/// 允许上传的 MIME 类型白名单。
const ALLOWED_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// 文件头 magic bytes 签名，用于校验实际上传内容是否为声明的图片类型。
const MAGIC_SIGNATURES: &[(&str, &[u8])] = &[
    ("image/jpeg", b"\xFF\xD8\xFF"),
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/gif", b"GIF87a"),
    ("image/gif", b"GIF89a"),
    ("image/webp", b"RIFF"),
];

/// 保存上传的媒体文件。
///
/// 1. 校验文件类型是否在 [`ALLOWED_TYPES`] 白名单内。
/// 2. 校验文件大小是否超过 `max_size`。
/// 3. 使用 UUID v7 生成文件名，写入磁盘。
/// 4. 在数据库中创建媒体记录。
pub async fn save_file(
    pool: &crate::db::Pool,
    user_id: &str,
    upload_dir: &str,
    max_size: usize,
    filename: &str,
    content_type: &str,
    data: &[u8],
) -> AppResult<media::Media> {
    if !ALLOWED_TYPES.contains(&content_type) {
        return Err(AppError::BadRequest("file_type_not_allowed".into()));
    }

    if !validate_magic_bytes(content_type, data) {
        return Err(AppError::BadRequest("file_type_not_allowed".into()));
    }

    if data.len() > max_size {
        return Err(AppError::BadRequest("file_too_large".into()));
    }

    let ext = match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    };

    let file_id = uuid::Uuid::now_v7().to_string();
    let stored_name = format!("{}.{}", file_id, ext);
    let dir = Path::new(upload_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to create upload dir: {}", e)))?;

    let filepath = dir.join(&stored_name);
    tokio::fs::write(&filepath, data)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to write file: {}", e)))?;

    media::create(
        pool,
        user_id,
        filename,
        &stored_name,
        content_type,
        data.len() as i64,
    )
    .await
}

/// 分页查询指定用户的媒体文件列表。
///
/// 返回媒体列表和总记录数。
pub async fn list(
    pool: &crate::db::Pool,
    user_id: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<media::Media>, i64)> {
    media::find_all(pool, user_id, page, page_size).await
}

/// 删除媒体文件。
///
/// 仅文件所有者或管理员可执行。同时删除磁盘文件和数据库记录。
pub async fn delete_media(
    pool: &crate::db::Pool,
    upload_dir: &str,
    media_id: &str,
    user_id: &str,
    role: &str,
) -> AppResult<()> {
    let m = media::find_by_id(pool, media_id)
        .await?
        .ok_or_else(|| AppError::NotFound("media".into()))?;

    if role != "admin" && m.user_id != user_id {
        return Err(AppError::Forbidden);
    }

    let filepath = Path::new(upload_dir).join(&m.filepath);
    let _ = tokio::fs::remove_file(filepath).await;

    media::delete(pool, media_id).await
}

/// 校验文件实际内容的 magic bytes 是否与声明的 Content-Type 匹配。
///
/// 防止攻击者伪造 `Content-Type` 上传非图片文件。
fn validate_magic_bytes(content_type: &str, data: &[u8]) -> bool {
    for (ct, magic) in MAGIC_SIGNATURES {
        if ct == &content_type && data.len() >= magic.len() && &data[..magic.len()] == *magic {
            return true;
        }
    }
    false
}
