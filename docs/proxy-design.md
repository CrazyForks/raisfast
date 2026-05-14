# raisfast 内置 Proxy 模块设计

> 替代 nginx/caddy，实现 raisfast 单二进制多租户部署，零外部依赖。

---

## 1. 目标

| 目标 | 说明 |
|------|------|
| **零依赖** | 不需要 nginx/caddy/证书工具，raisfast 一个二进制搞定 |
| **多租户路由** | 根据域名（Host 头）或路径前缀分发到后端实例 |
| **自动 HTTPS** | 集成 ACME (Let's Encrypt)，自动申请/续签证书 |
| **热更新** | 新增/删除租户不需要重启 proxy |
| **轻量** | proxy 进程自身内存 < 10MB |
| **可选** | proxy 是独立模式，不影响单实例直接部署 |

---

## 2. 架构

```
                              raisfast 二进制
┌────────────────────────────────────────────────────────────────┐
│                                                                │
│  模式 A：单实例（现有行为）                                       │
│  ┌──────────────────────────────────────────┐                  │
│  │  TCP :9898 → axum Router → 业务逻辑       │                  │
│  └──────────────────────────────────────────┘                  │
│                                                                │
│  模式 B：Proxy + 多实例（新模式）                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Proxy 进程 (TCP :80/:443)                                │  │
│  │  ┌─────────────────────────────────────────────────────┐ │  │
│  │  │  TLS 终结 (rustls)                                   │ │  │
│  │  │  Host/Path 匹配 → 路由表查找                          │ │  │
│  │  │  HTTP/1.1 反向代理 → Unix Socket / TCP               │ │  │
│  │  │  ACME 证书管理 (Let's Encrypt)                       │ │  │
│  │  └─────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

**关键决策：Proxy 是独立进程，不是嵌入业务进程的中间件。**

原因：
- Proxy 必须在 80/443 端口上运行，后端实例在内网端口/Unix socket
- Proxy 挂了不应影响已有的长连接
- 可以单独升级/重启 proxy 而不影响业务实例

---

## 3. 启动方式

### 3.1 CLI 命令

```bash
# 单实例模式（现有行为，不变）
raisfast server start

# Proxy 模式（新增）
raisfast proxy start --config /etc/raisfast/proxy.toml

# 两者可以同时运行在同一台机器上
```

### 3.2 Proxy 配置文件

```toml
# /etc/raisfast/proxy.toml

[proxy]
listen_http = "0.0.0.0:80"
listen_https = "0.0.0.0:443"

# 证书存储目录（ACME 自动管理）
acme_dir = "/var/lib/raisfast/acme"
# ACME 邮箱
acme_email = "admin@example.com"
# ACME 目录（默认 Let's Encrypt）
acme_directory = "https://acme-v02.api.letsencrypt.org/directory"
# 测试环境
# acme_directory = "https://acme-staging-v02.api.letsencrypt.org/directory"

# 是否自动 HTTP→HTTPS 重定向
redirect_http_to_https = true

# 管理 API（用于动态增删租户）
admin_listen = "127.0.0.1:9876"
admin_secret = "a-secret-for-admin-api"

# 租户配置文件目录（watch 模式）
tenants_dir = "/etc/raisfast/tenants"

# 健康检查间隔
health_check_interval_secs = 30

# 日志
log_dir = "/var/lib/raisfast/proxy/logs"
```

### 3.3 租户配置

每个租户一个 TOML 文件，放在 `tenants_dir` 下：

```toml
# /etc/raisfast/tenants/user1.toml
[tenant]
name = "user1"

# 路由匹配方式（二选一）
host = "user1.api.example.com"
# prefix = "/user1"

# 后端地址（支持 Unix socket 和 TCP）
backend = "unix:/run/raisfast/user1.sock"
# backend = "127.0.0.1:9901"

# TLS（可选，默认走通配符证书）
# 自定义证书路径
# tls_cert = "/etc/ssl/user1.pem"
# tls_key = "/etc/ssl/user1.key"

# 超时
connect_timeout_ms = 5000
read_timeout_ms = 30000

# 是否启用（可临时禁用）
enabled = true
```

---

## 4. 模块结构

```
src/proxy/
├── mod.rs              # 模块入口，模式判断
├── config.rs           # proxy 配置加载（proxy.toml + tenants/*.toml）
├── router.rs           # 路由表（Host/Prefix → Backend 映射）
├── proxy.rs            # HTTP 反向代理核心（hyper 实现）
├── tls.rs              # TLS 终结 + SNI 路由 + 证书管理
├── acme.rs             # ACME 自动证书申请/续签
├── admin.rs            # 管理 API（动态增删租户）
├── health.rs           # 后端健康检查
└── watcher.rs          # 配置文件热加载（notify crate）
```

### 4.1 模块职责

#### `config.rs` — 配置加载

```rust
struct ProxyConfig {
    listen_http: SocketAddr,
    listen_https: SocketAddr,
    acme_dir: PathBuf,
    acme_email: String,
    acme_directory: String,
    redirect_http_to_https: bool,
    admin_listen: SocketAddr,
    admin_secret: String,
    tenants_dir: PathBuf,
    health_check_interval_secs: u64,
    log_dir: PathBuf,
}

struct TenantConfig {
    name: String,
    host: Option<String>,        // 子域名匹配
    prefix: Option<String>,      // 路径前缀匹配
    backend: String,             // "unix:/path" 或 "tcp:host:port"
    tls_cert: Option<PathBuf>,   // 自定义证书
    tls_key: Option<PathBuf>,
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    enabled: bool,
}
```

加载优先级：
1. `tenants_dir/*.toml` — 文件配置
2. 管理 API 动态注册 — 运行时增删
3. 两者合并到路由表

#### `router.rs` — 路由表

```rust
use dashmap::DashMap;

struct RouterTable {
    by_host: DashMap<String, Backend>,    // host → backend
    by_prefix: DashMap<String, Backend>,  // prefix → backend
}

struct Backend {
    name: String,
    addr: BackendAddr,
    healthy: Arc<AtomicBool>,
    connect_timeout: Duration,
    read_timeout: Duration,
}

enum BackendAddr {
    UnixSocket(PathBuf),
    Tcp(SocketAddr),
}
```

路由匹配优先级：
1. 精确 Host 匹配
2. 路径前缀匹配（最长前缀优先）
3. 默认后端（可选）

#### `proxy.rs` — 反向代理核心

```rust
/// 核心代理函数
async fn proxy_request(
    req: Request<IncomingBody>,
    backend: &Backend,
) -> Result<Response<Body>, ProxyError> {
    // 1. 根据 BackendAddr 拨号连接
    // 2. 转发请求（headers + body）
    // 3. 流式转发响应
    // 4. 支持连接复用（keep-alive）
}
```

实现要点：
- 使用 `hyper` 直接做 HTTP/1.1 代理（不经过 axum，减少开销）
- Unix socket 用 `tokio::net::UnixStream` → `hyper::client::conn::http1::handshake`
- TCP 用 `tokio::net::TcpStream` → 同上
- 流式转发 body，不缓冲到内存（支持大文件上传）
- 透传 `X-Forwarded-For` / `X-Forwarded-Proto` / `X-Forwarded-Host`
- 支持 WebSocket 升级（`Connection: Upgrade`）

#### `tls.rs` — TLS 终结

```rust
use rustls::ServerConfig;

struct TlsManager {
    cert_resolver: Arc<dyn ResolvesServerCert>,
    acme: Arc<AcmeManager>,
}

/// 基于 SNI 的证书分发
/// - 通配符域名：*.api.example.com → 通配符证书
/// - 自定义证书：用户指定了 tls_cert/tls_key
/// - 自动 ACME：按需申请
```

实现要点：
- 使用 `tokio-rustls` 做 TLS 终结
- `rustls::server::ResolvesServerCert` trait 实现动态证书选择
- 支持 SNI 路由（TLS 握手时就知道目标域名）
- 通配符证书一个就够了，无需每租户单独申请

#### `acme.rs` — 自动 HTTPS

```rust
struct AcmeManager {
    dir: PathBuf,                    // 证书存储目录
    email: String,
    directory: String,               // ACME directory URL
    account: OnceCell<Account>,      // ACME 账号
}

impl AcmeManager {
    /// 获取指定域名的证书（缓存优先，过期自动续签）
    async fn get_certificate(&self, domain: &str) -> Result<Arc<CertifiedKey>>;
    
    /// 通配符证书申请（DNS-01 验证，需要 DNS provider）
    async fn request_wildcard(&self, domain: &str) -> Result<()>;
    
    /// 单域名证书申请（HTTP-01 验证，proxy 自身处理 /.well-known/）
    async fn request_single(&self, domain: &str) -> Result<()>;
}
```

实现方案：
- 不引入重量级 `rustls-acme` crate，自己实现轻量 ACME 客户端
- 使用 `reqwest` 调 ACME API + `ring` 做签名/验证
- HTTP-01 验证：proxy 自身在 `/.well-known/acme-challenge/` 路径返回验证文件
- DNS-01 验证：可选，通配符证书需要，需要集成 DNS provider API
- 证书缓存：`acme_dir/{domain}/fullchain.pem` + `privkey.pem`
- 续签：后台定时检查，过期前 30 天自动续签

#### `admin.rs` — 管理 API

```rust
// POST /admin/tenants          创建租户
// DELETE /admin/tenants/{name} 删除租户
// GET  /admin/tenants          列出所有租户
// GET  /admin/tenants/{name}   查看租户详情
// POST /admin/reload           重新加载配置文件
// GET  /admin/stats            代理统计
```

认证：`Authorization: Bearer {admin_secret}`

#### `health.rs` — 健康检查

```rust
struct HealthChecker {
    backends: Vec<(String, BackendAddr)>,
    interval: Duration,
}

impl HealthChecker {
    /// 定时检查所有后端，更新 healthy 状态
    /// TCP 后端：尝试 TCP connect
    /// Unix socket 后端：尝试 connect Unix socket
    /// 也可发 HTTP GET /health
}
```

#### `watcher.rs` — 配置热加载

```rust
/// 监听 tenants_dir 目录变化
/// 新增 .toml → 添加到路由表
/// 修改 .toml → 更新路由表
/// 删除 .toml → 从路由表移除
fn watch_tenants_dir(dir: &Path, router: Arc<RouterTable>) -> JoinHandle<()>;
```

使用 `notify` crate 监听文件系统事件。

---

## 5. 依赖

```toml
# Cargo.toml 新增（在 proxy feature 下）

[features]
proxy = [
    "hyper-util",      # HTTP 客户端/服务端工具
    "tower",           # Service trait
    "dashmap",         # 并发路由表
    "notify",          # 文件系统监听
    "rcgen",           # 自签名证书生成
]

# 已有依赖（复用）
# hyper        — 已在依赖中（axum 底层）
# tokio        — 已在依赖中
# rustls       — 已在 tls feature 中
# tokio-rustls — 已在 tls feature 中
# reqwest      — 已在依赖中（ACME HTTP 请求）
# serde/toml   — 已在依赖中
# tracing      — 已在依赖中
```

不需要新增大型依赖，大部分可复用现有 crate。

---

## 6. 核心流程

### 6.1 请求处理流程

```
客户端请求 → TCP accept → TLS 握手（SNI → 选证书）
    → HTTP 解析 → 提取 Host 头
    → 路由表查找（by_host）
    → 未命中 → 路径前缀匹配（by_prefix）
    → 未命中 → 502 Bad Gateway
    → 命中 → 检查 healthy 状态
    → 不健康 → 503 Service Unavailable
    → 健康 → 拨号后端（Unix socket / TCP）
    → 流式转发请求
    → 流式转发响应
    → 记录 metrics
```

### 6.2 证书获取流程

```
新域名首次请求
    → cert_resolver 查询缓存 → miss
    → ACME 申请（HTTP-01）
    → 保存到 acme_dir
    → 返回证书
    → TLS 握手完成

后续请求
    → cert_resolver 查询缓存 → hit
    → 检查过期时间
    → 即将过期 → 后台续签
    → 返回证书
```

### 6.3 租户生命周期

```
1. 创建租户
   方式 A：写入 /etc/raisfast/tenants/user1.toml → watcher 自动加载
   方式 B：POST /admin/tenants → API 动态注册
   
2. 运行中
   - 健康检查定时运行
   - 证书自动续签
   - metrics 收集
   
3. 删除租户
   方式 A：删除 .toml 文件 → watcher 自动移除
   方式 B：DELETE /admin/tenants/user1
```

---

## 7. CLI 集成

```rust
// src/cli.rs 新增

#[derive(Subcommand)]
enum Commands {
    // ... 现有命令 ...
    
    /// Proxy management (multi-tenant reverse proxy)
    Proxy {
        #[command(subcommand)]
        action: ProxyAction,
    },
}

#[derive(Subcommand)]
enum ProxyAction {
    /// Start the proxy server
    Start {
        /// Path to proxy config file
        #[arg(short, long, default_value = "/etc/raisfast/proxy.toml")]
        config: String,
    },
    /// Validate proxy configuration
    Check {
        /// Path to proxy config file
        #[arg(short, long, default_value = "/etc/raisfast/proxy.toml")]
        config: String,
    },
    /// List all registered tenants
    Tenants {
        /// Proxy admin API address
        #[arg(short, long, default_value = "127.0.0.1:9876")]
        addr: String,
    },
    /// Add a new tenant
    AddTenant {
        /// Tenant name
        name: String,
        /// Host domain (e.g., user1.api.example.com)
        #[arg(short, long)]
        host: Option<String>,
        /// Path prefix (e.g., /user1)
        #[arg(short, long)]
        prefix: Option<String>,
        /// Backend address (e.g., unix:/run/raisfast/user1.sock)
        #[arg(short, long)]
        backend: String,
    },
    /// Remove a tenant
    RemoveTenant {
        /// Tenant name
        name: String,
    },
}
```

---

## 8. 数据流

### HTTP 反向代理核心代码（简化版）

```rust
use hyper::body::Incoming;
use hyper::Request;
use hyper::Response;
use hyper::body::Body;

async fn handle_request(
    req: Request<Incoming>,
    router: Arc<RouterTable>,
) -> Result<Response<Body>, ProxyError> {
    // 1. 提取 Host
    let host = req.headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    // 2. 查找后端
    let backend = router.find_by_host(host)
        .or_else(|| router.find_by_prefix(req.uri().path()))
        .ok_or(ProxyError::NoBackend)?;

    // 3. 健康检查
    if !backend.healthy.load(Ordering::Relaxed) {
        return Ok(Response::builder()
            .status(503)
            .body("service unavailable".into())
            .unwrap());
    }

    // 4. 拨号连接
    let stream = match &backend.addr {
        BackendAddr::UnixSocket(path) => {
            let s = UnixStream::connect(path).await?;
            TokioIo::new(s)
        }
        BackendAddr::Tcp(addr) => {
            let s = TcpStream::connect(addr).await?;
            TokioIo::new(s)
        }
    };

    // 5. 构建代理请求
    let (mut sender, conn) = hyper::client::conn::http1::handshake(stream).await?;
    tokio::spawn(async move { let _ = conn.await; });

    // 6. 注入转发头
    let mut proxy_req = Request::new(req.into_body());
    *proxy_req.method_mut() = req.method().clone();
    *proxy_req.uri_mut() = req.uri().clone();
    *proxy_req.headers_mut() = req.headers().clone();
    proxy_req.headers_mut().insert(
        "x-forwarded-for",
        "peer-ip".parse().unwrap(),
    );
    proxy_req.headers_mut().insert(
        "x-forwarded-proto",
        "https".parse().unwrap(),
    );

    // 7. 发送并返回响应
    let response = sender.send_request(proxy_req).await?;
    Ok(response.map(|b| Body::wrap_stream(b)))
}
```

---

## 9. 性能预期

| 指标 | 目标 | 说明 |
|------|------|------|
| 代理延迟增加 | < 0.05ms | Unix socket 本地转发 |
| 代理吞吐 | > 100k req/s | hyper 直连，无 axum 开销 |
| 内存占用 | < 10MB | proxy 进程本身 |
| 并发连接 | > 10k | tokio 异步 |
| TLS 握手 | < 5ms | rustls 会话复用 |

---

## 10. 安全

| 安全项 | 措施 |
|--------|------|
| 管理 API | Bearer token 认证，仅监听 127.0.0.1 |
| 后端隔离 | 每租户独立 Unix socket，进程级隔离 |
| TLS | 仅支持 TLS 1.2+，禁用弱密码套件 |
| 请求头 | 注入 X-Forwarded-* 头，后端可验证 |
| 速率限制 | proxy 层全局速率限制（可选） |
| 路径穿越 | 阻止 `../` 等路径穿越攻击 |

---

## 11. 实施阶段

### Phase 1：最小可用（1 周）

- [ ] `src/proxy/mod.rs` + `config.rs` — 配置加载
- [ ] `src/proxy/router.rs` — 路由表
- [ ] `src/proxy/proxy.rs` — HTTP 反向代理（仅 Unix socket + TCP）
- [ ] CLI 集成 `raisfast proxy start`
- [ ] 测试：手动配置 2 个租户，curl 验证路由分发

**产出**：能用，但没有 TLS，没有自动证书，手动配路由。

### Phase 2：TLS + 自动证书（1 周）

- [ ] `src/proxy/tls.rs` — TLS 终结 + SNI 路由
- [ ] `src/proxy/acme.rs` — ACME HTTP-01 自动证书
- [ ] HTTP→HTTPS 自动重定向
- [ ] 证书缓存 + 自动续签
- [ ] 测试：Let's Encrypt 真实证书申请

**产出**：自动 HTTPS，不需要 certbot。

### Phase 3：生产就绪（1 周）

- [ ] `src/proxy/admin.rs` — 管理 API
- [ ] `src/proxy/health.rs` — 健康检查
- [ ] `src/proxy/watcher.rs` — 配置文件热加载
- [ ] `src/proxy/access_log.rs` — 访问日志（见第 15 节详细设计）
- [ ] WebSocket 代理支持
- [ ] metrics（请求数/延迟/错误率）
- [ ] 优雅关闭
- [ ] 压力测试

**产出**：生产可用的内置反向代理。

### Phase 4：高级特性（可选）

- [ ] 通配符证书（DNS-01 验证）
- [ ] HTTP/2 代理
- [ ] 请求/响应缓冲控制
- [ ] 限流（per-tenant rate limit）
- [ ] 负载均衡（一个租户多后端）

---

## 12. 与现有代码的关系

| 文件 | 变更 |
|------|------|
| `src/main.rs` | 无变更 |
| `src/cli.rs` | 新增 `Proxy` 子命令 |
| `src/cli/proxy_cmd.rs` | 新增 |
| `src/proxy/` | 新增整个模块 |
| `src/server.rs` | 无变更 |
| `src/config/app.rs` | 无变更（proxy 有自己的配置） |
| `Cargo.toml` | 新增 `proxy` feature |

**Proxy 模块完全独立于现有 server 模块，零侵入。**

---

## 13. 运维命令速查

```bash
# 启动 proxy
raisfast proxy start
raisfast proxy start --config /etc/raisfast/proxy.toml

# 检查配置
raisfast proxy check

# 列出租户
raisfast proxy tenants

# 添加租户
raisfast proxy add-tenant user1 --host user1.api.example.com --backend unix:/run/raisfast/user1.sock

# 删除租户
raisfast proxy remove-tenant user1

# 管理 API（curl）
curl -H "Authorization: Bearer secret" http://127.0.0.1:9876/admin/tenants
curl -X POST -H "Authorization: Bearer secret" http://127.0.0.1:9876/admin/tenants \
  -d '{"name":"user2","host":"user2.api.example.com","backend":"unix:/run/raisfast/user2.sock"}'
curl -X DELETE -H "Authorization: Bearer secret" http://127.0.0.1:9876/admin/tenants/user2
```

---

## 14. 与 nginx/caddy 的功能对比

| 功能 | nginx | caddy | raisfast proxy |
|------|-------|-------|---------------|
| HTTP 反向代理 | ✅ | ✅ | ✅ |
| Unix socket 后端 | ✅ | ✅ | ✅ |
| 自动 HTTPS | ❌ (需 certbot) | ✅ | ✅ |
| TLS SNI 路由 | ✅ | ✅ | ✅ |
| 管理 API | ❌ | ✅ | ✅ |
| 配置热加载 | ✅ (reload) | ✅ (API) | ✅ (API + fs watch) |
| WebSocket | ✅ | ✅ | ✅ |
| Access Log | ✅ | ✅ | ✅ (见第 15 节) |
| HTTP/3 | 需模块 | ✅ | ❌ (Phase 4+) |
| 负载均衡 | ✅ | ✅ | ❌ (Phase 4+) |
| HTTP/2 proxy | ✅ | ✅ | ❌ (Phase 4+) |
| 内置业务逻辑 | ❌ | ❌ | ✅ (同二进制) |
| 部署依赖 | 需安装 | 需安装 | 零依赖 |

---

## 15. Access Log 详细设计

### 15.1 设计目标

对标 nginx `log_format` + `access_log`，但更现代：

- **结构化 JSON 输出**，可直接对接 ELK / Loki / Datadog
- **每租户独立日志文件**，方便多租户隔离
- **异步写入**，不阻塞请求处理
- **可配置格式**（JSON / Common Log Format / 自定义）
- **自动日志轮转**，避免磁盘爆满

### 15.2 日志格式

#### JSON 格式（默认，推荐）

```json
{
  "timestamp": "2026-05-14T10:23:45.123Z",
  "tenant": "user1",
  "client_ip": "203.0.113.50",
  "method": "GET",
  "path": "/api/v1/posts",
  "query": "?page=1&limit=10",
  "protocol": "HTTP/1.1",
  "status": 200,
  "bytes_sent": 4523,
  "bytes_received": 0,
  "latency_ms": 12,
  "upstream": "unix:/run/raisfast/user1.sock",
  "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
  "referer": "https://example.com/blog",
  "request_id": "req-a1b2c3d4",
  "tls_version": "TLSv1.3",
  "tls_cipher": "TLS_AES_256_GCM_SHA384"
}
```

#### Common Log Format（兼容模式，对标 nginx）

```
203.0.113.50 - - [14/May/2026:10:23:45 +0000] "GET /api/v1/posts?page=1&limit=10 HTTP/1.1" 200 4523 "https://example.com/blog" "Mozilla/5.0 ..." "user1" 12ms
```

### 15.3 配置

```toml
# proxy.toml 新增

[access_log]
# 是否启用（默认 true）
enabled = true

# 日志格式："json" | "clf" | "combined"
format = "json"

# 日志输出："file" | "stdout" | "both"
output = "file"

# 日志文件目录
dir = "/var/lib/raisfast/proxy/logs"

# 是否按租户分文件
per_tenant = true

# 是否记录请求体大小（默认 true）
log_bytes_received = true

# 是否记录 TLS 信息（默认 true）
log_tls_info = true

# 排除的路径（不记录日志，如健康检查）
exclude_paths = ["/health", "/healthz", "/readyz", "/metrics"]

# 日志轮转
[access_log.rotation]
# 最大单文件大小（MB）
max_size_mb = 100
# 保留文件数
max_files = 30
# 压缩旧日志
compress = true
```

### 15.4 文件布局

```
/var/lib/raisfast/proxy/logs/
├── access.log                        # 全局日志（per_tenant=false 时）
├── access.2026-05-13.log.gz          # 轮转后的压缩日志
├── tenants/
│   ├── user1.access.log              # user1 的独立日志
│   ├── user1.access.2026-05-13.log.gz
│   ├── user2.access.log              # user2 的独立日志
│   └── ...
```

### 15.5 核心实现

#### `src/proxy/access_log.rs`

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AccessLogEntry {
    pub timestamp: String,
    pub tenant: String,
    pub client_ip: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub protocol: String,
    pub status: u16,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub latency_ms: u64,
    pub upstream: String,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub request_id: Option<String>,
    pub tls_version: Option<String>,
    pub tls_cipher: Option<String>,
}

#[derive(Debug, Clone)]
pub enum LogFormat {
    Json,
    Clf,
    Combined,
}

#[derive(Debug, Clone)]
pub enum LogOutput {
    File,
    Stdout,
    Both,
}

pub struct AccessLogConfig {
    pub enabled: bool,
    pub format: LogFormat,
    pub output: LogOutput,
    pub dir: PathBuf,
    pub per_tenant: bool,
    pub log_bytes_received: bool,
    pub log_tls_info: bool,
    pub exclude_paths: Vec<String>,
    pub max_size_mb: u64,
    pub max_files: usize,
    pub compress: bool,
}

/// 异步 access log 写入器
///
/// 通过 mpsc channel 接收日志条目，后台线程批量写入文件。
/// 不阻塞请求处理线程。
pub struct AccessLogger {
    tx: mpsc::Sender<AccessLogEntry>,
    config: Arc<AccessLogConfig>,
}

impl AccessLogger {
    pub fn new(config: AccessLogConfig) -> Self {
        let (tx, rx) = mpsc::channel::<AccessLogEntry>(4096);
        let config = Arc::new(config);

        // 启动后台写入协程
        tokio::spawn(Self::writer_loop(rx, config.clone()));

        Self { tx, config }
    }

    /// 记录一条 access log（非阻塞，发送到 channel）
    pub async fn log(&self, entry: AccessLogEntry) {
        // 排除路径检查
        if self.should_exclude(&entry.path) {
            return;
        }
        // channel 满时丢弃，不阻塞请求
        let _ = self.tx.try_send(entry);
    }

    /// 后台写入循环
    async fn writer_loop(
        mut rx: mpsc::Receiver<AccessLogEntry>,
        config: Arc<AccessLogConfig>,
    ) {
        // 打开文件句柄（全局 + per-tenant）
        // 批量缓冲，定期 flush
        // 检查文件大小，触发轮转
        // 轮转后压缩旧文件
        loop {
            match rx.recv().await {
                Some(entry) => {
                    let line = match config.format {
                        LogFormat::Json => serde_json::to_string(&entry).unwrap_or_default(),
                        LogFormat::Clf => format_clf(&entry),
                        LogFormat::Combined => format_combined(&entry),
                    };

                    if matches!(config.output, LogOutput::Stdout | LogOutput::Both) {
                        tracing::info!("{}", line);
                    }
                    if matches!(config.output, LogOutput::File | LogOutput::Both) {
                        Self::write_to_file(&config, &entry.tenant, &line).await;
                    }
                }
                None => break,
            }
        }
    }

    fn should_exclude(&self, path: &str) -> bool {
        self.config.exclude_paths.iter().any(|p| path == p)
    }
}
```

#### 在 `proxy.rs` 中集成

```rust
async fn handle_request(
    req: Request<Incoming>,
    router: Arc<RouterTable>,
    logger: Arc<AccessLogger>,
) -> Result<Response<Body>, ProxyError> {
    let start = std::time::Instant::now();
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let client_ip = extract_client_ip(&req);
    let user_agent = req.headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let referer = req.headers()
        .get("referer")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // ... 路由查找 + 代理转发 ...

    let latency = start.elapsed();
    let status = response.status().as_u16();

    // 异步记录 access log
    logger.log(AccessLogEntry {
        timestamp: crate::utils::tz::now_str(),
        tenant: backend.name.clone(),
        client_ip,
        method: method.to_string(),
        path,
        query: req.uri().query().map(|s| s.to_string()),
        protocol: "HTTP/1.1".to_string(),
        status,
        bytes_sent: 0,    // 从 response header 读取
        bytes_received: 0, // 从 request header 读取
        latency_ms: latency.as_millis() as u64,
        upstream: backend.addr.to_string(),
        user_agent,
        referer,
        request_id: None,
        tls_version: None,
        tls_cipher: None,
    }).await;

    Ok(response)
}
```

### 15.6 日志轮转

```rust
impl AccessLogger {
    async fn write_to_file(config: &AccessLogConfig, tenant: &str, line: &str) {
        let path = if config.per_tenant {
            config.dir.join("tenants").join(format!("{tenant}.access.log"))
        } else {
            config.dir.join("access.log")
        };

        // 追加写入
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .unwrap_or_else(|_| {
                // 目录不存在则创建
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await
                    .ok()
                    .unwrap()
            });

        let _ = file.write_all(format!("{line}\n").as_bytes()).await;

        // 检查文件大小，触发轮转
        if let Ok(metadata) = file.metadata().await {
            let size_mb = metadata.len() / (1024 * 1024);
            if size_mb >= config.max_size_mb {
                Self::rotate(&path, config.max_files, config.compress).await;
            }
        }
    }

    async fn rotate(path: &Path, max_files: usize, compress: bool) {
        // 1. 关闭当前文件句柄
        // 2. 重命名 access.log → access.YYYY-MM-DD.log
        // 3. 如果 compress=true，gzip 压缩
        // 4. 删除超过 max_files 的旧文件
    }
}
```

### 15.7 管理 API 集成

```bash
# 查看日志配置
GET /admin/access-log/config

# 动态修改日志配置（热更新）
PUT /admin/access-log/config
{
  "format": "clf",
  "exclude_paths": ["/health", "/metrics"]
}

# 查看指定租户的日志（尾部）
GET /admin/tenants/user1/logs?lines=100

# 全局日志统计
GET /admin/stats
{
  "total_requests": 125430,
  "by_tenant": { "user1": 50000, "user2": 75430 },
  "by_status": { "200": 120000, "404": 3000, "500": 430 },
  "avg_latency_ms": 15.3,
  "p99_latency_ms": 120
}
```

### 15.8 与现有日志系统的关系

| | raisfast 业务日志 (tracing) | proxy access log |
|---|---|---|
| 目的 | 记录应用内部行为（错误、调试信息） | 记录每个 HTTP 请求的元数据 |
| 格式 | tracing 结构化日志 | JSON / CLF |
| 存放 | `{log_dir}/app.log` | `{access_log.dir}/access.log` |
| 消费者 | 开发者调试 | 运维监控、审计、计费 |
| 生成方 | 业务进程 | proxy 进程 |

两者独立，互不影响。

---

## 16. 待完善功能清单

### 16.1 P0 — 性能与稳定性（必须做）

#### 16.1.1 后端连接池复用

当前设计每次请求新建连接（`UnixStream::connect` / `TcpStream::connect`），虽然有 keep-alive 但没有跨请求复用。

**目标**：维护一个 per-backend 连接池，避免重复握手。

```rust
use dashmap::DashMap;

struct ConnectionPool {
    pools: DashMap<String, mpsc::Sender<PooledConnection>>,
    max_idle_per_host: usize,    // 默认 8
    idle_timeout: Duration,      // 默认 90s
}

struct PooledConnection {
    sender: hyper::client::conn::http1::SendRequest<Body>,
    created_at: Instant,
}
```

**效果**：Unix socket 连接建立虽然快（~0.01ms），但池化后省掉 syscall 开销，高并发下差异明显。

---

#### 16.1.2 Per-Tenant 限流

防止单个租户占用全部带宽，影响其他租户。

```toml
# tenant 配置
[tenant]
# 每秒最大请求数
rate_limit_rps = 100
# 最大并发连接数
max_concurrent_connections = 50
# 每月流量配额（GB）
monthly_bandwidth_gb = 100
```

**实现**：token bucket 算法（已有 `RateLimiterSet` 模式可复用）。

---

#### 16.1.3 全局连接数限制

防止连接数爆增导致 OOM。

```toml
# proxy.toml
[proxy]
# 最大并发连接数（超出排队）
max_connections = 10000
# 排队超时（超出返回 503）
queue_timeout_ms = 5000
# 单 IP 最大连接数
max_connections_per_ip = 100
```

---

#### 16.1.4 请求超时

防止慢后端占住 proxy 连接。

```toml
# proxy.toml 全局默认
[proxy]
# 连接后端超时
connect_timeout_ms = 5000
# 读取后端响应超时
read_timeout_ms = 30000
# 总请求超时（含 body 传输）
request_timeout_ms = 60000

# per-tenant 覆盖
[tenant]
connect_timeout_ms = 3000
read_timeout_ms = 10000
```

---

### 16.2 P1 — 运维便利（应该做）

#### 16.2.1 自定义错误页

502/503/504 时展示品牌化的错误页面，而非裸 HTTP 状态码。

```toml
# proxy.toml
[error_pages]
# 错误页 HTML 目录
dir = "/etc/raisfast/error-pages"
# 默认错误页（按状态码命名：502.html, 503.html, 504.html）
# 缺失时使用内置简约页面
```

内置默认错误页（编译进二进制，零配置也可用）：

```html
<!-- 内置 503 页面 -->
<html>
<head><title>Service Temporarily Unavailable</title></head>
<body>
  <h1>Site Under Maintenance</h1>
  <p>We'll be back shortly.</p>
  <hr>
  <small>Powered by raisfast</small>
</body>
</html>
```

---

#### 16.2.2 IP 黑白名单

防爬虫 / DDoS / 恶意请求。

```toml
# proxy.toml
[ip_filter]
# 黑名单（优先级高于白名单）
blacklist = ["1.2.3.0/24", "5.6.7.8"]
# 白名单（设置后仅允许这些 IP）
# whitelist = ["10.0.0.0/8", "172.16.0.0/12"]
# 自动封禁：60 秒内 404 超过 100 次的 IP 自动封禁 1 小时
auto_ban_threshold = 100
auto_ban_window_secs = 60
auto_ban_duration_secs = 3600
```

**实现**：CIDR 匹配用 `ipnet` crate（轻量），封禁列表存 `DashMap<IpAddr, BanInfo>`。

---

#### 16.2.3 响应压缩（gzip / brotli）

减少出口带宽，特别是 JSON API 响应。

```toml
# proxy.toml
[compression]
enabled = true
# 压缩算法优先级
algorithms = ["brotli", "gzip"]
# 最小压缩阈值（字节）
min_size = 1024
# 压缩级别（1-9）
level = 4
# 不压缩的 Content-Type
exclude_types = ["image/", "video/", "application/zip"]
```

**实现**：`async-compression` crate（tokio + brotli + gzip）。响应体流式压缩，不缓冲到内存。

---

#### 16.2.4 响应缓存

静态资源 + GET 请求短期缓存，减少后端压力。

```toml
# proxy.toml
[cache]
enabled = true
# 缓存存储："memory" | "disk"
storage = "memory"
# 最大缓存大小（MB）
max_size_mb = 256
# 默认 TTL（秒）
default_ttl = 60
# 缓存条件：仅缓存 GET + 200 响应
methods = ["GET"]
statuses = [200, 301, 302]
# 缓存键：host + path + query
# 不缓存带 Authorization 头的请求
skip_auth_requests = true
```

**缓存键**：`{host}:{path}?{query}` → SHA256 hash
**缓存淘汰**：LRU（`moka` crate 已在依赖中）
**缓存失效**：通过管理 API 手动清除 `POST /admin/cache/purge?tenant=user1`

---

#### 16.2.5 实时指标（Prometheus 格式）

在 access log 基础上，提供实时聚合指标。

```bash
GET /metrics  # 已有 metrics endpoint，扩展 proxy 维度
```

新增指标维度：

| 指标 | 类型 | 标签 |
|------|------|------|
| `proxy_requests_total` | Counter | tenant, status, method |
| `proxy_request_duration_seconds` | Histogram | tenant |
| `proxy_active_connections` | Gauge | tenant |
| `proxy_backend_health` | Gauge | tenant, backend |
| `proxy_tls_handshake_seconds` | Histogram | tls_version |
| `proxy_bytes_sent` | Counter | tenant |
| `proxy_bytes_received` | Counter | tenant |
| `proxy_acme_cert_expiry_timestamp` | Gauge | domain |

---

#### 16.2.6 请求 ID 透传

全链路追踪，proxy → backend → 日志 全程同一个 request_id。

```
客户端请求 → proxy 生成 X-Request-ID: req-a1b2c3d4
    → 转发时透传 X-Request-ID
    → 后端 raisfast 读取 X-Request-ID（已有 request_id 中间件）
    → access log 记录 request_id
    → 业务 log 记录 request_id
```

**实现**：proxy 入口检查 `X-Request-ID`，缺失则生成 UUID v7，透传给后端。

---

### 16.3 P2 — 高级特性（按需做）

#### 16.3.1 慢请求日志

超阈值的请求单独记录，用于性能排查。

```toml
# proxy.toml
[slow_log]
enabled = true
# 慢请求阈值（ms）
threshold_ms = 3000
# 独立文件
file = "/var/lib/raisfast/proxy/logs/slow.log"
```

---

#### 16.3.2 灰度/蓝绿部署

按比例将流量切换到新版后端，零风险上线。

```toml
# tenant 配置
[tenant]
# 稳定版后端
backend = "unix:/run/raisfast/user1.sock"
# 灰度版后端
canary_backend = "unix:/run/raisfast/user1-canary.sock"
# 灰度流量比例（0-100）
canary_weight = 10
```

**实现**：加权随机（`rand::thread_rng()` < canary_weight / 100），逐步调大 weight。

---

#### 16.3.3 DNS 缓存

TCP 后端场景下，避免每次请求都做 DNS 解析。

```toml
# proxy.toml
[dns]
# DNS 缓存 TTL（秒）
cache_ttl = 300
# DNS 服务器（默认系统配置）
# servers = ["8.8.8.8", "1.1.1.1"]
```

**实现**：`DashMap<String, (SocketAddr, Instant)>` 简单缓存，过期重新解析。

---

#### 16.3.4 负载均衡（一个租户多后端）

单租户扩展到多实例，proxy 内置负载均衡。

```toml
# tenant 配置
[[tenant.backends]]
addr = "unix:/run/raisfast/user1-a.sock"
weight = 50

[[tenant.backends]]
addr = "unix:/run/raisfast/user1-b.sock"
weight = 50
```

**策略**：
- `round-robin` — 轮询（默认）
- `weighted` — 加权随机
- `least-connections` — 最少连接数

---

### 16.4 完善后的模块结构

```
src/proxy/
├── mod.rs              # 模块入口
├── config.rs           # 配置加载
├── router.rs           # 路由表
├── proxy.rs            # HTTP 反向代理核心
├── pool.rs             # 后端连接池 ← 新增
├── limiter.rs          # 限流 + 连接数限制 ← 新增
├── ip_filter.rs        # IP 黑白名单 + 自动封禁 ← 新增
├── compression.rs      # 响应压缩 ← 新增
├── cache.rs            # 响应缓存 ← 新增
├── error_page.rs       # 自定义错误页 ← 新增
├── tls.rs              # TLS 终结
├── acme.rs             # ACME 自动证书
├── admin.rs            # 管理 API
├── health.rs           # 健康检查
├── access_log.rs       # 访问日志
├── slow_log.rs         # 慢请求日志 ← 新增
├── watcher.rs          # 配置热加载
└── metrics.rs          # Prometheus 指标 ← 新增
```

### 16.5 实施优先级

| 阶段 | 功能 | 预估工作量 |
|------|------|-----------|
| Phase 3（与现有合并） | 连接池复用 + 超时 + 连接数限制 + 请求 ID 透传 | 3 天 |
| Phase 3（与现有合并） | 自定义错误页 | 0.5 天 |
| Phase 4 | Per-tenant 限流 + IP 黑白名单 | 2 天 |
| Phase 4 | 响应压缩 | 1 天 |
| Phase 4 | 响应缓存 | 2 天 |
| Phase 4 | 实时指标 | 1 天 |
| Phase 5 | 慢请求日志 | 0.5 天 |
| Phase 5 | 灰度/蓝绿部署 | 1 天 |
| Phase 5 | 负载均衡 | 2 天 |
| Phase 5 | DNS 缓存 | 0.5 天 |
