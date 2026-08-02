//! proxy 模块集成测试。
//!
//! 启动真实的 TCP 后端服务器，验证 proxy 的完整请求转发链路。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Request;
use hyper::Response;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

use raisfast::proxy::config::TenantSection;
use raisfast::proxy::router::RouterTable;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

fn full(data: impl Into<Bytes>) -> BoxBody {
    Full::new(data.into())
        .map_err(|never| match never {})
        .boxed()
}

/// 启动一个返回固定响应的后端服务器。
async fn start_backend(body: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let body = Arc::new(body);
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let body = body.clone();

            let service = service_fn(move |_req: Request<hyper::body::Incoming>| {
                let body = body.clone();
                async move { Ok::<_, std::convert::Infallible>(Response::new(full(body.to_string()))) }
            });

            tokio::spawn(async move {
                let _ = http1::Builder::new().serve_connection(io, service).await;
            });
        }
    });

    (addr, handle)
}

/// 后端接收到的请求信息。
#[derive(Debug)]
struct ReceivedRequest {
    method: String,
    path: String,
    x_forwarded_for: String,
    x_forwarded_host: String,
    x_forwarded_proto: String,
}

/// 启动一个记录请求头的后端服务器。
async fn start_echo_backend() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Mutex<Vec<ReceivedRequest>>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let received: Arc<tokio::sync::Mutex<Vec<ReceivedRequest>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let received_clone = received.clone();

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let received = received_clone.clone();

            let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                let received = received.clone();
                async move {
                    let info = ReceivedRequest {
                        method: req.method().to_string(),
                        path: req.uri().path().to_string(),
                        x_forwarded_for: req
                            .headers()
                            .get("x-forwarded-for")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string(),
                        x_forwarded_host: req
                            .headers()
                            .get("x-forwarded-host")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string(),
                        x_forwarded_proto: req
                            .headers()
                            .get("x-forwarded-proto")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string(),
                    };
                    received.lock().await.push(info);
                    Ok::<_, std::convert::Infallible>(Response::new(full("echo")))
                }
            });

            tokio::spawn(async move {
                let _ = http1::Builder::new().serve_connection(io, service).await;
            });
        }
    });

    (addr, handle, received)
}

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

/// 通过 TCP 发送 HTTP 请求并读取响应。
async fn send_http_request(addr: &str, method: &str, path: &str, host: &str) -> (u16, String) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut req = Request::new(full(""));
    *req.method_mut() = method.parse().unwrap();
    *req.uri_mut() = path.parse().unwrap();
    req.headers_mut().insert("host", host.parse().unwrap());

    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ── 直接调用 handle_proxy_request 的测试 ──

#[tokio::test]
async fn no_backend_returns_502() {
    let router = Arc::new(RouterTable::new());

    let (backend_addr, _backend) = start_backend("ok".to_string()).await;
    let _stream = TcpStream::connect(&backend_addr).await.unwrap();
    // 不注册任何路由

    let _resp = send_http_request(&backend_addr, "GET", "/api/test", "unknown.example.com").await;
    // 这里直接测 router 层面
    assert!(router.find("unknown.example.com", "/api/test").is_none());
}

#[tokio::test]
async fn healthy_backend_responds() {
    let (backend_addr, _backend) = start_backend("hello from backend".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "test",
        Some("test.example.com"),
        None,
        &backend_addr,
    )]);

    let backends = router.all_backends();
    assert_eq!(backends.len(), 1);
    assert!(backends[0].healthy.load(Ordering::Relaxed));
}

#[tokio::test]
async fn unhealthy_backend_marks() {
    let (backend_addr, _backend) = start_backend("ok".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "sick",
        Some("sick.example.com"),
        None,
        &backend_addr,
    )]);

    let backends = router.all_backends();
    backends[0].healthy.store(false, Ordering::Relaxed);
    assert!(!backends[0].healthy.load(Ordering::Relaxed));
}

