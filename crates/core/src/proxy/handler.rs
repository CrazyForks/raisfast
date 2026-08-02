//! HTTP reverse proxy core.
//!
//! Receives frontend requests, looks up backends via the routing table, and streams requests and responses.

use std::sync::Arc;

use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Request;
use hyper::Response;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::net::UnixStream;

use crate::proxy::router::{Backend, BackendAddr, RouterTable};

type ResBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

fn full(data: impl Into<Bytes>) -> ResBody {
    Full::new(data.into())
        .map_err(|never| match never {})
        .boxed()
}

/// Proxy error types.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("no backend found for this request")]
    NoBackend,
    #[error("backend {0} is unhealthy")]
    Unhealthy(String),
    #[error("connect timeout")]
    ConnectTimeout,
    #[error("read timeout")]
    ReadTimeout,
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("handshake failed: {0}")]
    HandshakeFailed(String),
    #[error("request failed: {0}")]
    RequestFailed(String),
}

fn error_html(status: u16, message: &str) -> Response<ResBody> {
    let html = format!(
        "<html><head><title>{status}</title></head>\
         <body><h1>{status}</h1><p>{message}</p>\
         <hr><small>Powered by raisfast proxy</small></body></html>"
    );
    Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .body(full(html))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(502)
                .body(full("bad gateway"))
                .unwrap()
        })
}

impl From<ProxyError> for Response<ResBody> {
    fn from(e: ProxyError) -> Response<ResBody> {
        match &e {
            ProxyError::NoBackend => error_html(502, "no backend found"),
            ProxyError::Unhealthy(name) => error_html(503, &format!("backend {name} is unhealthy")),
            ProxyError::ConnectTimeout => error_html(504, "connect timeout"),
            ProxyError::ReadTimeout => error_html(504, "read timeout"),
            ProxyError::ConnectionFailed(msg) => error_html(502, msg),
            ProxyError::HandshakeFailed(msg) => error_html(502, msg),
            ProxyError::RequestFailed(msg) => error_html(502, msg),
        }
    }
}

/// Handle a proxy request.
pub async fn handle_proxy_request(
    req: Request<Incoming>,
    router: &Arc<RouterTable>,
    client_ip: &str,
) -> Response<ResBody> {
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    let path = req.uri().path();

    let backend = match router.find(host, path) {
        Some(b) => b,
        None => return ProxyError::NoBackend.into(),
    };

    if !backend.healthy.load(std::sync::atomic::Ordering::Relaxed) {
        return ProxyError::Unhealthy(backend.name.clone()).into();
    }

    match proxy_to_backend(req, &backend, client_ip).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(tenant = %backend.name, error = %e, "proxy request failed");
            e.into()
        }
    }
}

async fn proxy_to_backend(
    req: Request<Incoming>,
    backend: &Backend,
    client_ip: &str,
) -> Result<Response<ResBody>, ProxyError> {
    let read_timeout = backend.read_timeout;

    match &backend.addr {
        BackendAddr::UnixSocket(path) => {
            let stream = tokio::time::timeout(backend.connect_timeout, UnixStream::connect(path))
                .await
                .map_err(|_| ProxyError::ConnectTimeout)?
                .map_err(|e| ProxyError::ConnectionFailed(e.to_string()))?;
            let io = TokioIo::new(stream);
            do_forward(req, io, client_ip, read_timeout).await
        }
        BackendAddr::Tcp(addr) => {
            let stream = tokio::time::timeout(backend.connect_timeout, TcpStream::connect(addr))
                .await
                .map_err(|_| ProxyError::ConnectTimeout)?
                .map_err(|e| ProxyError::ConnectionFailed(e.to_string()))?;
            let io = TokioIo::new(stream);
            do_forward(req, io, client_ip, read_timeout).await
        }
    }
}

async fn do_forward(
    req: Request<Incoming>,
    io: TokioIo<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static>,
    client_ip: &str,
    read_timeout: std::time::Duration,
) -> Result<Response<ResBody>, ProxyError> {
    let (mut sender, conn) = tokio::time::timeout(read_timeout, http1::handshake(io))
        .await
        .map_err(|_| ProxyError::ReadTimeout)?
        .map_err(|e| ProxyError::HandshakeFailed(e.to_string()))?;

    tokio::spawn(async move {
        let _ = conn.await;
    });

    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();
    let host_header = headers.get("host").cloned();
    let body = req.into_body();

    let mut proxy_req = Request::new(body);
    *proxy_req.method_mut() = method;
    *proxy_req.uri_mut() = uri;
    *proxy_req.headers_mut() = headers;

    proxy_req.headers_mut().insert(
        hyper::header::HeaderName::from_static("x-forwarded-for"),
        client_ip
            .parse()
            .unwrap_or_else(|_| "unknown".parse().unwrap()),
    );
    proxy_req.headers_mut().insert(
        hyper::header::HeaderName::from_static("x-forwarded-proto"),
        "http".parse().unwrap(),
    );
    if let Some(host) = host_header {
        proxy_req.headers_mut().insert(
            hyper::header::HeaderName::from_static("x-forwarded-host"),
            host.clone(),
        );
    }

    let resp = sender
        .send_request(proxy_req)
        .await
        .map_err(|e| ProxyError::RequestFailed(e.to_string()))?;

    Ok(resp.map(|b| b.boxed()))
}
