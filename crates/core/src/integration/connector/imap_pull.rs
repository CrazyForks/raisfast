//! `imap` pull connector — native email inbound with **mark-read**
//! semantics (integration.md §2 "邮箱收信", §5.3 pull table).
//!
//! Layer stack: `mode=pull · transport=imap · framing=mime · codec=email`.
//!
//! Channel config:
//! - `endpoint`: `imap://host:port` (or `imaps://` to force TLS)
//! - `pull_config`:
//!   ```json
//!   {
//!     "folder": "INBOX",       // mailbox to watch
//!     "ssl": true,             // STARTTLS-style implicit TLS (default true)
//!     "batch": 50,             // max messages per drain round
//!     "idle_secs": 0           // >0: IDLE up to N seconds waiting for new
//!   }                          //      mail after the drain (29min cap)
//!   ```
//! - `credentials` (vault-sealed): `{ "username": "...", "password": "..." }`
//!
//! Semantics: UNSEEN messages are pushed through the standard pipeline
//! (`external_id = $.message_id` via mapping — receipts idempotency applies);
//! **delivered/duplicate → `STORE \Seen`**, failures stay unseen so the next
//! round re-fetches them (dedup absorbs the overlap). No local cursor —
//! the mailbox IS the state (§5.3 mark-read row).

use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::connector::PullSummary;
use crate::integration::pipeline::Pipeline;
use crate::integration::verify::InboundHttpRequest;

struct ImapConfig {
    host: String,
    port: u16,
    ssl: bool,
    folder: String,
    batch: usize,
    idle_secs: u64,
}

fn parse_config(channel: &ItgChannel) -> AppResult<ImapConfig> {
    let endpoint = channel
        .endpoint
        .as_deref()
        .filter(|s| s.starts_with("imap://") || s.starts_with("imaps://"))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "imap channel '{}' needs an imap:// or imaps:// endpoint",
                channel.channel_key
            ))
        })?;
    let forced_tls = endpoint.starts_with("imaps://");
    let authority = endpoint
        .trim_start_matches("imap://")
        .trim_start_matches("imaps://")
        .trim_end_matches('/')
        .to_string();
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| {
                AppError::BadRequest(format!("invalid imap endpoint port: {authority}"))
            })?,
        ),
        None => (authority, if forced_tls { 993 } else { 143 }),
    };

    let cfg = channel.pull_config.clone().unwrap_or(Value::Null);
    let ssl = cfg.get("ssl").and_then(Value::as_bool).unwrap_or(true) || forced_tls;
    Ok(ImapConfig {
        host,
        port,
        ssl,
        folder: cfg
            .get("folder")
            .and_then(Value::as_str)
            .unwrap_or("INBOX")
            .to_string(),
        batch: cfg.get("batch").and_then(Value::as_u64).unwrap_or(50) as usize,
        // RFC2177: clients MUST NOT idle longer than 29 minutes.
        idle_secs: cfg
            .get("idle_secs")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(29 * 60),
    })
}

/// Unseal IMAP credentials `{username, password}` from the vault.
///
/// # Errors
///
/// `AppError` when credentials are missing/malformed or the vault is sealed.
pub fn pull_credentials(
    channel: &ItgChannel,
    vault: Option<&crate::integration::vault::Vault>,
) -> AppResult<(String, String)> {
    let Some(sealed) = channel.credentials.as_deref() else {
        return Err(AppError::BadRequest(format!(
            "imap channel '{}' requires credentials {{username, password}}",
            channel.channel_key
        )));
    };
    let Some(vault) = vault else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "imap channel has credentials but vault is sealed"
        )));
    };
    let json = vault.unseal(sealed)?;
    let v: Value = json
        .parse()
        .map_err(|_| AppError::BadRequest("imap credentials must be a JSON object".into()))?;
    let username = v
        .get("username")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("imap credentials missing 'username'".into()))?
        .to_string();
    let password = v
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((username, password))
}

