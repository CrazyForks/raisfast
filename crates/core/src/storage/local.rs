//! Local filesystem storage backend.
//!
//! Files are written to `{upload_dir}/{key}`, served via `/uploads/{key}` static file serving.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;

/// Local filesystem storage.
#[derive(Debug)]
pub struct LocalStorage {
    base_dir: PathBuf,
    base_url: String,
}

impl LocalStorage {
    /// Create a local storage instance; automatically creates the root directory.
    pub fn new(upload_dir: &str, base_url: &str) -> AppResult<Self> {
        let base_dir = PathBuf::from(upload_dir);
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to create upload dir: {e}")))?;

        let base_url = base_url.trim_end_matches('/').to_string();
        tracing::info!(base_dir = %base_dir.display(), base_url = %base_url, "LocalStorage initialized");

        Ok(Self { base_dir, base_url })
    }
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(&self, key: &str, data: &[u8], _content_type: &str) -> AppResult<()> {
        let path = self.base_dir.join(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to create dir: {e}")))?;
        }
        tokio::fs::write(&path, data)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to write file: {e}")))?;
        tracing::debug!(key = %key, size = data.len(), "file stored locally");
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
        let path = self.base_dir.join(key);
        tokio::fs::read(&path)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to read file: {e}")))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let path = self.base_dir.join(key);
        let Err(e) = tokio::fs::remove_file(&path).await else {
            return Ok(());
        };
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(AppError::Internal(anyhow::anyhow!(
                "failed to delete file: {e}"
            )));
        }
        Ok(())
    }

    async fn url(&self, key: &str) -> AppResult<String> {
        Ok(format!("{}/uploads/{key}", self.base_url))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::config::app::AppConfig;
    use crate::storage::{self, Storage};

    fn setup_storage() -> (tempfile::TempDir, std::sync::Arc<dyn Storage>) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let base_url = "http://localhost:3000";
        let storage =
            crate::storage::local::LocalStorage::new(dir.path().to_str().unwrap(), base_url)
                .expect("create local storage");
        (dir, std::sync::Arc::new(storage))
    }

    fn setup_config(dir: &tempfile::TempDir) -> AppConfig {
        let mut config = AppConfig::test_defaults();
        config.upload_dir = dir.path().to_str().unwrap().to_string();
        config.base_url = "http://localhost:3000".into();
        config.storage_driver = "local".into();
        config
    }

    // ── LocalStorage basic CRUD ─────────────────────────────────────

    #[tokio::test]
    async fn local_put_and_get() {
        let (_dir, storage) = setup_storage();
        let key = "blog/2026/04/test.jpg";
        let data = b"\xFF\xD8\xFF\xE0test-image-data";

        storage
            .put(key, data, "image/jpeg")
            .await
            .expect("put should succeed");

        let result = storage.get(key).await.expect("get should succeed");
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn local_put_creates_subdirectories() {
        let (_dir, storage) = setup_storage();
        let key = "blog/2026/04/deep/nested/dir/file.txt";
        let data = b"hello";

        storage
            .put(key, data, "text/plain")
            .await
            .expect("put should create nested dirs");

        let result = storage.get(key).await.expect("get should succeed");
        assert_eq!(result, b"hello");
    }

    #[tokio::test]
    async fn local_put_overwrites_existing() {
        let (_dir, storage) = setup_storage();
        let key = "blog/test.txt";

        storage
            .put(key, b"version1", "text/plain")
            .await
            .expect("put v1");
        storage
            .put(key, b"version2", "text/plain")
            .await
            .expect("put v2");

        let result = storage.get(key).await.expect("get should succeed");
        assert_eq!(result, b"version2");
    }

    #[tokio::test]
    async fn local_delete_removes_file() {
        let (_dir, storage) = setup_storage();
        let key = "blog/to-delete.txt";

        storage.put(key, b"data", "text/plain").await.expect("put");
        storage.delete(key).await.expect("delete should succeed");

        let result = storage.get(key).await;
        assert!(result.is_err(), "file should be deleted");
    }

    #[tokio::test]
    async fn local_delete_nonexistent_is_ok() {
        let (_dir, storage) = setup_storage();
        storage
            .delete("nonexistent/file.txt")
            .await
            .expect("deleting nonexistent file should not error");
    }

    #[tokio::test]
    async fn local_url_format() {
        let (_dir, storage) = setup_storage();
        let url = storage
            .url("blog/2026/04/test.jpg")
            .await
            .expect("url should succeed");
        assert_eq!(url, "http://localhost:3000/uploads/blog/2026/04/test.jpg");
    }

    #[tokio::test]
    async fn local_url_no_trailing_slash_in_base() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let storage = crate::storage::local::LocalStorage::new(
            dir.path().to_str().unwrap(),
            "http://localhost:3000/",
        )
        .expect("create storage with trailing slash");
        let url = storage.url("test.txt").await.expect("url");
        assert_eq!(url, "http://localhost:3000/uploads/test.txt");
    }

    #[tokio::test]
    async fn local_get_nonexistent_returns_error() {
        let (_dir, storage) = setup_storage();
        let result = storage.get("does/not/exist.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn local_presigned_upload_returns_empty() {
        let (_dir, storage) = setup_storage();
        let url = storage
            .presigned_upload("test.txt", Duration::from_secs(300))
            .await
            .expect("presigned should succeed");
        assert!(url.is_empty());
    }

    // ── Concurrent multi-file writes ────────────────────────────────────────────────

    #[tokio::test]
    async fn local_concurrent_puts() {
        let (_dir, storage) = setup_storage();
        let mut handles = Vec::new();

        for i in 0..10 {
            let s = storage.clone();
            handles.push(tokio::spawn(async move {
                let key = format!("concurrent/file_{i}.txt");
                let data = format!("data-{i}").into_bytes();
                s.put(&key, &data, "text/plain").await.expect("put");
                let result = s.get(&key).await.expect("get");
                assert_eq!(result, data);
            }));
        }

        for h in handles {
            h.await.expect("task should complete");
        }
    }

    // ── Large file writes ────────────────────────────────────────────────────

    #[tokio::test]
    async fn local_large_file() {
        let (_dir, storage) = setup_storage();
        let data = vec![0xAB_u8; 10 * 1024 * 1024]; // 10 MB
        let key = "blog/large-file.bin";

        storage
            .put(key, &data, "application/octet-stream")
            .await
            .expect("put large file");

        let result = storage.get(key).await.expect("get large file");
        assert_eq!(result.len(), 10 * 1024 * 1024);
        assert!(result.iter().all(|&b| b == 0xAB));
    }

    // ── Empty files ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn local_empty_file() {
        let (_dir, storage) = setup_storage();
        let key = "blog/empty.txt";

        storage
            .put(key, b"", "text/plain")
            .await
            .expect("put empty");

        let result = storage.get(key).await.expect("get empty");
        assert!(result.is_empty());
    }

    // ── Special character keys ──────────────────────────────────────────────────

    #[tokio::test]
    async fn local_key_with_spaces_and_unicode() {
        let (_dir, storage) = setup_storage();
        let key = "blog/2026/04/你好世界 file name.txt";

        storage
            .put(key, b"unicode", "text/plain")
            .await
            .expect("put with unicode key");

        let result = storage.get(key).await.expect("get unicode key");
        assert_eq!(result, b"unicode");
    }

    // ── create_storage factory function ──────────────────────────────────────

    #[test]
    fn create_storage_local() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = setup_config(&dir);
        let storage = storage::create_storage(&config).expect("create local storage");

        let key = "factory/test.txt";
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            storage
                .put(key, b"factory-test", "text/plain")
                .await
                .expect("put");
            let data = storage.get(key).await.expect("get");
            assert_eq!(data, b"factory-test");
        });
    }

    #[test]
    fn create_storage_rejects_unknown_driver() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = setup_config(&dir);
        config.storage_driver = "ftp".into();

        let result = storage::create_storage(&config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown STORAGE_DRIVER: ftp"), "error: {err}");
    }

    #[test]
    fn create_storage_rejects_s3_without_feature() {
        #[cfg(not(feature = "storage-s3"))]
        {
            let dir = tempfile::tempdir().expect("temp dir");
            let mut config = setup_config(&dir);
            config.storage_driver = "s3".into();

            let result = storage::create_storage(&config);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("storage-s3 feature not enabled"),
                "error: {err}"
            );
        }
    }

    // ── storage_key generation format ─────────────────────────────────────────

    #[test]
    fn storage_key_format() {
        use crate::services::media::storage_key;
        let key = storage_key("blog", "jpg");
        assert!(key.starts_with("blog/"), "key: {key}");
        assert!(key.ends_with(".jpg"), "key: {key}");

        let parts: Vec<&str> = key.split('/').collect();
        assert_eq!(parts.len(), 4, "expected 4 parts: {key}");
        assert_eq!(parts[0], "blog");
        assert!(
            parts[1].len() == 4 && parts[1].parse::<u32>().is_ok(),
            "year part: {}",
            parts[1]
        );
        assert!(
            parts[2].len() == 2 && parts[2].parse::<u32>().is_ok(),
            "month part: {}",
            parts[2]
        );
    }

    // ── mime_to_ext mapping ─────────────────────────────────────────────

    #[test]
    fn mime_to_ext_mapping() {
        use crate::services::media::mime_to_ext;
        // Images
        assert_eq!(mime_to_ext("image/jpeg"), "jpg");
        assert_eq!(mime_to_ext("image/png"), "png");
        assert_eq!(mime_to_ext("image/gif"), "gif");
        assert_eq!(mime_to_ext("image/webp"), "webp");
        assert_eq!(mime_to_ext("image/svg+xml"), "svg");
        // Videos
        assert_eq!(mime_to_ext("video/mp4"), "mp4");
        assert_eq!(mime_to_ext("video/webm"), "webm");
        assert_eq!(mime_to_ext("video/quicktime"), "mov");
        // Audio
        assert_eq!(mime_to_ext("audio/mpeg"), "mp3");
        assert_eq!(mime_to_ext("audio/ogg"), "ogg");
        assert_eq!(mime_to_ext("audio/wav"), "wav");
        assert_eq!(mime_to_ext("audio/aac"), "aac");
        // Documents
        assert_eq!(mime_to_ext("application/pdf"), "pdf");
        assert_eq!(mime_to_ext("application/msword"), "doc");
        assert_eq!(
            mime_to_ext("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            "docx"
        );
        assert_eq!(mime_to_ext("application/vnd.ms-excel"), "xls");
        assert_eq!(
            mime_to_ext("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "xlsx"
        );
        // Archives
        assert_eq!(mime_to_ext("application/zip"), "zip");
        assert_eq!(mime_to_ext("application/x-tar"), "tar");
        assert_eq!(mime_to_ext("application/gzip"), "gz");
        // Text
        assert_eq!(mime_to_ext("text/plain"), "txt");
        assert_eq!(mime_to_ext("text/csv"), "csv");
        assert_eq!(mime_to_ext("text/markdown"), "md");
        // Unknown
        assert_eq!(mime_to_ext("application/unknown"), "bin");
    }

    // ── magic bytes validation ─────────────────────────────────────────────

    #[test]
    fn validate_magic_bytes_jpeg() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("image/jpeg", b"\xFF\xD8\xFF\xE0data"));
    }

    #[test]
    fn validate_magic_bytes_png() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("image/png", b"\x89PNG\r\n\x1a\ndata"));
    }

    #[test]
    fn validate_magic_bytes_gif87a() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("image/gif", b"GIF87a..."));
    }

    #[test]
    fn validate_magic_bytes_gif89a() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("image/gif", b"GIF89a..."));
    }

    #[test]
    fn validate_magic_bytes_webp() {
        use crate::services::media::validate_magic_bytes;
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"RIFF");
        data[8..12].copy_from_slice(b"WEBP");
        assert!(validate_magic_bytes("image/webp", &data));
    }

    #[test]
    fn validate_magic_bytes_mp4() {
        use crate::services::media::validate_magic_bytes;
        let mut data = vec![0u8; 32];
        let len = data.len() as u32;
        data[0..4].copy_from_slice(&len.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom");
        assert!(validate_magic_bytes("video/mp4", &data));
    }

    #[test]
    fn validate_magic_bytes_webm() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes(
            "video/webm",
            b"\x1a\x45\xdf\xa3\x00\x00"
        ));
    }

    #[test]
    fn validate_magic_bytes_pdf() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes(
            "application/pdf",
            b"%PDF-1.7 rest of document"
        ));
    }

    #[test]
    fn validate_magic_bytes_rejects_mismatch() {
        use crate::services::media::validate_magic_bytes;
        assert!(!validate_magic_bytes(
            "image/jpeg",
            b"\x89PNG\r\n\x1a\nfake"
        ));
        assert!(!validate_magic_bytes("image/png", b"\xFF\xD8\xFF\xE0fake"));
    }

    #[test]
    fn validate_magic_bytes_rejects_empty() {
        use crate::services::media::validate_magic_bytes;
        assert!(!validate_magic_bytes("image/jpeg", b""));
        assert!(!validate_magic_bytes("image/png", b""));
    }

    #[test]
    fn validate_magic_bytes_rejects_short_data() {
        use crate::services::media::validate_magic_bytes;
        assert!(!validate_magic_bytes("image/jpeg", b"\xFF"));
        assert!(!validate_magic_bytes("image/png", b"\x89"));
    }

    #[test]
    fn validate_magic_bytes_rejects_unknown_type() {
        use crate::services::media::validate_magic_bytes;
        assert!(!validate_magic_bytes(
            "application/x-executable",
            b"\x7FELF"
        ));
    }

    #[test]
    fn validate_magic_bytes_zip() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("application/zip", b"PK\x03\x04rest"));
    }

    #[test]
    fn validate_magic_bytes_gzip() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes(
            "application/gzip",
            b"\x1F\x8B\x08data"
        ));
    }

    #[test]
    fn validate_magic_bytes_rar() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes(
            "application/x-rar-compressed",
            b"Rar!\x1A\x07rest"
        ));
    }

    #[test]
    fn validate_magic_bytes_mp3_frame() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("audio/mpeg", b"\xFF\xFB\x90data"));
    }

    #[test]
    fn validate_magic_bytes_mp3_id3() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("audio/mpeg", b"ID3\x03tagdata"));
    }

    #[test]
    fn validate_magic_bytes_ogg() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("audio/ogg", b"OggS\x00data"));
    }

    #[test]
    fn validate_magic_bytes_wav() {
        use crate::services::media::validate_magic_bytes;
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(b"RIFF");
        data[8..12].copy_from_slice(b"WAVE");
        assert!(validate_magic_bytes("audio/wav", &data));
    }

    #[test]
    fn validate_magic_bytes_text_types_skip() {
        use crate::services::media::validate_magic_bytes;
        assert!(validate_magic_bytes("text/plain", b"hello"));
        assert!(validate_magic_bytes("text/csv", b"a,b,c"));
        assert!(validate_magic_bytes("text/markdown", b"# heading"));
        assert!(validate_magic_bytes("image/svg+xml", b"<svg>"));
    }

    #[test]
    fn validate_magic_bytes_text_types_reject_empty() {
        use crate::services::media::validate_magic_bytes;
        assert!(!validate_magic_bytes("text/plain", b""));
        assert!(!validate_magic_bytes("text/csv", b""));
    }

    // ── File type whitelist ────────────────────────────────────────────────

    #[test]
    fn allowed_types_includes_image_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"image/jpeg"));
        assert!(ALLOWED_TYPES.contains(&"image/png"));
        assert!(ALLOWED_TYPES.contains(&"image/gif"));
        assert!(ALLOWED_TYPES.contains(&"image/webp"));
        assert!(ALLOWED_TYPES.contains(&"image/svg+xml"));
    }

    #[test]
    fn allowed_types_includes_video_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"video/mp4"));
        assert!(ALLOWED_TYPES.contains(&"video/webm"));
        assert!(ALLOWED_TYPES.contains(&"video/quicktime"));
    }

    #[test]
    fn allowed_types_includes_audio_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"audio/mpeg"));
        assert!(ALLOWED_TYPES.contains(&"audio/ogg"));
        assert!(ALLOWED_TYPES.contains(&"audio/wav"));
        assert!(ALLOWED_TYPES.contains(&"audio/aac"));
    }

    #[test]
    fn allowed_types_includes_document_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"application/pdf"));
        assert!(ALLOWED_TYPES.contains(&"application/msword"));
        assert!(
            ALLOWED_TYPES.contains(
                &"application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )
        );
        assert!(ALLOWED_TYPES.contains(&"application/vnd.ms-excel"));
        assert!(
            ALLOWED_TYPES
                .contains(&"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        );
    }

    #[test]
    fn allowed_types_includes_archive_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"application/zip"));
        assert!(ALLOWED_TYPES.contains(&"application/x-tar"));
        assert!(ALLOWED_TYPES.contains(&"application/gzip"));
        assert!(ALLOWED_TYPES.contains(&"application/x-rar-compressed"));
    }

    #[test]
    fn allowed_types_includes_text_types() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(ALLOWED_TYPES.contains(&"text/plain"));
        assert!(ALLOWED_TYPES.contains(&"text/csv"));
        assert!(ALLOWED_TYPES.contains(&"text/markdown"));
    }

    #[test]
    fn allowed_types_excludes_executables() {
        use crate::services::media::ALLOWED_TYPES;
        assert!(!ALLOWED_TYPES.contains(&"application/x-executable"));
        assert!(!ALLOWED_TYPES.contains(&"application/x-sh"));
    }

    // ── put → get → delete full lifecycle ──────────────────────────────

    #[tokio::test]
    async fn local_lifecycle_put_get_delete() {
        let (_dir, storage) = setup_storage();
        let key = "lifecycle/test.txt";
        let data = b"lifecycle-data";

        storage.put(key, data, "text/plain").await.expect("put");

        let url = storage.url(key).await.expect("url");
        assert!(url.contains(key), "url should contain key: {url}");

        let fetched = storage.get(key).await.expect("get");
        assert_eq!(fetched, data);

        storage.delete(key).await.expect("delete");
        assert!(storage.get(key).await.is_err(), "should be deleted");
    }

    // ── Different bucket isolation ─────────────────────────────────────────────

    #[tokio::test]
    async fn local_different_buckets_isolated() {
        let (_dir, storage) = setup_storage();

        storage
            .put("blog/a.txt", b"blog-data", "text/plain")
            .await
            .expect("put blog");
        storage
            .put("avatar/b.txt", b"avatar-data", "text/plain")
            .await
            .expect("put avatar");
        storage
            .put("product/c.txt", b"product-data", "text/plain")
            .await
            .expect("put product");

        assert_eq!(
            storage.get("blog/a.txt").await.expect("get blog"),
            b"blog-data"
        );
        assert_eq!(
            storage.get("avatar/b.txt").await.expect("get avatar"),
            b"avatar-data"
        );
        assert_eq!(
            storage.get("product/c.txt").await.expect("get product"),
            b"product-data"
        );
    }

    // ── url with different keys ──────────────────────────────────────────────────

    #[tokio::test]
    async fn local_url_various_keys() {
        let (_dir, storage) = setup_storage();

        let u1 = storage.url("a.txt").await.expect("url");
        assert_eq!(u1, "http://localhost:3000/uploads/a.txt");

        let u2 = storage.url("blog/2026/04/img.jpg").await.expect("url");
        assert_eq!(u2, "http://localhost:3000/uploads/blog/2026/04/img.jpg");
    }
}
