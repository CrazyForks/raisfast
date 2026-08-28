//! `tcp-listen` connector — accept inbound TCP connections for listen-mode
//! channels (integration.md §5.1/§5.2). Frame disassembly per
//! `stream_config.framing`: `"line"` (default, `\n`-delimited) or
//! `"lenprefixed"` (4-byte big-endian length + payload). Each frame is one
//! envelope through the standard pipeline; no wire-level ack (`none`).

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::supervisor::{ConnectionSink, StreamConnector};

pub struct TcpListenConnector;

#[async_trait::async_trait]
impl StreamConnector for TcpListenConnector {
    async fn run(&self, ch: Arc<ItgChannel>, sink: ConnectionSink) -> anyhow::Result<()> {
        run(ch, sink).await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn framing_kind(ch: &ItgChannel) -> String {
    ch.stream_config
        .as_ref()
        .and_then(|c| c.get("framing"))
        .and_then(Value::as_str)
        .unwrap_or("line")
        .to_string()
}

async fn run(ch: Arc<ItgChannel>, sink: ConnectionSink) -> AppResult<()> {
    let addr = ch
        .endpoint
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("listen requires host:port endpoint".into()))?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("tcp bind {addr}: {e}")))?;
    tracing::info!(channel = %ch.channel_key, addr, "tcp listener bound");

    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(channel = %ch.channel_key, error = %e, "tcp accept failed");
                continue;
            }
        };
        let ch = Arc::clone(&ch);
        let sink = sink.clone();
        let framing = framing_kind(&ch);
        tokio::spawn(async move {
            tracing::debug!(peer = %peer, "tcp connection accepted");
            if let Err(err) = serve_connection(socket, &ch, &sink, &framing).await {
                tracing::warn!(peer = %peer, error = %err, "tcp connection ended with error");
            }
        });
    }
}

async fn serve_connection(
    socket: tokio::net::TcpStream,
    ch: &ItgChannel,
    sink: &ConnectionSink,
    framing: &str,
) -> AppResult<()> {
    let (read_half, mut write_half) = socket.into_split();
    // Best-effort welcome banner; clients may ignore.
    let _ = write_half
        .write_all(format!("{}\n", ch.channel_key).as_bytes())
        .await;

    match framing {
        "line" => {
            let mut reader = tokio::io::BufReader::new(read_half);
            use tokio::io::AsyncBufReadExt;
            loop {
                let mut line = Vec::new();
                let consumed = reader
                    .read_until(b'\n', &mut line)
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("tcp read: {e}")))?;
                if consumed == 0 {
                    return Ok(()); // peer closed
                }
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.is_empty() {
                    continue; // blank line heartbeat
                }
                let outcome = sink.submit(ch, line).await;
                let _ = outcome;
            }
        }
        "lenprefixed" => {
            let mut read_half = read_half;
            loop {
                let mut len_buf = [0u8; 4];
                if read_half.read_exact(&mut len_buf).await.is_err() {
                    return Ok(());
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                if len > 8 * 1024 * 1024 {
                    return Err(AppError::BadRequest("lenprefixed frame > 8MB".into()));
                }
                let mut payload = vec![0u8; len];
                if read_half.read_exact(&mut payload).await.is_err() {
                    return Ok(());
                }
                let outcome = sink.submit(ch, payload).await;
                let _ = outcome;
            }
        }
        other => Err(AppError::BadRequest(format!(
            "tcp framing must be line | lenprefixed — got '{other}'"
        ))),
    }
}