/// Combined futures-io stream trait (trait objects cannot list two
/// non-auto traits; this alias bundles them for `Box<dyn …>`).
trait ImapIo: futures::AsyncRead + futures::AsyncWrite + Unpin + Send + std::fmt::Debug {}
impl<T> ImapIo for T where
    T: futures::AsyncRead + futures::AsyncWrite + Unpin + Send + std::fmt::Debug
{
}

type ImapStream = Box<dyn ImapIo>;
type ImapSession = async_imap::Session<ImapStream>;

async fn connect(config: &ImapConfig, username: &str, password: &str) -> AppResult<ImapSession> {
    use tokio_util::compat::TokioAsyncReadCompatExt;
    let tcp = tokio::net::TcpStream::connect((config.host.as_str(), config.port))
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "imap connect {}:{}: {e}",
                config.host,
                config.port
            ))
        })?;
    // async-imap speaks the futures-io traits; compat-wrap the tokio stream
    // (and the tokio-native-tls handshake result) to bridge the two worlds.
    let stream: ImapStream = if config.ssl {
        let connector = tokio_native_tls::TlsConnector::from(
            native_tls::TlsConnector::new()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("tls connector: {e}")))?,
        );
        let tls = connector.connect(&config.host, tcp).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "imap tls handshake {}:{}: {e}",
                config.host,
                config.port
            ))
        })?;
        Box::new(tls.compat())
    } else {
        Box::new(tcp.compat())
    };
    let client = async_imap::Client::new(stream);
    let session = client.login(username, password).await.map_err(|(e, _)| {
        AppError::Internal(anyhow::anyhow!("imap login for '{username}': {e}"))
    })?;
    Ok(session)
}

/// Fetch + process one drain round. Returns how many messages were seen.
async fn drain_round(
    session: &mut ImapSession,
    pipeline: &Pipeline,
    channel: &ItgChannel,
    config: &ImapConfig,
    summary: &mut PullSummary,
) -> AppResult<usize> {
    let mut uids: Vec<u32> = session
        .uid_search("UNSEEN")
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("imap uid_search UNSEEN: {e}")))?
        .into_iter()
        .collect();
    uids.sort_unstable();
    uids.truncate(config.batch);

    let ch = std::sync::Arc::new(channel.clone());
    let mut processed = 0_usize;
    for uid in uids {
        // Collect the raw RFC5322 bytes first — the fetch stream borrows the
        // session, and the pipeline call below must not hold that borrow.
        // BODY.PEEK[] (not RFC822): PEEK does NOT auto-set \Seen, so the
        // mark-read decision stays ours — failures remain UNSEEN (§5.3).
        let body: Vec<u8> = {
            use futures::StreamExt;
            let mut fetches = session
                .uid_fetch(uid.to_string(), "(BODY.PEEK[])")
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("imap uid_fetch {uid}: {e}")))?;
            let mut body = Vec::new();
            while let Some(item) = fetches.next().await {
                let f =
                    item.map_err(|e| AppError::Internal(anyhow::anyhow!("imap fetch item: {e}")))?;
                if let Some(b) = f.body() {
                    body = b.to_vec();
                }
            }
            body
        };
        if body.is_empty() {
            tracing::warn!(
                channel = %channel.channel_key,
                uid,
                "imap fetch returned empty body — skipped"
            );
            continue;
        }
        summary.fetched += 1;
        processed += 1;

        let req = InboundHttpRequest {
            method: "POST".into(),
            query: String::new(),
            headers: Vec::new(),
            body,
        };
        let outcome = pipeline.run_push(&ch, &req).await;
        let settled = outcome.duplicate || outcome.delivered;
        if outcome.duplicate {
            summary.duplicates += 1;
        } else if outcome.delivered {
            summary.delivered += 1;
        } else {
            summary.failed += 1;
        }

        // mark-read: only settle the flag once the pipeline is done with the
        // message — failures stay UNSEEN and are re-fetched next round
        // (receipts dedup absorbs the overlap, §5.3).
        if settled {
            use futures::StreamExt;
            let mut store = session
                .uid_store(uid.to_string(), "+FLAGS (\\Seen)")
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("imap uid_store \\Seen {uid}: {e}"))
                })?;
            // The untagged FETCH responses stream must be drained.
            while let Some(item) = store.next().await {
                item.map_err(|e| AppError::Internal(anyhow::anyhow!("imap store item: {e}")))?;
            }
        }
    }
    summary.pages += 1;
    Ok(processed)
}