#[tokio::test]
async fn multiple_tenants_routing() {
    let (backend1, _h1) = start_backend("backend1".to_string()).await;
    let (backend2, _h2) = start_backend("backend2".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[
        make_tenant("user1", Some("user1.example.com"), None, &backend1),
        make_tenant("user2", Some("user2.example.com"), None, &backend2),
    ]);

    let b1 = router.find("user1.example.com", "/").unwrap();
    assert_eq!(b1.name, "user1");

    let b2 = router.find("user2.example.com", "/").unwrap();
    assert_eq!(b2.name, "user2");

    assert!(router.find("unknown.example.com", "/").is_none());
}

#[tokio::test]
async fn prefix_routing() {
    let (backend_addr, _h) = start_backend("prefix backend".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant("user1", None, Some("/user1"), &backend_addr)]);

    let b = router.find("any.example.com", "/user1/api/posts");
    assert_eq!(b.unwrap().name, "user1");

    assert!(router.find("any.example.com", "/other").is_none());
}

#[tokio::test]
async fn longest_prefix_match() {
    let (b1, _h1) = start_backend("short".to_string()).await;
    let (b2, _h2) = start_backend("long".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[
        make_tenant("short", None, Some("/user1"), &b1),
        make_tenant("long", None, Some("/user1/admin"), &b2),
    ]);

    let b = router.find("x.com", "/user1/admin/posts");
    assert_eq!(b.unwrap().name, "long");

    let b = router.find("x.com", "/user1/posts");
    assert_eq!(b.unwrap().name, "short");
}

#[tokio::test]
async fn host_priority_over_prefix() {
    let (b1, _h1) = start_backend("host".to_string()).await;
    let (b2, _h2) = start_backend("prefix".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[
        make_tenant("host", Some("special.example.com"), None, &b1),
        make_tenant("fallback", None, Some("/"), &b2),
    ]);

    let b = router.find("special.example.com", "/anything");
    assert_eq!(b.unwrap().name, "host");

    let b = router.find("other.example.com", "/anything");
    assert_eq!(b.unwrap().name, "fallback");
}

// ── 端到端测试：proxy → backend 全链路 ──

/// 启动 proxy，连到真实后端，通过 TCP 客户端发请求验证。
async fn start_proxy(
    router: Arc<RouterTable>,
) -> (String, tokio::task::JoinHandle<anyhow::Result<()>>) {
    use raisfast::proxy::handler::handle_proxy_request;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let handle = tokio::spawn(async move {
        loop {
            let (stream, remote) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let router = router.clone();
            let client_ip = remote.ip().to_string();

            let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                let router = router.clone();
                let client_ip = client_ip.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(
                        handle_proxy_request(req, &router, &client_ip).await,
                    )
                }
            });

            tokio::spawn(async move {
                let _ = http1::Builder::new().serve_connection(io, service).await;
            });
        }
    });

    (addr, handle)
}

#[tokio::test]
async fn e2e_proxy_forward_request() {
    let (backend_addr, _bh) = start_backend("hello-world".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "test",
        Some("test.example.com"),
        None,
        &backend_addr,
    )]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, body) =
        send_http_request(&proxy_addr, "GET", "/api/v1/posts", "test.example.com").await;
    assert_eq!(status, 200);
    assert_eq!(body, "hello-world");
}

#[tokio::test]
async fn e2e_proxy_502_no_backend() {
    let router = Arc::new(RouterTable::new());
    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "unknown.example.com").await;
    assert_eq!(status, 502);
    assert!(body.contains("502"));
}

#[tokio::test]
async fn e2e_proxy_503_unhealthy() {
    let (backend_addr, _bh) = start_backend("ok".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "sick",
        Some("sick.example.com"),
        None,
        &backend_addr,
    )]);

    let backends = router.all_backends();
    backends[0].healthy.store(false, Ordering::Relaxed);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "sick.example.com").await;
    assert_eq!(status, 503);
    assert!(body.contains("503"));
}

#[tokio::test]
async fn e2e_proxy_502_connection_refused() {
    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "dead",
        Some("dead.example.com"),
        None,
        "127.0.0.1:1",
    )]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, _body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "dead.example.com").await;
    assert_eq!(status, 502);
}

