//! 媒体文件服务。
//!
//! 处理文件上传、列表查询和删除等操作。
//! 文件 I/O 通过 [`Storage`] trait 抽象，支持本地文件系统和 S3 兼容对象存储。

use chrono::Utc;

use crate::commands::CreateMediaCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::media;
use crate::repositories::MediaRepository;
use crate::storage::Storage;

/// 允许上传的 MIME 类型白名单。
pub(crate) const ALLOWED_TYPES: &[&str] = &[
    // 图片
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    // 视频
    "video/mp4",
    "video/webm",
    "video/quicktime",
    // 音频
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/aac",
    // 文档
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    // 压缩
    "application/zip",
    "application/x-tar",
    "application/gzip",
    "application/x-rar-compressed",
    // 文本
    "text/plain",
    "text/csv",
    "text/markdown",
];

/// 生成存储 key：`{bucket}/{year}/{month}/{uuid}.{ext}`
pub(crate) fn storage_key(bucket: &str, ext: &str) -> String {
    let now = Utc::now();
    let id = uuid::Uuid::now_v7();
    format!(
        "{}/{}/{:02}/{}.{}",
        bucket,
        now.format("%Y"),
        now.format("%m"),
        id,
        ext
    )
}

/// 从 MIME 类型推断文件扩展名。
pub(crate) fn mime_to_ext(content_type: &str) -> &'static str {
    match content_type {
        // 图片
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        // 视频
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        // 音频
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" => "wav",
        "audio/aac" => "aac",
        // 文档
        "application/pdf" => "pdf",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        // 压缩
        "application/zip" => "zip",
        "application/x-tar" => "tar",
        "application/gzip" => "gz",
        "application/x-rar-compressed" => "rar",
        // 文本
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/markdown" => "md",
        _ => "bin",
    }
}

/// 保存上传的媒体文件。
///
/// 1. 校验文件类型是否在白名单内。
/// 2. 校验文件大小。
/// 3. 通过 [`Storage`] trait 写入文件。
/// 4. 在数据库中创建媒体记录。
#[allow(clippy::too_many_arguments)]
pub async fn save_file(
    storage: &dyn Storage,
    media_repo: &dyn MediaRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    max_size: usize,
    bucket: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
) -> AppResult<media::Media> {
    let user_id = auth.ensure_authenticated()?;
    let tenant_id = auth.tenant_id();
    if !ALLOWED_TYPES.contains(&content_type) {
        tracing::warn!(content_type = %content_type, "file type not allowed");
        return Err(AppError::BadRequest("file_type_not_allowed".into()));
    }

    let detected_type = detect_mime_from_magic(data);
    let content_type = match detected_type {
        Some(detected) if detected != content_type => {
            tracing::info!(
                declared = %content_type,
                detected = %detected,
                "auto-correcting MIME type from file content"
            );
            detected
        }
        _ => content_type,
    };

    if !validate_magic_bytes(content_type, data) {
        tracing::warn!(content_type = %content_type, data_len = data.len(), "file content magic bytes mismatch");
        return Err(AppError::BadRequest("file_content_mismatch".into()));
    }

    if data.len() > max_size {
        tracing::warn!(data_len = data.len(), max_size, "file too large");
        return Err(AppError::BadRequest("file_too_large".into()));
    }

    let ext = mime_to_ext(content_type);
    let key = storage_key(bucket, ext);

    storage.put(&key, data, content_type).await?;

    let (width, height) = if content_type.starts_with("image/") {
        parse_image_dimensions(data)
    } else {
        (None, None)
    };

    let user = crate::models::user::find_by_id(pool, user_id, tenant_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let media = media_repo
        .create(
            CreateMediaCmd {
                user_id: user.id,
                filename: filename.to_string(),
                filepath: key.clone(),
                mimetype: content_type.to_string(),
                size: data.len() as i64,
                width,
                height,
            },
            tenant_id,
        )
        .await?;

    Ok(media)
}

/// 从图片二进制数据中解析宽高。
///
/// 仅读取图片头部信息，不解码整张图片，性能开销极小。
/// 解析失败时静默返回 `None`，不阻断上传流程。
fn parse_image_dimensions(data: &[u8]) -> (Option<i32>, Option<i32>) {
    match image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format() {
        Ok(reader) => match reader.into_dimensions() {
            Ok((w, h)) => (Some(w as i32), Some(h as i32)),
            Err(e) => {
                tracing::debug!(error = %e, "failed to parse image dimensions");
                (None, None)
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "failed to guess image format");
            (None, None)
        }
    }
}

/// 分页查询指定用户的媒体文件列表。
pub async fn list(
    media_repo: &dyn MediaRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<media::Media>, i64)> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;
    media_repo
        .find_all(user.id, page, page_size, auth.tenant_id())
        .await
}