/// Identify the client via the IMAP ID extension (RFC 2971).
///
/// NetEase servers (163 / 126 / 188) reject subsequent commands with
/// `SELECT Unsafe Login. Please contact kefu@188.com` when the client does
/// not identify itself — a vendor anti-abuse policy. Servers without the
/// extension reply BAD and ignore it, which we treat as success.
async fn send_imap_id(session: &mut ImapSession) -> AppResult<()> {
    use async_imap::imap_proto::{Response, Status};
    let request_id = session
        .run_command("ID (\"name\" \"RaisFast\" \"version\" \"1.0\")")
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("imap ID command: {e}")))?;
    while let Some(res) = session
        .read_response()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("imap ID response: {e}")))?
    {
        if let Response::Done {
            tag,
            status,
            information,
            ..
        } = res.parsed()
            && tag == &request_id
        {
            return match status {
                Status::Ok | Status::PreAuth => Ok(()),
                // Unsupported command (or rejected) — not fatal: only
                // NetEase requires it, others work either way.
                _ => {
                    tracing::debug!(
                        "imap ID not accepted: {} — continuing",
                        information.as_deref().unwrap_or_default()
                    );
                    Ok(())
                }
            };
        }
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "imap connection lost while awaiting ID response"
    )))
}

/// Execute one pull run for the channel (drain + optional IDLE window).
///
/// # Errors
///
/// Returns `AppError` on connect/login/protocol failures; the job logs it
/// and the next tick retries with a fresh connection.
pub async fn run(
    pipeline: &Pipeline,
    channel: &ItgChannel,
    username: &str,
    password: &str,
) -> AppResult<PullSummary> {
    let config = parse_config(channel)?;
    let mut session = connect(&config, username, password).await?;
    // RFC 2971 client identification — required by NetEase (163/126/188)
    // before any mailbox command, harmless elsewhere.
    send_imap_id(&mut session).await?;
    session
        .select(&config.folder)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("imap select '{}': {e}", config.folder)))?;

    let mut summary = PullSummary::default();
    drain_round(&mut session, pipeline, channel, &config, &mut summary).await?;

    // IDLE window: near-push latency for channels that opt in. `idle()`
    // consumes the session; `done()` returns it (async-imap ownership
    // contract). One extra drain after wake — cron cadence bounds the rest.
    // Servers without IDLE support (observed on NetEase 163) fail `init` —
    // degrade to pure polling instead of failing the whole run: the drain
    // above already delivered everything UNSEEN.
    if config.idle_secs > 0 {
        let mut handle = session.idle();
        match handle.init().await {
            Ok(()) => {
                let (fut, _stop) =
                    handle.wait_with_timeout(std::time::Duration::from_secs(config.idle_secs));
                let woke = fut.await;
                let mut session = handle
                    .done()
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("imap idle done: {e}")))?;
                if matches!(
                    woke,
                    Ok(async_imap::extensions::idle::IdleResponse::NewData(_))
                ) {
                    drain_round(&mut session, pipeline, channel, &config, &mut summary).await?;
                }
                let _ = session.logout().await;
            }
            Err(e) => {
                tracing::warn!(
                    channel = %channel.channel_key,
                    "imap IDLE unavailable ({e}) — degrading to cron-cadence polling; \
                     consider idle_secs: 0 to silence this"
                );
                let _ = handle.done().await;
            }
        }
    } else {
        let _ = session.logout().await;
    }

    Ok(summary)
}
