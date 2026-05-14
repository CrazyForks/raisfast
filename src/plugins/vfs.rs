//! Plugin virtual filesystem
//!
//! Each plugin has an isolated sandbox directory; virtual paths map under `{vfs_root}/{plugin_id}/`.
//! Provides path escape protection, file size limits, and total quota limits.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::app::AppConfig;
use crate::plugins::Permissions;

/// Virtual filesystem errors
#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    /// Path contains `..` or other escape attempts
    #[error("path escape detected")]
    PathEscape,
    /// Exceeds single file size limit
    #[error("file exceeds max size ({max} bytes)")]
    FileTooLarge { max: usize },
    /// Exceeds total storage quota
    #[error("quota exceeded (used {used}/{max}, need {need})")]
    QuotaExceeded {
        used: usize,
        max: usize,
        need: usize,
    },
    /// Permission denied (read-only / no access)
    #[error("filesystem permission denied")]
    PermissionDenied,
    /// IO error
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Virtual filesystem instance
///
/// One instance per plugin, bound to the `{vfs_root}/{plugin_id}/` directory.
pub struct VirtualFs {
    root: PathBuf,
    max_file_size: usize,
    max_total_size: usize,
    can_write: bool,
    can_read: bool,
}

impl VirtualFs {
    /// Create a VFS instance from config and permissions
    ///
    /// Automatically creates the sandbox root directory.
    #[must_use]
    pub fn new(config: &AppConfig, plugin_id: &str, permissions: &Permissions) -> Self {
        let root = PathBuf::from(&config.plugin_vfs_root).join(plugin_id);
        let can_read = permissions
            .filesystem
            .iter()
            .any(|p| p == "read" || p == "read-write" || p == "*");
        let can_write = permissions
            .filesystem
            .iter()
            .any(|p| p == "write" || p == "read-write" || p == "*");
        Self {
            root,
            max_file_size: config.plugin_vfs_max_file_size,
            max_total_size: config.plugin_vfs_max_total_size,
            can_read,
            can_write,
        }
    }

    /// Resolve virtual path to real path, detecting escapes
    fn resolve(&self, path: &str) -> Result<PathBuf, VfsError> {
        let cleaned = path.replace('\\', "/");
        let parts: Vec<&str> = cleaned.split('/').filter(|s| !s.is_empty()).collect();
        for part in &parts {
            if *part == ".." {
                return Err(VfsError::PathEscape);
            }
        }
        if parts.is_empty() {
            return Err(VfsError::PathEscape);
        }
        let virtual_path = parts.join("/");
        Ok(self.root.join(&virtual_path))
    }

    /// Calculate current used storage size
    fn used_size(&self) -> usize {
        dir_size(&self.root).unwrap_or(0)
    }

    /// Read file contents
    pub fn read_file(&self, path: &str) -> Result<String, VfsError> {
        if !self.can_read {
            return Err(VfsError::PermissionDenied);
        }
        let real = self.resolve(path)?;
        fs::read_to_string(&real).map_err(VfsError::Io)
    }

    /// Write file (auto-creates intermediate directories)
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), VfsError> {
        if !self.can_write {
            return Err(VfsError::PermissionDenied);
        }
        let content_len = content.len();
        if content_len > self.max_file_size {
            return Err(VfsError::FileTooLarge {
                max: self.max_file_size,
            });
        }
        let real = self.resolve(path)?;
        let used = self.used_size();
        let existing_len = fs::metadata(&real).map(|m| m.len() as usize).unwrap_or(0);
        let new_used = used.saturating_sub(existing_len) + content_len;
        if new_used > self.max_total_size {
            return Err(VfsError::QuotaExceeded {
                used,
                max: self.max_total_size,
                need: content_len,
            });
        }
        if let Some(parent) = real.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&real, content)?;
        Ok(())
    }

    /// Delete file
    pub fn delete_file(&self, path: &str) -> Result<(), VfsError> {
        if !self.can_write {
            return Err(VfsError::PermissionDenied);
        }
        let real = self.resolve(path)?;
        if real.exists() {
            fs::remove_file(&real)?;
        }
        Ok(())
    }

    /// Check if a file exists
    pub fn exists(&self, path: &str) -> Result<bool, VfsError> {
        if !self.can_read {
            return Err(VfsError::PermissionDenied);
        }
        let real = self.resolve(path)?;
        Ok(real.exists())
    }

    /// List files and subdirectories under a directory
    pub fn list_dir(&self, path: &str) -> Result<Vec<String>, VfsError> {
        if !self.can_read {
            return Err(VfsError::PermissionDenied);
        }
        let dir_path = if path.is_empty() || path == "/" {
            self.root.clone()
        } else {
            self.resolve(path)?
        };
        if !dir_path.exists() {
            return Ok(vec![]);
        }
        let mut entries = Vec::new();
        let entries_iter = fs::read_dir(&dir_path)?;
        for entry in entries_iter {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                entries.push(format!("{name}/"));
            } else {
                entries.push(name);
            }
        }
        entries.sort();
        Ok(entries)
    }

    /// Get file metadata
    pub fn stat(&self, path: &str) -> Result<VfsFileInfo, VfsError> {
        if !self.can_read {
            return Err(VfsError::PermissionDenied);
        }
        let real = self.resolve(path)?;
        let meta = fs::metadata(&real)?;
        Ok(VfsFileInfo {
            size: meta.len() as usize,
            is_dir: meta.is_dir(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        })
    }

    /// Return sandbox root directory path
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// File metadata
#[derive(Debug, serde::Serialize)]
pub struct VfsFileInfo {
    pub size: usize,
    pub is_dir: bool,
    pub modified: Option<u64>,
}

/// Recursively calculate directory size
fn dir_size(path: &Path) -> Result<usize, std::io::Error> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0usize;
    dir_size_recursive(path, &mut total)?;
    Ok(total)
}