pub async fn admin_list(
    media_repo: &dyn MediaRepository,
    page: i64,
    page_size: i64,
    auth: &AuthUser,
) -> AppResult<(Vec<media::Media>, i64)> {
    media_repo
        .find_all_admin(page, page_size, auth.tenant_id())
        .await
}

pub async fn admin_delete_media(
    storage: &dyn Storage,
    media_repo: &dyn MediaRepository,
    pool: &crate::db::Pool,
    media_id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let sql = format!(
        "SELECT id FROM media WHERE document_id = {}{}",
        crate::db::dialect::ph(1),
        crate::db::tenant::tenant_filter_ph(auth.tenant_id(), 2)
    );
    let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(media_id);
    if let Some(tid) = auth.tenant_id() {
        q = q.bind(tid);
    }
    let (media_pk,): (i64,) = q
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
        .ok_or_else(|| AppError::not_found("media"))?;

    let m = crate::models::media::find_by_id(pool, media_pk, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("media"))?;

    media_repo.delete(media_pk, auth.tenant_id()).await?;

    if let Err(e) = storage.delete(&m.filepath).await {
        tracing::warn!(key = %m.filepath, error = %e, "failed to delete file from storage");
    }

    Ok(())
}

/// 删除媒体文件。
///
/// 仅文件所有者或管理员可执行。先删除数据库记录，再通过 [`Storage`] 删除文件。
pub async fn delete_media(
    storage: &dyn Storage,
    media_repo: &dyn MediaRepository,
    pool: &crate::db::Pool,
    media_id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;

    let sql = format!(
        "SELECT id FROM media WHERE document_id = {}{}",
        crate::db::dialect::ph(1),
        crate::db::tenant::tenant_filter_ph(auth.tenant_id(), 2)
    );
    let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(media_id);
    if let Some(tid) = auth.tenant_id() {
        q = q.bind(tid);
    }
    let (media_pk,): (i64,) = q
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
        .ok_or_else(|| AppError::not_found("media"))?;

    let m = crate::models::media::find_by_id(pool, media_pk, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("media"))?;

    if !auth.is_admin() && m.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    media_repo.delete(media_pk, auth.tenant_id()).await?;

    if let Err(e) = storage.delete(&m.filepath).await {
        tracing::warn!(key = %m.filepath, error = %e, "failed to delete file from storage");
    }

    Ok(())
}

/// 获取存储统计
pub async fn stats(
    media_repo: &dyn MediaRepository,
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<media::MediaStats> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, auth.tenant_id())
        .await?
        .ok_or(AppError::Unauthorized)?;
    media_repo.stats(user.id, auth.tenant_id()).await
}

