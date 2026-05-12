//! Plugin permission enforcement module
//!
//! Parses permission rules declared in manifest and enforces them at runtime.
//! Supports domain whitelisting (wildcards), config key prefix matching, DB table read/write access control.

use crate::plugins::Permissions;

/// Runtime permission checker
pub struct PermissionChecker;

impl PermissionChecker {
    /// Check if a URL is in the HTTP whitelist.
    ///
    /// Whitelist rules support `*` wildcards (e.g. `*.example.com`, `api.example.com/*`).
    /// If the whitelist is empty, all HTTP requests are denied.
    /// Additionally blocks internal/private IP ranges (SSRF protection).
    #[must_use]
    pub fn is_url_allowed(permissions: &Permissions, url: &str) -> bool {
        if permissions.http.is_empty() {
            return false;
        }

        let host = extract_host(url).unwrap_or_default();
        if is_private_host(&host) {
            return false;
        }
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

    /// Check if a config key is in the config whitelist.
    ///
    /// Whitelist supports prefix matching (e.g. `seo.*` matches `seo.title`, `seo.description`).
    /// If the whitelist is empty, all config access is denied.
    #[must_use]
    pub fn is_config_key_allowed(permissions: &Permissions, key: &str) -> bool {
        if permissions.config.is_empty() {
            return false;
        }

        permissions.config.iter().any(|pattern| {
            if pattern.ends_with('*') {
                key.starts_with(pattern.trim_end_matches('*'))
            } else {
                key == pattern
            }
        })
    }

    /// Check if a table allows read-only access
    #[must_use]
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

    /// Check if a table allows write operations
    #[must_use]
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

    /// Check if SQL is a read-only statement (SELECT only)
    #[must_use]
    pub fn is_readonly_query(sql: &str) -> bool {
        let trimmed = sql.trim().to_uppercase();
        trimmed.starts_with("SELECT")
    }

    /// Check if SQL is a write operation (INSERT / UPDATE / DELETE)
    #[must_use]
    pub fn is_write_query(sql: &str) -> bool {
        let trimmed = sql.trim().to_uppercase();
        trimmed.starts_with("INSERT")
            || trimmed.starts_with("UPDATE")
            || trimmed.starts_with("DELETE")
    }

    /// Check if SQL is a DDL statement (CREATE / DROP / ALTER / TRUNCATE / ATTACH / DETACH / PRAGMA / REINDEX / ANALYZE / VACUUM)
    #[must_use]
    pub fn is_ddl_query(sql: &str) -> bool {
        let trimmed = sql.trim().to_uppercase();
        trimmed.starts_with("CREATE")
            || trimmed.starts_with("DROP")
            || trimmed.starts_with("ALTER")
            || trimmed.starts_with("TRUNCATE")
            || trimmed.starts_with("ATTACH")
            || trimmed.starts_with("DETACH")
            || trimmed.starts_with("PRAGMA")
            || trimmed.starts_with("REINDEX")
            || trimmed.starts_with("ANALYZE")
            || trimmed.starts_with("VACUUM")
    }

    /// Check if a table name is in the protected table list (not writable by plugins/CTs)
    #[must_use]
    pub fn is_protected_table(table: &str, protected: &[String]) -> bool {
        protected.iter().any(|t| t.eq_ignore_ascii_case(table))
    }
}

/// Extract table name from SQL statement (simple heuristic: first identifier after FROM).
///
/// **Security limitation:** Returns `None` when UNION, subqueries, JOIN or other suspicious patterns
/// are detected. Callers should treat `None` as denied.
#[must_use]
pub fn extract_table_name(sql: &str) -> Option<String> {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    if upper.contains("UNION") || upper.contains("JOIN") {
        return None;
    }

    let rest = upper.strip_prefix("SELECT")?;
    let from_pos = rest.find("FROM")?;
    let after_from = rest[from_pos + 4..].trim_start();

    if after_from.starts_with('(') {
        return None;
    }

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

/// Extract target table name from write SQL (INSERT/UPDATE/DELETE)
#[must_use]
pub fn extract_write_table_name(sql: &str) -> Option<String> {
    let upper = sql.trim().to_uppercase();
    if upper.starts_with("INSERT") {
        extract_after_keyword(&upper, "INTO")
    } else if upper.starts_with("UPDATE") {
        extract_first_identifier_after(&upper, "UPDATE")
    } else if upper.starts_with("DELETE") {
        extract_after_keyword(&upper, "FROM")
    } else {
        None
    }
}

/// Extract the first identifier after a keyword (table name)
fn extract_after_keyword(sql: &str, keyword: &str) -> Option<String> {
    let pos = sql.find(keyword)?;
    let after = sql[pos + keyword.len()..].trim_start();
    let table: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if table.is_empty() {
        None
    } else {
        Some(table.to_lowercase())
    }
}

/// Extract the first identifier after a specified keyword (for UPDATE table_name SET ...)
fn extract_first_identifier_after(sql: &str, keyword: &str) -> Option<String> {
    let pos = sql.find(keyword)?;
    let after = sql[pos + keyword.len()..].trim_start();
    let table: String = after
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

fn is_private_host(host: &str) -> bool {
    let lower = host.to_lowercase();
    if lower.is_empty() {
        return false;
    }
    if lower == "localhost" || lower == "0.0.0.0" || lower == "[::1]" || lower == "::1" {
        return true;
    }
    let octets: Vec<&str> = lower.split('.').collect();
    if octets.len() == 4
        && let (Some(a), Some(b), Some(c), Some(_d)) = (
            octets[0].parse::<u8>().ok(),
            octets[1].parse::<u8>().ok(),
            octets[2].parse::<u8>().ok(),
            octets[3].parse::<u8>().ok(),
        )
    {
        if a == 127 {
            return true;
        }
        if a == 10 {
            return true;
        }
        if a == 172 && (16..=31).contains(&b) {
            return true;
        }
        if a == 192 && b == 168 {
            return true;
        }
        if a == 169 && b == 254 {
            return true;
        }
        if a == 0 && b == 0 && c == 0 {
            return true;
        }
    }
    false
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
    fn config_empty_blocks_all() {
        let p = perms(vec![], vec![], vec![]);
        assert!(!PermissionChecker::is_config_key_allowed(&p, "anything"));
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

    #[test]
    fn is_write_query_insert() {
        assert!(PermissionChecker::is_write_query(
            "INSERT INTO orders (id) VALUES ('1')"
        ));
    }

    #[test]
    fn is_write_query_update() {
        assert!(PermissionChecker::is_write_query(
            "UPDATE products SET stock = 0"
        ));
    }

    #[test]
    fn is_write_query_delete() {
        assert!(PermissionChecker::is_write_query("DELETE FROM cart_items"));
    }

    #[test]
    fn is_write_query_rejects_select() {
        assert!(!PermissionChecker::is_write_query("SELECT * FROM orders"));
    }

    #[test]
    fn is_write_query_rejects_ddl() {
        assert!(!PermissionChecker::is_write_query("DROP TABLE orders"));
    }

    #[test]
    fn is_ddl_query_create() {
        assert!(PermissionChecker::is_ddl_query(
            "CREATE TABLE foo (id TEXT)"
        ));
    }

    #[test]
    fn is_ddl_query_alter() {
        assert!(PermissionChecker::is_ddl_query(
            "ALTER TABLE foo ADD COLUMN bar TEXT"
        ));
    }

    #[test]
    fn is_ddl_query_rejects_insert() {
        assert!(!PermissionChecker::is_ddl_query(
            "INSERT INTO foo VALUES (1)"
        ));
    }

    #[test]
    fn extract_write_table_name_insert() {
        assert_eq!(
            extract_write_table_name("INSERT INTO orders (id, status) VALUES ('1', 'pending')"),
            Some("orders".into())
        );
    }

    #[test]
    fn extract_write_table_name_update() {
        assert_eq!(
            extract_write_table_name("UPDATE products SET stock = stock - 1 WHERE id = 'abc'"),
            Some("products".into())
        );
    }

    #[test]
    fn extract_write_table_name_delete() {
        assert_eq!(
            extract_write_table_name("DELETE FROM cart_items WHERE user_id = 'u1'"),
            Some("cart_items".into())
        );
    }

    #[test]
    fn extract_write_table_name_select_returns_none() {
        assert_eq!(extract_write_table_name("SELECT * FROM orders"), None);
    }

    fn default_protected() -> Vec<String> {
        vec![
            "users".into(),
            "roles".into(),
            "permissions".into(),
            "audit_log".into(),
            "plugin_storage".into(),
            "options".into(),
            "rbac_roles".into(),
            "rbac_permissions".into(),
            "rbac_role_permissions".into(),
            "tenants".into(),
        ]
    }

    #[test]
    fn is_protected_table_users() {
        assert!(PermissionChecker::is_protected_table(
            "users",
            &default_protected()
        ));
    }

    #[test]
    fn is_protected_table_orders_is_not() {
        assert!(!PermissionChecker::is_protected_table(
            "orders",
            &default_protected()
        ));
    }

    #[test]
    fn is_protected_table_case_insensitive() {
        assert!(PermissionChecker::is_protected_table(
            "USERS",
            &default_protected()
        ));
        assert!(PermissionChecker::is_protected_table(
            "Roles",
            &default_protected()
        ));
    }

    #[test]
    fn ssrf_blocks_localhost() {
        let p = perms(vec!["localhost"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://localhost/admin"
        ));
    }

    #[test]
    fn ssrf_blocks_127_loopback() {
        let p = perms(vec!["127.0.0.1"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://127.0.0.1/secret"
        ));
    }

    #[test]
    fn ssrf_blocks_private_10() {
        let p = perms(vec!["10.0.0.1"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://10.0.0.1/internal"
        ));
    }

    #[test]
    fn ssrf_blocks_private_172() {
        let p = perms(vec!["172.16.0.1"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://172.16.0.1/db"
        ));
    }

    #[test]
    fn ssrf_blocks_private_192() {
        let p = perms(vec!["192.168.1.1"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://192.168.1.1/admin"
        ));
    }

    #[test]
    fn ssrf_blocks_link_local() {
        let p = perms(vec!["169.254.1.1"], vec![], vec![]);
        assert!(!PermissionChecker::is_url_allowed(
            &p,
            "http://169.254.1.1/metadata"
        ));
    }

    #[test]
    fn ssrf_allows_public_ip() {
        let p = perms(vec!["93.184.216.34"], vec![], vec![]);
        assert!(PermissionChecker::is_url_allowed(
            &p,
            "http://93.184.216.34/index.html"
        ));
    }

    #[test]
    fn sql_union_bypass_returns_none() {
        assert_eq!(
            extract_table_name("SELECT * FROM posts UNION SELECT * FROM users"),
            None
        );
    }

    #[test]
    fn sql_join_returns_none() {
        assert_eq!(
            extract_table_name("SELECT * FROM posts JOIN users ON posts.user_id = users.id"),
            None
        );
    }

    #[test]
    fn sql_subquery_returns_none() {
        assert_eq!(
            extract_table_name("SELECT * FROM (SELECT * FROM users)"),
            None
        );
    }
}
