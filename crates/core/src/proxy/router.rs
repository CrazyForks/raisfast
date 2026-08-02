//! Concurrency-safe routing table.
//!
//! Maintains Host/Prefix → Backend mapping with runtime dynamic add/remove.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use dashmap::DashMap;

use crate::proxy::config::TenantSection;

/// Backend address.
#[derive(Debug, Clone)]
pub enum BackendAddr {
    UnixSocket(PathBuf),
    Tcp(SocketAddr),
}

impl std::fmt::Display for BackendAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnixSocket(p) => write!(f, "unix:{}", p.display()),
            Self::Tcp(addr) => write!(f, "tcp:{addr}"),
        }
    }
}

/// Backend instance.
#[derive(Debug, Clone)]
pub struct Backend {
    /// Tenant name.
    pub name: String,
    /// Backend address.
    pub addr: BackendAddr,
    /// Whether the backend is healthy.
    pub healthy: Arc<AtomicBool>,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read timeout.
    pub read_timeout: Duration,
}

/// Concurrency-safe routing table.
pub struct RouterTable {
    /// Host → Backend (exact match).
    by_host: DashMap<String, Arc<Backend>>,
    /// Prefix → Backend (longest prefix match).
    by_prefix: DashMap<String, Arc<Backend>>,
}

impl Default for RouterTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RouterTable {
    /// Create an empty routing table.
    pub fn new() -> Self {
        Self {
            by_host: DashMap::new(),
            by_prefix: DashMap::new(),
        }
    }