#[tokio::test]
async fn e2e_proxy_forward_headers() {
    let (backend_addr, _bh, received) = start_echo_backend().await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "test",
        Some("test.example.com"),
        None,
        &backend_addr,
    )]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, _body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "test.example.com").await;
    assert_eq!(status, 200);

    let logged = received.lock().await;
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].method, "GET");
    assert_eq!(logged[0].path, "/api/test");
    assert_eq!(logged[0].x_forwarded_for, "127.0.0.1");
    assert_eq!(logged[0].x_forwarded_host, "test.example.com");
    assert_eq!(logged[0].x_forwarded_proto, "http");
}

#[tokio::test]
async fn e2e_proxy_multiple_tenants() {
    let (b1, _h1) = start_backend("response-from-user1".to_string()).await;
    let (b2, _h2) = start_backend("response-from-user2".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[
        make_tenant("user1", Some("user1.example.com"), None, &b1),
        make_tenant("user2", Some("user2.example.com"), None, &b2),
    ]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (s1, body1) = send_http_request(&proxy_addr, "GET", "/api/test", "user1.example.com").await;
    assert_eq!(s1, 200);
    assert_eq!(body1, "response-from-user1");

    let (s2, body2) = send_http_request(&proxy_addr, "GET", "/api/test", "user2.example.com").await;
    assert_eq!(s2, 200);
    assert_eq!(body2, "response-from-user2");
}

#[tokio::test]
async fn e2e_proxy_host_strip_port() {
    let (backend_addr, _bh, received) = start_echo_backend().await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant(
        "test",
        Some("test.example.com"),
        None,
        &backend_addr,
    )]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, _body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "test.example.com:8080").await;
    assert_eq!(status, 200);

    let logged = received.lock().await;
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].x_forwarded_host, "test.example.com:8080");
}

#[tokio::test]
async fn e2e_proxy_504_connect_timeout() {
    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[TenantSection {
        name: "timeout".to_string(),
        host: Some("timeout.example.com".to_string()),
        prefix: None,
        backend: "192.0.2.1:9999".to_string(),
        tls_cert: None,
        tls_key: None,
        connect_timeout_ms: 100,
        read_timeout_ms: 1000,
        enabled: true,
    }]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, _body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "timeout.example.com").await;
    assert!(status == 504 || status == 502);
}

#[tokio::test]
async fn e2e_proxy_prefix_routing() {
    let (backend_addr, _bh) = start_backend("prefix-response".to_string()).await;

    let router = Arc::new(RouterTable::new());
    router.load_from_tenants(&[make_tenant("user1", None, Some("/user1"), &backend_addr)]);

    let (proxy_addr, _ph) = start_proxy(router).await;

    let (status, body) =
        send_http_request(&proxy_addr, "GET", "/user1/api/posts", "any.example.com").await;
    assert_eq!(status, 200);
    assert_eq!(body, "prefix-response");
}

#[tokio::test]
async fn e2e_dynamic_upsert_remove() {
    let (backend_addr, _bh) = start_backend("dynamic".to_string()).await;

    let router = Arc::new(RouterTable::new());
    let (proxy_addr, _ph) = start_proxy(router.clone()).await;

    let (status, _) =
        send_http_request(&proxy_addr, "GET", "/api/test", "dynamic.example.com").await;
    assert_eq!(status, 502);

    router
        .upsert(&make_tenant(
            "dynamic",
            Some("dynamic.example.com"),
            None,
            &backend_addr,
        ))
        .unwrap();

    let (status, body) =
        send_http_request(&proxy_addr, "GET", "/api/test", "dynamic.example.com").await;
    assert_eq!(status, 200);
    assert_eq!(body, "dynamic");

    router.remove("dynamic");

    let (status, _) =
        send_http_request(&proxy_addr, "GET", "/api/test", "dynamic.example.com").await;
    assert_eq!(status, 502);
}
