//! 插件权限执行模块
//!
//! 解析 manifest 中声明的权限规则，并在运行时校验。
//! 支持域名白名单（通配符）、配置 key 前缀匹配、DB 表只读/只写控制。

use crate::plugins::Permissions;

/// 运行时权限检查器
pub struct PermissionChecker;

impl PermissionChecker {
    /// 检查 URL 是否在 HTTP 白名单中。
    ///
    /// 白名单规则支持 `*` 通配符（如 `*.example.com`、`api.example.com/*`）。
    /// 若白名单为空，拒绝所有 HTTP 请求。
    pub fn is_url_allowed(permissions: &Permissions, url: &str) -> bool {
        if permissions.http.is_empty() {
            return false;
        }

        let host = extract_host(url).unwrap_or_default();
        let host: &str = &host;
        let path = extract_path(url);
        let path: &str = &path;

        permissions.http.iter().any(|pattern| {
            if pattern.starts_with("*.") {
                let suffix = &pattern[1..];
                host.ends_with(suffix) && glob_match_path(path, "")
            } else if pattern.contains('*') {
                let parts: Vec<&str> = pattern.splitn(2, '/').collect();
                if parts.len() == 2 {
                    host == parts[0] && glob_match_path(path, parts[1])
                } else {
                    host == pattern.trim_end_matches('*')
                        && path.starts_with(pattern.trim_end_matches('*'))
                }
            } else {
                let parts: Vec<&str> = pattern.splitn(2, '/').collect();
                if parts.len() == 2 {
                    host == parts[0] && path == parts[1] || path.starts_with(parts[1])
                } else {
                    host == pattern
                }
            }
        })
    }

    /// 检查配置 key 是否在 config 白名单中。
    ///
    /// 白名单支持前缀匹配（如 `seo.*` 匹配 `seo.title`、`seo.description`）。
    /// 若白名单为空，允许读取所有配置项（向后兼容）。
    pub fn is_config_key_allowed(permissions: &Permissions, key: &str) -> bool {
        if permissions.config.is_empty() {
            return true;
        }

        permissions.config.iter().any(|pattern| {
            if pattern.ends_with('*') {
                key.starts_with(pattern.trim_end_matches('*'))
            } else {
                key == pattern
            }
        })
    }

    /// 检查数据表是否允许只读访问
    pub fn is_table_readable(permissions: &Permissions, table: &str) -> bool {
        if permissions.database.is_empty() {
            return false;
        }
        permissions.database.iter().any(|p| {
            let p = p.to_lowercase();
            let table = table.to_lowercase();
            p == table || p == format!("read:{table}") || p == "*"
        })
    }

    /// 检查数据表是否允许写操作
    pub fn is_table_writable(permissions: &Permissions, table: &str) -> bool {
        if permissions.database.is_empty() {
            return false;
        }
        permissions.database.iter().any(|p| {
            let p = p.to_lowercase();
            let table = table.to_lowercase();
            p == table || p == format!("write:{table}") || p == "*"
        })
    }

    /// 校验 SQL 是否为只读语句（SELECT only）
    pub fn is_readonly_query(sql: &str) -> bool {
        let trimmed = sql.trim().to_uppercase();
        trimmed.starts_with("SELECT")
    }
}

/// 从 SQL 语句中提取表名（简单启发式，取 FROM 后的第一个标识符）
pub fn extract_table_name(sql: &str) -> Option<String> {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();
    let rest = upper.strip_prefix("SELECT")?;
    let from_pos = rest.find("FROM")?;
    let after_from = rest[from_pos + 4..].trim_start();
    let table: String = after_from
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if table.is_empty() {
        None
    } else {
        Some(table.to_lowercase())
    }
}

fn extract_host(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host_part = without_scheme.split('?').next().unwrap_or(without_scheme);
    let host = host_part.split('/').next().unwrap_or(host_part);
    Some(host.split(':').next().unwrap_or(host).to_string())
}

fn extract_path(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let after_host = without_scheme
        .find('/')
        .map_or("", |i| &without_scheme[i..]);
    after_host
        .split('?')
        .next()
        .unwrap_or(after_host)
        .to_string()
}

