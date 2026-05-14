//! Proxy 模块配置加载。
//!
//! 加载 `proxy.toml` 和 `tenants/*.toml`，提供类型安全的配置结构。

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

/// Proxy 全局配置。
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyConfig {
    pub proxy: ProxySection,
}

/// `[proxy]` 段。
#[derive(Debug, Clone, Deserialize)]
pub struct ProxySection {
    /// HTTP 监听地址（默认 `0.0.0.0:80`）。
    #[serde(default = "default_listen_http")]
    pub listen_http: String,
    /// HTTPS 监听地址（默认 `0.0.0.0:443`）。
    #[serde(default = "default_listen_https")]
    pub listen_https: String,
    /// 证书存储目录。
    #[serde(default = "default_acme_dir")]
    pub acme_dir: PathBuf,
    /// ACME 注册邮箱。
    pub acme_email: Option<String>,
    /// ACME 目录 URL（默认 Let's Encrypt 生产环境）。
    #[serde(default = "default_acme_directory")]
    pub acme_directory: String,
    /// 是否自动 HTTP→HTTPS 重定向。
    #[serde(default = "default_true")]
    pub redirect_http_to_https: bool,
    /// 管理 API 监听地址。
    #[serde(default = "default_admin_listen")]
    pub admin_listen: String,
    /// 管理 API 密钥。
    #[serde(default = "default_admin_secret")]
    pub admin_secret: String,
    /// 租户配置文件目录。
    #[serde(default = "default_tenants_dir")]
    pub tenants_dir: PathBuf,
    /// 健康检查间隔（秒）。
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,
    /// 日志目录。
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
}

/// 单个租户配置。
///
/// 每个租户一个 TOML 文件，放在 `tenants_dir` 下。
#[derive(Debug, Clone, Deserialize)]
pub struct TenantConfig {
    pub tenant: TenantSection,
}

/// `[tenant]` 段。
#[derive(Debug, Clone, Deserialize)]
pub struct TenantSection {
    /// 租户名称（唯一标识）。
    pub name: String,
    /// 子域名匹配（如 `user1.api.example.com`）。
    pub host: Option<String>,
    /// 路径前缀匹配（如 `/user1`）。
    pub prefix: Option<String>,
    /// 后端地址（`unix:/path/to.sock` 或 `127.0.0.1:9901`）。
    pub backend: String,
    /// 自定义 TLS 证书路径。
    pub tls_cert: Option<PathBuf>,
    /// 自定义 TLS 私钥路径。
    pub tls_key: Option<PathBuf>,
    /// 连接后端超时（毫秒）。
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_ms: u64,
    /// 读取后端响应超时（毫秒）。
    #[serde(default = "default_read_timeout")]
    pub read_timeout_ms: u64,
    /// 是否启用。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_listen_http() -> String {
    "0.0.0.0:80".into()
}

fn default_listen_https() -> String {
    "0.0.0.0:443".into()
}

fn default_acme_dir() -> PathBuf {
    PathBuf::from("/var/lib/raisfast/acme")
}

fn default_acme_directory() -> String {
    "https://acme-v02.api.letsencrypt.org/directory".into()
}

fn default_admin_listen() -> String {
    "127.0.0.1:9876".into()
}

fn default_admin_secret() -> String {
    "change-me-in-production".into()
}

fn default_tenants_dir() -> PathBuf {
    PathBuf::from("/etc/raisfast/tenants")
}

fn default_health_check_interval() -> u64 {
    30
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("/var/lib/raisfast/proxy/logs")
}

fn default_connect_timeout() -> u64 {
    5000
}

fn default_read_timeout() -> u64 {
    30000
}

impl ProxyConfig {
    /// 从 TOML 文件加载配置。
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// 解析 HTTP 监听地址。
    pub fn http_addr(&self) -> anyhow::Result<SocketAddr> {
        self.proxy.listen_http.parse().map_err(Into::into)
    }

    /// 解析 HTTPS 监听地址。
    pub fn https_addr(&self) -> anyhow::Result<SocketAddr> {
        self.proxy.listen_https.parse().map_err(Into::into)
    }

    /// 解析管理 API 监听地址。
    pub fn admin_addr(&self) -> anyhow::Result<SocketAddr> {
        self.proxy.admin_listen.parse().map_err(Into::into)
    }
}