fn dir_size_recursive(path: &Path, total: &mut usize) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            dir_size_recursive(&entry.path(), total)?;
        } else {
            *total += meta.len() as usize;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_test_config() -> Arc<AppConfig> {
        let mut config = AppConfig::test_defaults();
        config.plugin_vfs_root = tempfile::tempdir()
            .unwrap()
            .path()
            .to_string_lossy()
            .to_string();
        config.plugin_vfs_max_file_size = 1024;
        config.plugin_vfs_max_total_size = 4096;
        Arc::new(config)
    }

    fn make_rw_perms() -> Permissions {
        Permissions {
            filesystem: vec!["read-write".into()],
            ..Permissions::default()
        }
    }

    fn make_ro_perms() -> Permissions {
        Permissions {
            filesystem: vec!["read".into()],
            ..Permissions::default()
        }
    }

    fn make_wo_perms() -> Permissions {
        Permissions {
            filesystem: vec!["write".into()],
            ..Permissions::default()
        }
    }

    #[test]
    fn write_and_read_roundtrip() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("hello.txt", "hello world").unwrap();
        let content = vfs.read_file("hello.txt").unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn write_creates_subdirectories() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("templates/index.html", "<h1>Hi</h1>")
            .unwrap();
        let content = vfs.read_file("templates/index.html").unwrap();
        assert_eq!(content, "<h1>Hi</h1>");
    }

    #[test]
    fn path_escape_blocked() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        assert!(vfs.resolve("../etc/passwd").is_err());
        assert!(vfs.resolve("foo/../../bar").is_err());
        assert!(vfs.write_file("../escape.txt", "nope").is_err());
    }

    #[test]
    fn file_too_large() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        let big = "x".repeat(2048);
        let result = vfs.write_file("big.txt", &big);
        assert!(matches!(result, Err(VfsError::FileTooLarge { .. })));
    }

    #[test]
    fn quota_exceeded() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("a.txt", &"a".repeat(1000)).unwrap();
        vfs.write_file("b.txt", &"b".repeat(1000)).unwrap();
        vfs.write_file("c.txt", &"c".repeat(1000)).unwrap();
        vfs.write_file("d.txt", &"d".repeat(1000)).unwrap();
        let result = vfs.write_file("e.txt", &"e".repeat(100));
        match &result {
            Err(VfsError::QuotaExceeded { .. }) => {}
            other => panic!("expected QuotaExceeded, got: {other:?}"),
        }
    }

    #[test]
    fn overwrite_within_quota() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("data.txt", &"x".repeat(1000)).unwrap();
        vfs.write_file("data.txt", &"y".repeat(500)).unwrap();
        assert_eq!(vfs.read_file("data.txt").unwrap(), "y".repeat(500));
    }

    #[test]
    fn delete_file() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("temp.txt", "temp").unwrap();
        assert!(vfs.exists("temp.txt").unwrap());
        vfs.delete_file("temp.txt").unwrap();
        assert!(!vfs.exists("temp.txt").unwrap());
    }

    #[test]
    fn delete_nonexistent_ok() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        assert!(vfs.delete_file("nope.txt").is_ok());
    }

    #[test]
    fn exists_check() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        assert!(!vfs.exists("missing.txt").unwrap());
        vfs.write_file("found.txt", "yes").unwrap();
        assert!(vfs.exists("found.txt").unwrap());
    }

    #[test]
    fn list_dir_contents() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("a.txt", "a").unwrap();
        vfs.write_file("b.txt", "b").unwrap();
        vfs.write_file("sub/c.txt", "c").unwrap();
        let entries = vfs.list_dir("").unwrap();
        assert!(entries.contains(&"a.txt".to_string()));
        assert!(entries.contains(&"b.txt".to_string()));
        assert!(entries.contains(&"sub/".to_string()));
    }

    #[test]
    fn list_nonexistent_dir_returns_empty() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        let entries = vfs.list_dir("nonexistent").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn stat_file() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "test-plugin", &make_rw_perms());
        vfs.write_file("info.txt", "hello").unwrap();
        let info = vfs.stat("info.txt").unwrap();
        assert_eq!(info.size, 5);
        assert!(!info.is_dir);
        assert!(info.modified.is_some());
    }

    #[test]
    fn read_without_permission_denied() {
        let config = make_test_config();
        let vfs_rw = VirtualFs::new(&config, "p1", &make_rw_perms());
        vfs_rw.write_file("secret.txt", "data").unwrap();
        let vfs_ro = VirtualFs::new(&config, "p1", &make_wo_perms());
        assert!(matches!(
            vfs_ro.read_file("secret.txt"),
            Err(VfsError::PermissionDenied)
        ));
    }

    #[test]
    fn write_without_permission_denied() {
        let config = make_test_config();
        let vfs = VirtualFs::new(&config, "p1", &make_ro_perms());
        assert!(matches!(
            vfs.write_file("file.txt", "data"),
            Err(VfsError::PermissionDenied)
        ));
    }

    #[test]
    fn delete_without_permission_denied() {
        let config = make_test_config();
        let vfs_rw = VirtualFs::new(&config, "p1", &make_rw_perms());
        vfs_rw.write_file("f.txt", "x").unwrap();
        let vfs_ro = VirtualFs::new(&config, "p1", &make_ro_perms());
        assert!(matches!(
            vfs_ro.delete_file("f.txt"),
            Err(VfsError::PermissionDenied)
        ));
    }

    #[test]
    fn wildcard_permission_allows_all() {
        let config = make_test_config();
        let perms = Permissions {
            filesystem: vec!["*".into()],
            ..Permissions::default()
        };
        let vfs = VirtualFs::new(&config, "p1", &perms);
        vfs.write_file("w.txt", "ok").unwrap();
        assert_eq!(vfs.read_file("w.txt").unwrap(), "ok");
    }

    #[test]
    fn no_filesystem_permission_blocks_all() {
        let config = make_test_config();
        let vfs_rw = VirtualFs::new(&config, "p1", &make_rw_perms());
        vfs_rw.write_file("f.txt", "x").unwrap();
        let vfs_none = VirtualFs::new(&config, "p1", &Permissions::default());
        assert!(matches!(
            vfs_none.read_file("f.txt"),
            Err(VfsError::PermissionDenied)
        ));
        assert!(matches!(
            vfs_none.write_file("f.txt", "y"),
            Err(VfsError::PermissionDenied)
        ));
        assert!(matches!(
            vfs_none.exists("f.txt"),
            Err(VfsError::PermissionDenied)
        ));
        assert!(matches!(
            vfs_none.list_dir(""),
            Err(VfsError::PermissionDenied)
        ));
        assert!(matches!(
            vfs_none.stat("f.txt"),
            Err(VfsError::PermissionDenied)
        ));
    }

    #[test]
    fn isolation_between_plugins() {
        let config = make_test_config();
        let vfs_a = VirtualFs::new(&config, "plugin-a", &make_rw_perms());
        let vfs_b = VirtualFs::new(&config, "plugin-b", &make_rw_perms());
        vfs_a.write_file("data.txt", "from-a").unwrap();
        vfs_b.write_file("data.txt", "from-b").unwrap();
        assert_eq!(vfs_a.read_file("data.txt").unwrap(), "from-a");
        assert_eq!(vfs_b.read_file("data.txt").unwrap(), "from-b");
    }

    #[test]
    fn error_display_human_readable() {
        assert!(VfsError::PathEscape.to_string().contains("escape"));
        assert!(
            VfsError::PermissionDenied
                .to_string()
                .contains("permission")
        );
        assert!(
            VfsError::FileTooLarge { max: 1024 }
                .to_string()
                .contains("1024")
        );
        assert!(
            VfsError::QuotaExceeded {
                used: 100,
                max: 200,
                need: 150,
            }
            .to_string()
            .contains("quota")
        );
    }
}
