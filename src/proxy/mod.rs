//! Built-in reverse proxy module (multi-tenant).
//!
//! Replaces nginx/caddy for single-binary multi-tenant deployment.
//! Routes requests to backend instances (Unix socket / TCP) based on Host header or path prefix.

pub mod config;
pub mod handler;
pub mod router;

use std::sync::Arc;

use hyper::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use config::ProxyConfig;
use router::RouterTable;

/// Start the proxy server.
pub async fn start(config_path: &str) -> anyhow::Result<()> {
    let path = std::path::Path::new(config_path);
    let proxy_config = ProxyConfig::load(path)?;
    let addr = proxy_config.http_addr()?;

    let router = Arc::new(RouterTable::new());

    let tenants = config::load_all_tenants(&proxy_config.proxy.tenants_dir);
    let tenant_sections: Vec<_> = tenants.iter().map(|(_, t)| t.tenant.clone()).collect();
    router.load_from_tenants(&tenant_sections);

    tracing::info!(
        addr = %addr,
        tenants = tenant_sections.len(),
        routes = router.len(),
        "starting raisfast proxy"
    );
    println!("raisfast proxy listening on http://{addr}");
    println!(
        "registered {} routes for {} tenants",
        router.len(),
        tenant_sections.len()
    );

    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, remote_addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let router = router.clone();
        let client_ip = remote_addr.ip().to_string();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let router = router.clone();
                let client_ip = client_ip.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(
                        handler::handle_proxy_request(req, &router, &client_ip).await,
                    )
                }
            });

            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });
    }
}