    /// Batch load from tenant configs.
    pub fn load_from_tenants(&self, tenants: &[TenantSection]) {
        for t in tenants {
            if !t.enabled {
                continue;
            }
            let backend = match parse_backend(&t.backend) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        name = %t.name,
                        backend = %t.backend,
                        error = %e,
                        "skipping tenant with invalid backend"
                    );
                    continue;
                }
            };
            let backend = Arc::new(Backend {
                name: t.name.clone(),
                addr: backend,
                healthy: Arc::new(AtomicBool::new(true)),
                connect_timeout: Duration::from_millis(t.connect_timeout_ms),
                read_timeout: Duration::from_millis(t.read_timeout_ms),
            });

            if let Some(host) = &t.host {
                self.by_host.insert(host.clone(), backend.clone());
                tracing::info!(name = %t.name, host = %host, "registered host route");
            }
            if let Some(prefix) = &t.prefix {
                let key = if prefix.starts_with('/') {
                    prefix.clone()
                } else {
                    format!("/{prefix}")
                };
                self.by_prefix.insert(key.clone(), backend);
                tracing::info!(name = %t.name, prefix = %key, "registered prefix route");
            }
        }
    }

    /// Find backend by Host.
    pub fn find_by_host(&self, host: &str) -> Option<Arc<Backend>> {
        let host = host.split(':').next().unwrap_or(host);
        self.by_host.get(host).map(|r| r.value().clone())
    }

    /// Find backend by path prefix (longest prefix match).
    pub fn find_by_prefix(&self, path: &str) -> Option<Arc<Backend>> {
        let mut best: Option<Arc<Backend>> = None;
        let mut best_len = 0;
        for entry in self.by_prefix.iter() {
            let prefix = entry.key();
            if path.starts_with(prefix) && prefix.len() > best_len {
                best = Some(entry.value().clone());
                best_len = prefix.len();
            }
        }
        best
    }

    /// Combined lookup: Host first, then Prefix.
    pub fn find(&self, host: &str, path: &str) -> Option<Arc<Backend>> {
        self.find_by_host(host)
            .or_else(|| self.find_by_prefix(path))
    }

    /// Add or update tenant route.
    pub fn upsert(&self, tenant: &TenantSection) -> anyhow::Result<()> {
        let addr = parse_backend(&tenant.backend)?;
        let backend = Arc::new(Backend {
            name: tenant.name.clone(),
            addr,
            healthy: Arc::new(AtomicBool::new(true)),
            connect_timeout: Duration::from_millis(tenant.connect_timeout_ms),
            read_timeout: Duration::from_millis(tenant.read_timeout_ms),
        });

        if let Some(host) = &tenant.host {
            self.by_host.insert(host.clone(), backend.clone());
        }
        if let Some(prefix) = &tenant.prefix {
            let key = if prefix.starts_with('/') {
                prefix.clone()
            } else {
                format!("/{prefix}")
            };
            self.by_prefix.insert(key, backend);
        }
        Ok(())
    }

    /// Remove tenant route.
    pub fn remove(&self, name: &str) {
        self.by_host.retain(|_, b| b.name != name);
        self.by_prefix.retain(|_, b| b.name != name);
    }

    /// List all backends.
    pub fn all_backends(&self) -> Vec<Arc<Backend>> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for entry in self.by_host.iter() {
            if seen.insert(entry.value().name.clone()) {
                result.push(entry.value().clone());
            }
        }
        for entry in self.by_prefix.iter() {
            if seen.insert(entry.value().name.clone()) {
                result.push(entry.value().clone());
            }
        }
        result
    }

    /// Number of route entries.
    pub fn len(&self) -> usize {
        self.by_host.len() + self.by_prefix.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parse backend address string.
///
/// Supported formats:
/// - `unix:/path/to/socket`
/// - `127.0.0.1:9901`
fn parse_backend(s: &str) -> anyhow::Result<BackendAddr> {
    if let Some(path) = s.strip_prefix("unix:") {
        Ok(BackendAddr::UnixSocket(PathBuf::from(path)))
    } else {
        let addr: SocketAddr = s.parse()?;
        Ok(BackendAddr::Tcp(addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tenant(
        name: &str,
        host: Option<&str>,
        prefix: Option<&str>,
        backend: &str,
    ) -> TenantSection {
        TenantSection {
            name: name.to_string(),
            host: host.map(|s| s.to_string()),
            prefix: prefix.map(|s| s.to_string()),
            backend: backend.to_string(),
            tls_cert: None,
            tls_key: None,
            connect_timeout_ms: 5000,
            read_timeout_ms: 30000,
            enabled: true,
        }
    }

    #[test]
    fn parse_unix_backend() {
        let addr = parse_backend("unix:/run/raisfast/user1.sock").unwrap();
        let BackendAddr::UnixSocket(p) = addr else {
            panic!("expected unix")
        };
        assert_eq!(p, PathBuf::from("/run/raisfast/user1.sock"));
    }

    #[test]
    fn parse_tcp_backend() {
        let addr = parse_backend("127.0.0.1:9901").unwrap();
        assert!(matches!(addr, BackendAddr::Tcp(a) if a.port() == 9901));
    }

    #[test]
    fn parse_invalid_backend() {
        assert!(parse_backend("invalid-host").is_err());
    }

    #[test]
    fn route_by_host() {
        let router = RouterTable::new();
        router.load_from_tenants(&[
            make_tenant(
                "user1",
                Some("user1.example.com"),
                None,
                "unix:/run/user1.sock",
            ),
            make_tenant(
                "user2",
                Some("user2.example.com"),
                None,
                "unix:/run/user2.sock",
            ),
        ]);

        let b = router.find_by_host("user1.example.com").unwrap();
        assert_eq!(b.name, "user1");

        let b = router.find_by_host("user2.example.com").unwrap();
        assert_eq!(b.name, "user2");

        assert!(router.find_by_host("unknown.example.com").is_none());
    }

    #[test]
    fn route_by_prefix_longest_match() {
        let router = RouterTable::new();
        router.load_from_tenants(&[
            make_tenant("user1", None, Some("/user1"), "unix:/run/user1.sock"),
            make_tenant(
                "user1-admin",
                None,
                Some("/user1/admin"),
                "unix:/run/user1-admin.sock",
            ),
        ]);

        let b = router.find_by_prefix("/user1/admin/posts").unwrap();
        assert_eq!(b.name, "user1-admin");

        let b = router.find_by_prefix("/user1/posts").unwrap();
        assert_eq!(b.name, "user1");

        assert!(router.find_by_prefix("/other").is_none());
    }

    #[test]
    fn route_host_before_prefix() {
        let router = RouterTable::new();
        router.load_from_tenants(&[
            make_tenant(
                "user1",
                Some("user1.example.com"),
                None,
                "unix:/run/user1.sock",
            ),
            make_tenant("fallback", None, Some("/"), "unix:/run/fallback.sock"),
        ]);

        let b = router.find("user1.example.com", "/anything");
        assert_eq!(b.unwrap().name, "user1");

        let b = router.find("unknown.example.com", "/anything");
        assert_eq!(b.unwrap().name, "fallback");
    }

    #[test]
    fn upsert_and_remove() {
        let router = RouterTable::new();
        let tenant = make_tenant("test", Some("test.example.com"), None, "127.0.0.1:9999");
        router.upsert(&tenant).unwrap();

        assert!(router.find_by_host("test.example.com").is_some());

        router.remove("test");
        assert!(router.find_by_host("test.example.com").is_none());
    }

    #[test]
    fn skip_disabled_tenant() {
        let router = RouterTable::new();
        let mut tenant = make_tenant(
            "disabled",
            Some("disabled.example.com"),
            None,
            "127.0.0.1:9999",
        );
        tenant.enabled = false;
        router.load_from_tenants(&[tenant]);

        assert!(router.find_by_host("disabled.example.com").is_none());
        assert!(router.is_empty());
    }
}
