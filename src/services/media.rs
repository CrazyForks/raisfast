//! 媒体文件服务。
//!
//! 处理文件上传、列表查询和删除等操作，包括类型校验、大小限制和磁盘 I/O。

use std::path::Path;

use crate::commands::CreateMediaCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::models::media;
use crate::repositories::MediaRepository;

/// 允许上传的 MIME 类型白名单。
const ALLOWED_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// 文件头 magic bytes 签名，用于校验实际上传内容是否为声明的图片类型。
const MAGIC_SIGNATURES: &[(&str, &[u8])] = &[
    ("image/jpeg", b"\xFF\xD8\xFF"),
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/gif", b"GIF87a"),
    ("image/gif", b"GIF89a"),
];

/// 保存上传的媒体文件。
///
/// 1. 校验文件类型是否在 [`ALLOWED_TYPES`] 白名单内。
/// 2. 校验文件大小是否超过 `max_size`。
/// 3. 使用 UUID v7 生成文件名，写入磁盘。
/// 4. 在数据库中创建媒体记录。
#[allow(clippy::too_many_arguments)]
pub async fn save_file(
    media_repo: &dyn MediaRepository,
    user_id: &str,
    upload_dir: &str,
    max_size: usize,
    filename: &str,
    content_type: &str,
    data: &[u8],
    tenant_id: Option<&str>,
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
    let stored_name = format!("{file_id}.{ext}");
    let dir = Path::new(upload_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to create upload dir: {e}")))?;

    let filepath = dir.join(&stored_name);
    tokio::fs::write(&filepath, data)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to write file: {e}")))?;

    media_repo
        .create(
            CreateMediaCmd {
                user_id: user_id.to_string(),
                filename: filename.to_string(),
                filepath: stored_name,
                mimetype: content_type.to_string(),
                size: data.len() as i64,
            },
            tenant_id,
        )
        .await
}

/// 分页查询指定用户的媒体文件列表。
///
/// 返回媒体列表和总记录数。
pub async fn list(
    media_repo: &dyn MediaRepository,
    user_id: &str,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<media::Media>, i64)> {
    media_repo
        .find_all(user_id, page, page_size, tenant_id)
        .await
}

/// 删除媒体文件。
///
/// 仅文件所有者或管理员可执行。先删除数据库记录，再删除磁盘文件。
/// 若磁盘文件删除失败，仅记录日志不阻断（可后续手动清理）。
pub async fn delete_media(
    media_repo: &dyn MediaRepository,
    upload_dir: &str,
    media_id: &str,
    user_id: &str,
    role: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let m = media_repo
        .find_by_id(media_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("media"))?;

    crate::utils::auth::require_owner_or_admin(role, user_id, &m.user_id)?;

    media_repo.delete(media_id, tenant_id).await?;

    let filepath = Path::new(upload_dir).join(&m.filepath);
    if let Err(e) = tokio::fs::remove_file(&filepath).await {
        tracing::warn!(path = %filepath.display(), error = %e, "failed to delete media file from disk");
    }

    Ok(())
}

/// 校验文件实际内容的 magic bytes 是否与声明的 Content-Type 匹配。
///
/// 防止攻击者伪造 `Content-Type` 上传非图片文件。
/// WebP 格式特殊处理：校验 bytes 0-3 = `RIFF` + bytes 8-11 = `WEBP`。
fn validate_magic_bytes(content_type: &str, data: &[u8]) -> bool {
    for (ct, magic) in MAGIC_SIGNATURES {
        if ct == &content_type && data.len() >= magic.len() && &data[..magic.len()] == *magic {
            return true;
        }
    }
    if content_type == "image/webp" && data.len() >= 12 {
        return &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP";
    }
    false
}
