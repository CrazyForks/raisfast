use anyhow::Result;

const DEFAULT_SERVER: &str = "tunnel.raisfast.com";

pub async fn run(
    local_port: u16,
    local_host: Option<&str>,
    server: Option<&str>,
    port: Option<u16>,
    secret: Option<&str>,
) -> Result<()> {
    let server = server.unwrap_or(DEFAULT_SERVER);
    let local_host = local_host.unwrap_or("localhost");

    println!("Connecting to {}...", server);

    let client = bore_cli::client::Client::new(
        local_host,
        local_port,
        server,
        port.unwrap_or(0),
        secret,
    )
    .await?;

    let remote_port = client.remote_port();

    println!();
    println!("  Tunnel established!");
    println!("  Public URL: http://{}:{}", server, remote_port);
    println!("  Local:      http://{}:{}", local_host, local_port);
    println!();
    println!("  Press Ctrl+C to stop.");
    println!();

    client.listen().await?;

    Ok(())
}