/// 校验文件实际内容的 magic bytes 是否与声明的 Content-Type 匹配。
///
/// 对无法通过 magic bytes 校验的类型（纯文本、SVG、Office Open XML、tar），
/// 只做最小长度检查后直接放行。
pub(crate) fn validate_magic_bytes(content_type: &str, data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }

    // 无法可靠校验 magic bytes 的类型：仅检查非空
    const SKIP_MAGIC_TYPES: &[&str] = &[
        "text/plain",
        "text/csv",
        "text/markdown",
        "image/svg+xml",
        "application/x-tar",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/vnd.ms-excel",
        "application/msword",
        "application/vnd.ms-powerpoint",
        "audio/aac",
        "video/quicktime",
    ];
    if SKIP_MAGIC_TYPES.contains(&content_type) {
        return true;
    }

    // 图片：宽松校验前缀
    if content_type == "image/jpeg" && data.len() >= 2 {
        return &data[0..2] == b"\xFF\xD8";
    }
    if content_type == "image/png" && data.len() >= 8 {
        return &data[0..8] == b"\x89PNG\r\n\x1a\n";
    }
    if content_type == "image/gif" && data.len() >= 6 {
        return &data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a";
    }
    if content_type == "image/webp" && data.len() >= 12 {
        return &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP";
    }

    // 视频
    if content_type == "video/mp4" && data.len() >= 8 {
        let len = u32::from_be_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
        if len > 0 && len <= data.len() && &data[4..8] == b"ftyp" {
            return true;
        }
    }
    if content_type == "video/webm" && data.len() >= 4 {
        return &data[0..4] == b"\x1a\x45\xdf\xa3";
    }

    // 音频
    if content_type == "audio/mpeg" && data.len() >= 3 {
        return data.starts_with(b"\xFF\xFB")
            || data.starts_with(b"\xFF\xF3")
            || data.starts_with(b"\xFF\xF2")
            || data.starts_with(b"ID3");
    }
    if content_type == "audio/ogg" && data.len() >= 4 {
        return &data[0..4] == b"OggS";
    }
    if content_type == "audio/wav" && data.len() >= 12 {
        return &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE";
    }

    // 文档
    if content_type == "application/pdf" && data.len() >= 5 {
        return &data[0..5] == b"%PDF-";
    }

    // 压缩
    if content_type == "application/zip" && data.len() >= 4 {
        return &data[0..4] == b"PK\x03\x04";
    }
    if content_type == "application/gzip" && data.len() >= 2 {
        return &data[0..2] == b"\x1F\x8B";
    }
    if content_type == "application/x-rar-compressed" && data.len() >= 6 {
        return &data[0..6] == b"Rar!\x1A\x07";
    }

    false
}