impl TenantConfig {
    /// 从 TOML 文件加载租户配置。
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

/// 加载 tenants_dir 下所有 .toml 文件。
pub fn load_all_tenants(dir: &std::path::Path) -> Vec<(PathBuf, TenantConfig)> {
    let mut tenants = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return tenants;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            match TenantConfig::load(&path) {
                Ok(t) => tenants.push((path, t)),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load tenant config");
                }
            }
        }
    }
    tenants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_proxy_config() {
        let toml_str = r#"
[proxy]
"#;
        let config: ProxyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.proxy.listen_http, "0.0.0.0:80");
        assert_eq!(config.proxy.listen_https, "0.0.0.0:443");
        assert!(config.proxy.redirect_http_to_https);
    }

    #[test]
    fn parse_full_proxy_config() {
        let toml_str = r#"
[proxy]
listen_http = "0.0.0.0:8080"
listen_https = "0.0.0.0:8443"
acme_dir = "/data/acme"
acme_email = "admin@example.com"
admin_listen = "127.0.0.1:9999"
admin_secret = "my-secret"
tenants_dir = "/etc/raisfast/tenants"
health_check_interval_secs = 60
"#;
        let config: ProxyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.proxy.listen_http, "0.0.0.0:8080");
        assert_eq!(
            config.proxy.acme_email.as_deref(),
            Some("admin@example.com")
        );
        assert_eq!(config.proxy.health_check_interval_secs, 60);
    }

    #[test]
    fn parse_tenant_config() {
        let toml_str = r#"
[tenant]
name = "user1"
host = "user1.api.example.com"
backend = "unix:/run/raisfast/user1.sock"
connect_timeout_ms = 3000
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tenant.name, "user1");
        assert_eq!(config.tenant.host.as_deref(), Some("user1.api.example.com"));
        assert_eq!(config.tenant.backend, "unix:/run/raisfast/user1.sock");
        assert_eq!(config.tenant.connect_timeout_ms, 3000);
        assert!(config.tenant.enabled);
    }

    #[test]
    fn parse_tenant_with_prefix() {
        let toml_str = r#"
[tenant]
name = "user2"
prefix = "/user2"
backend = "127.0.0.1:9902"
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tenant.prefix.as_deref(), Some("/user2"));
        assert!(config.tenant.host.is_none());
    }

    #[test]
    fn default_values() {
        let toml_str = r#"
[proxy]
"#;
        let config: ProxyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.proxy.acme_dir,
            PathBuf::from("/var/lib/raisfast/acme")
        );
        assert_eq!(
            config.proxy.acme_directory,
            "https://acme-v02.api.letsencrypt.org/directory"
        );
        assert_eq!(config.proxy.health_check_interval_secs, 30);
    }

    #[test]
    fn tenant_missing_host_and_prefix() {
        let toml_str = r#"
[tenant]
name = "bare"
backend = "127.0.0.1:9901"
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert!(config.tenant.host.is_none());
        assert!(config.tenant.prefix.is_none());
        assert!(config.tenant.enabled);
    }

    #[test]
    fn tenant_disabled() {
        let toml_str = r#"
[tenant]
name = "off"
host = "off.example.com"
backend = "127.0.0.1:9901"
enabled = false
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.tenant.enabled);
    }

    #[test]
    fn tenant_with_tls() {
        let toml_str = r#"
[tenant]
name = "secure"
host = "secure.example.com"
backend = "127.0.0.1:9901"
tls_cert = "/etc/ssl/secure.pem"
tls_key = "/etc/ssl/secure.key"
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.tenant.tls_cert,
            Some(PathBuf::from("/etc/ssl/secure.pem"))
        );
        assert_eq!(
            config.tenant.tls_key,
            Some(PathBuf::from("/etc/ssl/secure.key"))
        );
    }

    #[test]
    fn tenant_default_timeouts() {
        let toml_str = r#"
[tenant]
name = "defaults"
host = "d.example.com"
backend = "127.0.0.1:9901"
"#;
        let config: TenantConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.tenant.connect_timeout_ms, 5000);
        assert_eq!(config.tenant.read_timeout_ms, 30000);
    }

    #[test]
    fn parse_proxy_addr() {
        let toml_str = r#"
[proxy]
listen_http = "0.0.0.0:8080"
"#;
        let config: ProxyConfig = toml::from_str(toml_str).unwrap();
        let addr = config.http_addr().unwrap();
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn parse_proxy_addr_invalid() {
        let toml_str = r#"
[proxy]
listen_http = "not-an-address"
"#;
        let config: ProxyConfig = toml::from_str(toml_str).unwrap();
        assert!(config.http_addr().is_err());
    }

    #[test]
    fn load_all_tenants_from_dir() {
        let dir = tempfile::tempdir().unwrap();

        let t1 = dir.path().join("user1.toml");
        std::fs::write(
            &t1,
            r#"
[tenant]
name = "user1"
host = "user1.example.com"
backend = "127.0.0.1:9901"
"#,
        )
        .unwrap();

        let t2 = dir.path().join("user2.toml");
        std::fs::write(
            &t2,
            r#"
[tenant]
name = "user2"
host = "user2.example.com"
backend = "127.0.0.1:9902"
"#,
        )
        .unwrap();

        let non_toml = dir.path().join("readme.txt");
        std::fs::write(&non_toml, "ignore me").unwrap();

        let tenants = super::load_all_tenants(dir.path());
        assert_eq!(tenants.len(), 2);

        let names: Vec<&str> = tenants
            .iter()
            .map(|(_, t)| t.tenant.name.as_str())
            .collect();
        assert!(names.contains(&"user1"));
        assert!(names.contains(&"user2"));
    }

    #[test]
    fn load_all_tenants_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let tenants = super::load_all_tenants(dir.path());
        assert!(tenants.is_empty());
    }

    #[test]
    fn load_all_tenants_nonexistent_dir() {
        let tenants = super::load_all_tenants(PathBuf::from("/nonexistent/path").as_path());
        assert!(tenants.is_empty());
    }

    #[test]
    fn load_all_tenants_invalid_toml_skipped() {
        let dir = tempfile::tempdir().unwrap();

        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "this is not valid toml {{{{").unwrap();

        let good = dir.path().join("good.toml");
        std::fs::write(
            &good,
            r#"
[tenant]
name = "good"
host = "good.example.com"
backend = "127.0.0.1:9901"
"#,
        )
        .unwrap();

        let tenants = super::load_all_tenants(dir.path());
        assert_eq!(tenants.len(), 1);
        assert_eq!(tenants[0].1.tenant.name, "good");
    }
}