fn glob_match_path(path: &str, pattern: &str) -> bool {
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    let path = path.strip_prefix('/').unwrap_or(path);
    if pattern.is_empty() || pattern == "*" || pattern == "/*" {
        return true;
    }
    if pattern.ends_with('*') {
        path.starts_with(pattern.trim_end_matches('*'))
    } else {
        path == pattern || path.starts_with(pattern.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(http: Vec<&str>, config: Vec<&str>, database: Vec<&str>) -> Permissions {
        Permissions {
            http: http.into_iter().map(String::from).collect(),
            config: config.into_iter().map(String::from).collect(),
            database: database.into_iter().map(String::from).collect(),
            filesystem: vec![],
            max_memory_mb: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn url_empty_whitelist_blocks_all() {
        let p = perms(vec![], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "https://example.com/api"
        ));
    }

    #[test]
    fn url_exact_domain_match() {
        let p = perms(vec!["api.example.com"], vec![], vec![]);
        assert!(PermissionChecker::is_url_allowed(
            &p,
            "https://api.example.com/v1/data"
        ));
    }

    #[test]
    fn url_wildcard_subdomain() {
        let p = perms(vec!["*.example.com"], vec![], vec![]);
        assert!(PermissionChecker::is_url_allowed(
            &p,
            "https://cdn.example.com/asset.js"
        ));
        assert!(PermissionChecker::is_url_allowed(
            &p,
            "https://api.example.com/v1"
        ));
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "https://evil.com/steal"
        ));
    }

    #[test]
    fn url_wildcard_path() {
        let p = perms(vec!["api.example.com/v1/*"], vec![], vec![]);
        assert!(PermissionChecker::is_url_allowed(
            &p,
            "https://api.example.com/v1/posts"
        ));
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "https://api.example.com/v2/posts"
        ));
    }

    #[test]
    fn config_prefix_match() {
        let p = perms(vec![], vec!["seo.*"], vec![]);
        assert!(PermissionChecker::is_config_key_allowed(&p, "seo.title"));
        assert!(PermissionChecker::is_config_key_allowed(
            &p,
            "seo.description"
        ));
        assert!(!PermissionChecker::is_config_key_allowed(&p, "app.host"));
    }

    #[test]
    fn config_empty_allows_all() {
        let p = perms(vec![], vec![], vec![]);
        assert!(PermissionChecker::is_config_key_allowed(&p, "anything"));
    }

    #[test]
    fn config_exact_match() {
        let p = perms(vec![], vec!["app.env"], vec![]);
        assert!(PermissionChecker::is_config_key_allowed(&p, "app.env"));
        assert!(!PermissionChecker::is_config_key_allowed(&p, "app.host"));
    }

    #[test]
    fn readonly_query_check() {
        assert!(PermissionChecker::is_readonly_query("SELECT * FROM posts"));
        assert!(PermissionChecker::is_readonly_query(
            "  select id from users"
        ));
        assert!(!PermissionChecker::is_readonly_query(
            "INSERT INTO posts VALUES(1)"
        ));
        assert!(!PermissionChecker::is_readonly_query("DELETE FROM posts"));
        assert!(!PermissionChecker::is_readonly_query(
            "UPDATE posts SET title='x'"
        ));
    }

    #[test]
    fn database_empty_blocks_all() {
        let p = perms(vec![], vec![], vec![]);
        assert!(!PermissionChecker::is_table_readable(&p, "posts"));
    }

    #[test]
    fn database_read_access() {
        let p = perms(vec![], vec![], vec!["read:posts"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(!PermissionChecker::is_table_writable(&p, "posts"));
    }

    #[test]
    fn database_full_access() {
        let p = perms(vec![], vec![], vec!["posts"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(PermissionChecker::is_table_writable(&p, "posts"));
    }

    #[test]
    fn database_wildcard_access() {
        let p = perms(vec![], vec![], vec!["*"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(PermissionChecker::is_table_writable(&p, "comments"));
    }

    #[test]
    fn database_case_insensitive() {
        let p = perms(vec![], vec![], vec!["Posts"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(PermissionChecker::is_table_readable(&p, "POSTS"));
    }

    #[test]
    fn database_write_only_no_read() {
        let p = perms(vec![], vec![], vec!["write:posts"]);
        assert!(!PermissionChecker::is_table_readable(&p, "posts"));
        assert!(PermissionChecker::is_table_writable(&p, "posts"));
    }

    #[test]
    fn database_read_only_no_write() {
        let p = perms(vec![], vec![], vec!["read:posts"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(!PermissionChecker::is_table_writable(&p, "posts"));
    }

    #[test]
    fn database_multiple_tables() {
        let p = perms(vec![], vec![], vec!["read:posts", "write:comments"]);
        assert!(PermissionChecker::is_table_readable(&p, "posts"));
        assert!(!PermissionChecker::is_table_writable(&p, "posts"));
        assert!(!PermissionChecker::is_table_readable(&p, "comments"));
        assert!(PermissionChecker::is_table_writable(&p, "comments"));
        assert!(!PermissionChecker::is_table_readable(&p, "users"));
    }

    #[test]
    fn extract_table_name_basic() {
        assert_eq!(
            extract_table_name("SELECT * FROM posts"),
            Some("posts".into())
        );
        assert_eq!(
            extract_table_name("SELECT id FROM users"),
            Some("users".into())
        );
        assert_eq!(
            extract_table_name("select * from comments"),
            Some("comments".into())
        );
    }

    #[test]
    fn extract_table_name_with_where() {
        assert_eq!(
            extract_table_name("SELECT * FROM posts WHERE id = ?"),
            Some("posts".into())
        );
    }

    #[test]
    fn extract_table_name_invalid() {
        assert_eq!(extract_table_name("INSERT INTO posts VALUES(1)"), None);
        assert_eq!(extract_table_name(""), None);
        assert_eq!(extract_table_name("DELETE FROM posts"), None);
    }

    #[test]
    fn extract_table_name_extra_whitespace() {
        assert_eq!(
            extract_table_name("  SELECT   *   FROM   tags  "),
            Some("tags".into())
        );
    }
}