/// 从文件内容 magic bytes 推断真实 MIME 类型。
fn detect_mime_from_magic(data: &[u8]) -> Option<&'static str> {
    if data.len() < 2 {
        return None;
    }
    // 图片
    if &data[0..2] == b"\xFF\xD8" {
        return Some("image/jpeg");
    }
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // 压缩
    if data.len() >= 4 && &data[0..4] == b"PK\x03\x04" {
        return Some("application/zip");
    }
    if data.len() >= 2 && &data[0..2] == b"\x1F\x8B" {
        return Some("application/gzip");
    }
    if data.len() >= 6 && &data[0..6] == b"Rar!\x1A\x07" {
        return Some("application/x-rar-compressed");
    }
    // 文档
    if data.len() >= 5 && &data[0..5] == b"%PDF-" {
        return Some("application/pdf");
    }
    // 音频
    if data.len() >= 3
        && (data.starts_with(b"\xFF\xFB")
            || data.starts_with(b"\xFF\xF3")
            || data.starts_with(b"\xFF\xF2")
            || data.starts_with(b"ID3"))
    {
        return Some("audio/mpeg");
    }
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        return Some("audio/ogg");
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    // 视频
    if data.len() >= 8 {
        let len = u32::from_be_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
        if len > 0 && len <= data.len() && &data[4..8] == b"ftyp" {
            return Some("video/mp4");
        }
    }
    if data.len() >= 4 && &data[0..4] == b"\x1a\x45\xdf\xa3" {
        return Some("video/webm");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_to_ext_known_types() {
        assert_eq!(mime_to_ext("image/jpeg"), "jpg");
        assert_eq!(mime_to_ext("image/png"), "png");
        assert_eq!(mime_to_ext("image/gif"), "gif");
        assert_eq!(mime_to_ext("image/webp"), "webp");
        assert_eq!(mime_to_ext("image/svg+xml"), "svg");
        assert_eq!(mime_to_ext("video/mp4"), "mp4");
        assert_eq!(mime_to_ext("audio/mpeg"), "mp3");
        assert_eq!(mime_to_ext("application/pdf"), "pdf");
        assert_eq!(mime_to_ext("application/zip"), "zip");
        assert_eq!(mime_to_ext("text/plain"), "txt");
        assert_eq!(mime_to_ext("text/markdown"), "md");
        assert_eq!(mime_to_ext("text/csv"), "csv");
    }

    #[test]
    fn mime_to_ext_unknown_returns_bin() {
        assert_eq!(mime_to_ext("application/x-unknown"), "bin");
        assert_eq!(mime_to_ext("foo/bar"), "bin");
    }

    #[test]
    fn validate_magic_bytes_empty_data() {
        assert!(!validate_magic_bytes("image/jpeg", &[]));
    }

    #[test]
    fn validate_magic_bytes_jpeg() {
        assert!(validate_magic_bytes("image/jpeg", b"\xFF\xD8\xFF\xE0"));
        assert!(!validate_magic_bytes("image/jpeg", b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn validate_magic_bytes_png() {
        let png_header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert!(validate_magic_bytes("image/png", png_header));
        assert!(!validate_magic_bytes("image/png", b"\xFF\xD8\xFF"));
    }

    #[test]
    fn validate_magic_bytes_gif() {
        assert!(validate_magic_bytes("image/gif", b"GIF89a\x00\x00"));
        assert!(validate_magic_bytes("image/gif", b"GIF87a\x00\x00"));
        assert!(!validate_magic_bytes("image/gif", b"BM\x00\x00\x00"));
    }

    #[test]
    fn validate_magic_bytes_webp() {
        let mut webp = vec![b'R', b'I', b'F', b'F', 0, 0, 0, 0, b'W', b'E', b'B', b'P'];
        webp.extend_from_slice(&[0; 20]);
        assert!(validate_magic_bytes("image/webp", &webp));
        assert!(!validate_magic_bytes("image/webp", b"\xFF\xD8\xFF"));
    }

    #[test]
    fn validate_magic_bytes_pdf() {
        assert!(validate_magic_bytes("application/pdf", b"%PDF-1.4\n"));
        assert!(!validate_magic_bytes("application/pdf", b"<html>"));
    }

    #[test]
    fn validate_magic_bytes_zip() {
        assert!(validate_magic_bytes(
            "application/zip",
            b"PK\x03\x04\x00\x00"
        ));
        assert!(!validate_magic_bytes("application/zip", b"\x1F\x8B\x08"));
    }

    #[test]
    fn validate_magic_bytes_gzip() {
        assert!(validate_magic_bytes(
            "application/gzip",
            b"\x1F\x8B\x08\x00"
        ));
        assert!(!validate_magic_bytes("application/gzip", b"PK\x03\x04"));
    }

    #[test]
    fn validate_magic_bytes_rar() {
        assert!(validate_magic_bytes(
            "application/x-rar-compressed",
            b"Rar!\x1A\x07\x00"
        ));
        assert!(!validate_magic_bytes(
            "application/x-rar-compressed",
            b"PK\x03\x04"
        ));
    }

    #[test]
    fn validate_magic_bytes_mp3() {
        assert!(validate_magic_bytes("audio/mpeg", b"\xFF\xFB\x90\x00"));
        assert!(validate_magic_bytes("audio/mpeg", b"ID3\x03\x00"));
        assert!(!validate_magic_bytes("audio/mpeg", b"OggS\x00"));
    }

    #[test]
    fn validate_magic_bytes_ogg() {
        assert!(validate_magic_bytes("audio/ogg", b"OggS\x00\x02"));
        assert!(!validate_magic_bytes("audio/ogg", b"\xFF\xFB"));
    }

    #[test]
    fn validate_magic_bytes_wav() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0, 0, 0, 0]);
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(&[0; 20]);
        assert!(validate_magic_bytes("audio/wav", &wav));
        assert!(!validate_magic_bytes("audio/wav", b"\xFF\xD8\xFF"));
    }

    #[test]
    fn validate_magic_bytes_mp4() {
        let mut mp4 = vec![0, 0, 0, 0x20];
        mp4.extend_from_slice(b"ftypisom");
        mp4.extend_from_slice(&[0; 24]);
        assert!(validate_magic_bytes("video/mp4", &mp4));
        assert!(!validate_magic_bytes("video/mp4", b"\x1a\x45\xdf\xa3"));
    }

    #[test]
    fn validate_magic_bytes_webm() {
        assert!(validate_magic_bytes(
            "video/webm",
            b"\x1a\x45\xdf\xa3\x00\x00"
        ));
        assert!(!validate_magic_bytes("video/webm", b"\xFF\xD8\xFF"));
    }

    #[test]
    fn validate_magic_bytes_skip_types_always_pass() {
        assert!(validate_magic_bytes("text/plain", b"hello"));
        assert!(validate_magic_bytes("text/csv", b"a,b"));
        assert!(validate_magic_bytes("text/markdown", b"# Title"));
        assert!(validate_magic_bytes("image/svg+xml", b"<svg>"));
        assert!(validate_magic_bytes("application/x-tar", b"foo"));
    }

    #[test]
    fn validate_magic_bytes_unknown_type_fails() {
        assert!(!validate_magic_bytes(
            "application/x-totally-fake",
            b"\x00\x01\x02"
        ));
    }

    #[test]
    fn detect_mime_from_magic_jpeg() {
        assert_eq!(
            detect_mime_from_magic(b"\xFF\xD8\xFF\xE0"),
            Some("image/jpeg")
        );
    }

    #[test]
    fn detect_mime_from_magic_png() {
        assert_eq!(
            detect_mime_from_magic(b"\x89PNG\r\n\x1a\n\x00"),
            Some("image/png")
        );
    }

    #[test]
    fn detect_mime_from_magic_gif() {
        assert_eq!(
            detect_mime_from_magic(b"GIF89a\x00\x00\x00"),
            Some("image/gif")
        );
    }

    #[test]
    fn detect_mime_from_magic_pdf() {
        assert_eq!(
            detect_mime_from_magic(b"%PDF-1.4\n"),
            Some("application/pdf")
        );
    }

    #[test]
    fn detect_mime_from_magic_zip() {
        assert_eq!(
            detect_mime_from_magic(b"PK\x03\x04\x00"),
            Some("application/zip")
        );
    }

    #[test]
    fn detect_mime_from_magic_unknown() {
        assert_eq!(detect_mime_from_magic(b"\x00\x01"), None);
        assert_eq!(detect_mime_from_magic(b"\x00"), None);
    }

    #[test]
    fn storage_key_format() {
        let key = storage_key("uploads", "jpg");
        assert!(key.starts_with("uploads/"));
        assert!(key.ends_with(".jpg"));
        let parts: Vec<&str> = key.split('/').collect();
        assert_eq!(parts.len(), 4);
        assert!(parts[1].len() == 4);
        assert!(parts[2].len() == 2);
    }

    #[test]
    fn parse_image_dimensions_invalid_data() {
        let (w, h) = parse_image_dimensions(b"not an image");
        assert_eq!(w, None);
        assert_eq!(h, None);
    }

    #[test]
    fn parse_image_dimensions_empty() {
        let (w, h) = parse_image_dimensions(&[]);
        assert_eq!(w, None);
        assert_eq!(h, None);
    }
}
