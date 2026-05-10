//! WebSocket 实时推送处理器
//!
//! 将 `EventBus` 事件通过 WebSocket 双向通道推送给客户端。
//!
//! # 端点
//!
//! `GET /api/v1/ws` — 升级为 WebSocket 连接
//!
//! # 协议
//!
//! 服务端推送事件格式：
//!
//! ```json
//! {"event": "PostCreated", "data": {"id": "...", "slug": "...", ...}}
//! ```
//!
//! 客户端发送控制消息：
//!
//! ```json
//! {"type": "subscribe", "filter": ["PostCreated", "CommentCreated"]}
//! {"type": "ping"}
//! ```
//!
//! 服务端响应：
//!
//! ```json
//! {"type": "pong"}
//! {"type": "connected", "message": "raisfast websocket"}
//! ```

use axum::extract::{
    Query, State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;

use crate::handlers::sse::event_type_name;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::get;

    reg_route!(axum::Router::new(), registry, "/ws", get(ws_handler), "system", "ws", ["GET"])
}

/// WS 连接查询参数
#[derive(Debug, Deserialize, Default)]
pub struct WsQuery {
    /// 逗号分隔的事件类型过滤，如 `PostCreated,CommentCreated`
    pub filter: Option<String>,
}

/// 客户端 → 服务端消息
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// 订阅指定事件类型（空列表 = 全部）
    Subscribe { filter: Option<Vec<String>> },
    /// 心跳
    Ping,
}

/// 服务端 → 客户端推送
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    /// 事件推送
    Event {
        event: String,
        data: serde_json::Value,
    },
    /// 心跳响应
    Pong,
    /// 连接成功
    Connected { message: String },
    /// 错误
    Error { message: String },
}

/// WebSocket 升级端点
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<crate::AppState>,
    Query(query): Query<WsQuery>,
) -> axum::response::Response {
    let initial_filter: Vec<String> = query
        .filter
        .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    ws.on_upgrade(move |socket| handle_socket(socket, state, initial_filter))
}

async fn handle_socket(socket: WebSocket, state: crate::AppState, initial_filter: Vec<String>) {
    let (mut sender, mut receiver) = socket.split();

    let connected = serde_json::to_string(&ServerMessage::Connected {
        message: "raisfast websocket".into(),
    })
    .unwrap_or_default();
    let _ = sender.send(Message::Text(connected.into())).await;

    let rx = state.eventbus.subscribe();
    let mut event_stream = BroadcastStream::new(rx);
    let mut filter_types = initial_filter;

    loop {
        tokio::select! {
            event_result = event_stream.next() => {
                match event_result {
                    Some(Ok(arc_event)) => {
                        let type_name = event_type_name(arc_event.as_ref());

                        if !filter_types.is_empty()
                            && !filter_types.iter().any(|f| f == type_name.as_ref())
                        {
                            continue;
                        }

                        let data = serde_json::to_value(arc_event.as_ref())
                            .unwrap_or(serde_json::Value::Null);

                        let msg = ServerMessage::Event {
                            event: type_name.to_string(),
                            data,
                        };

                        let payload = serde_json::to_string(&msg).unwrap_or_default();
                        if sender.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n))) => {
                        tracing::warn!("WS client lagged, skipped {n} events");
                    }
                    None => break,
                }
            }
            client_msg = receiver.next() => {
                match client_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Subscribe { filter }) => {
                                filter_types = filter.unwrap_or_default();
                            }
                            Ok(ClientMessage::Ping) => {
                                let pong = serde_json::to_string(&ServerMessage::Pong)
                                    .unwrap_or_default();
                                let _ = sender.send(Message::Text(pong.into())).await;
                            }
                            Err(_) => {
                                let err = serde_json::to_string(&ServerMessage::Error {
                                    message: "invalid message format".into(),
                                })
                                .unwrap_or_default();
                                let _ = sender.send(Message::Text(err.into())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_message_serialization() {
        let msg = ServerMessage::Connected {
            message: "test".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("connected"));

        let msg = ServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("pong"));
    }

    #[test]
    fn client_message_deserialization() {
        let msg: ClientMessage =
            serde_json::from_str(r#"{"type":"subscribe","filter":["PostCreated"]}"#).unwrap();
        match msg {
            ClientMessage::Subscribe { filter } => {
                assert_eq!(filter.unwrap(), vec!["PostCreated"]);
            }
            _ => panic!("expected subscribe"),
        }

        let msg: ClientMessage = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert!(matches!(msg, ClientMessage::Ping));
    }
}
