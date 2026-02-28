use codex_app_server_protocol::ThreadItem as AppServerThreadItem;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TurnAbortReason;
use futures::SinkExt;
use futures::stream::StreamExt;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

pub type ConnectionId = u64;

const WEBSOCKET_CHANNEL_CAPACITY: usize = 128;

struct WebSocketRuntime {
    broadcaster: WebSocketEventBroadcaster,
    stream_state: Mutex<WebSocketStreamState>,
}

static WEBSOCKET_RUNTIME: OnceLock<WebSocketRuntime> = OnceLock::new();

#[derive(Default)]
struct WebSocketStreamState {
    thread_id: Option<String>,
}

/// Manages WebSocket clients and broadcasts events to them.
pub struct WebSocketEventBroadcaster {
    clients: Arc<RwLock<HashMap<ConnectionId, ClientState>>>,
    next_id: Arc<AtomicU64>,
}

struct ClientState {
    tx: mpsc::Sender<String>,
}

impl WebSocketEventBroadcaster {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Broadcast a JSONL event to all connected clients.
    /// Slow clients that can't keep up will be disconnected.
    pub async fn broadcast(&self, event_json: String) {
        let clients = self.clients.read().await;

        // Fast path: no clients connected
        if clients.is_empty() {
            return;
        }

        let mut disconnected = Vec::new();

        for (&id, client) in clients.iter() {
            match client.tx.try_send(event_json.clone()) {
                Ok(_) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!("Client {} is too slow, marking for disconnect", id);
                    disconnected.push(id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!("Client {} channel closed", id);
                    disconnected.push(id);
                }
            }
        }

        drop(clients);

        // Remove disconnected clients
        if !disconnected.is_empty() {
            let mut clients = self.clients.write().await;
            for id in disconnected {
                clients.remove(&id);
                debug!("Removed client {}", id);
            }
        }
    }

    async fn add_client(&self, tx: mpsc::Sender<String>) -> ConnectionId {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut clients = self.clients.write().await;
        clients.insert(id, ClientState { tx });
        info!("WebSocket client {} connected", id);
        id
    }

    async fn remove_client(&self, id: ConnectionId) {
        let mut clients = self.clients.write().await;
        clients.remove(&id);
        info!("WebSocket client {} disconnected", id);
    }
}

impl Clone for WebSocketEventBroadcaster {
    fn clone(&self) -> Self {
        Self {
            clients: Arc::clone(&self.clients),
            next_id: Arc::clone(&self.next_id),
        }
    }
}

/// Spawns only the WebSocket server and registers a global broadcaster runtime.
/// Callers can stream protocol events later via `broadcast_protocol_event`.
pub async fn spawn_websocket_server_only(port: u16) -> anyhow::Result<(JoinHandle<()>, u16)> {
    if WEBSOCKET_RUNTIME.get().is_some() {
        anyhow::bail!("WebSocket runtime already initialized");
    }

    let broadcaster = WebSocketEventBroadcaster::new();
    let (server_handle, bound_port) = spawn_websocket_server(port, broadcaster.clone()).await?;

    let runtime = WebSocketRuntime {
        broadcaster,
        stream_state: Mutex::new(WebSocketStreamState::default()),
    };
    if WEBSOCKET_RUNTIME.set(runtime).is_err() {
        anyhow::bail!("WebSocket runtime already initialized");
    }

    Ok((server_handle, bound_port))
}

/// Converts a protocol event to app-server-style WS notifications and broadcasts them.
/// No-op when the websocket runtime has not been started.
pub async fn broadcast_protocol_event(event: &Event) {
    let Some(runtime) = WEBSOCKET_RUNTIME.get() else {
        return;
    };

    let notifications = {
        let mut stream_state = runtime
            .stream_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        protocol_event_to_notifications(event, &mut stream_state)
    };

    for notification in notifications {
        match serde_json::to_string(&notification) {
            Ok(json) => {
                runtime.broadcaster.broadcast(json).await;
            }
            Err(e) => {
                error!("Failed to serialize event for WebSocket: {}", e);
            }
        }
    }
}

fn protocol_event_to_notifications(
    event: &Event,
    stream_state: &mut WebSocketStreamState,
) -> Vec<serde_json::Value> {
    match &event.msg {
        EventMsg::SessionConfigured(ev) => {
            let thread_id = ev.session_id.to_string();
            stream_state.thread_id = Some(thread_id.clone());
            vec![json!({
                "method": "thread/started",
                "params": {
                    "thread": {
                        "id": thread_id,
                    },
                },
            })]
        }
        EventMsg::TurnStarted(ev) => {
            let Some(thread_id) = stream_state.thread_id.as_ref() else {
                return Vec::new();
            };

            vec![json!({
                "method": "turn/started",
                "params": {
                    "threadId": thread_id,
                    "turn": {
                        "id": ev.turn_id,
                        "status": "in_progress",
                    },
                },
            })]
        }
        EventMsg::TurnComplete(ev) => vec![json!({
            "method": "turn/completed",
            "params": {
                "turn": {
                    "id": ev.turn_id,
                    "status": "completed",
                },
            },
        })],
        EventMsg::TurnAborted(ev) => {
            let Some(turn_id) = ev.turn_id.as_ref() else {
                return Vec::new();
            };

            let status = match ev.reason {
                TurnAbortReason::Interrupted
                | TurnAbortReason::Replaced
                | TurnAbortReason::ReviewEnded => "cancelled",
            };

            vec![json!({
                "method": "turn/completed",
                "params": {
                    "turn": {
                        "id": turn_id,
                        "status": status,
                    },
                },
            })]
        }
        EventMsg::ItemStarted(ev) => {
            let thread_id = ev.thread_id.to_string();
            stream_state.thread_id = Some(thread_id.clone());
            let item = AppServerThreadItem::from(ev.item.clone());
            vec![json!({
                "method": "item/started",
                "params": {
                    "threadId": thread_id,
                    "turnId": ev.turn_id,
                    "item": item,
                },
            })]
        }
        EventMsg::ItemCompleted(ev) => {
            let thread_id = ev.thread_id.to_string();
            stream_state.thread_id = Some(thread_id.clone());
            let item = AppServerThreadItem::from(ev.item.clone());
            vec![json!({
                "method": "item/completed",
                "params": {
                    "threadId": thread_id,
                    "turnId": ev.turn_id,
                    "item": item,
                },
            })]
        }
        _ => Vec::new(),
    }
}

/// Spawns the WebSocket server on the specified port.
/// Returns a JoinHandle for the server task.
pub async fn spawn_websocket_server(
    port: u16,
    broadcaster: WebSocketEventBroadcaster,
) -> anyhow::Result<(JoinHandle<()>, u16)> {
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let listener = TcpListener::bind(&addr).await?;
    let bound_port = listener.local_addr()?.port();
    info!(
        "WebSocket event streaming server listening on ws://127.0.0.1:{}",
        bound_port
    );

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("New WebSocket connection from {}", peer_addr);
                    let broadcaster_clone = broadcaster.clone();
                    tokio::spawn(handle_websocket_client(stream, broadcaster_clone));
                }
                Err(e) => {
                    error!("Failed to accept WebSocket connection: {}", e);
                }
            }
        }
    });

    Ok((handle, bound_port))
}

/// Handles a single WebSocket client connection.
async fn handle_websocket_client(stream: TcpStream, broadcaster: WebSocketEventBroadcaster) {
    // Upgrade the TCP connection to WebSocket
    let ws_stream =
        match tokio_tungstenite::accept_async_with_config(stream, Some(WebSocketConfig::default()))
            .await
        {
            Ok(ws) => ws,
            Err(e) => {
                error!("WebSocket upgrade failed: {}", e);
                return;
            }
        };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let (tx, mut rx) = mpsc::channel::<String>(WEBSOCKET_CHANNEL_CAPACITY);

    let client_id = broadcaster.add_client(tx).await;

    // Spawn task to forward messages from channel to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(json) = rx.recv().await {
            if let Err(e) = ws_sender.send(Message::Text(json.into())).await {
                error!("Failed to send WebSocket message: {}", e);
                break;
            }
        }
        // Close the WebSocket gracefully
        let _ = ws_sender.close().await;
    });

    // Read messages from client (we don't expect any, but need to detect disconnection)
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => {
                    debug!("Client {} sent close frame", client_id);
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    // Pong is automatically sent by tokio-tungstenite
                    debug!("Client {} sent ping", client_id);
                    let _ = payload; // Suppress unused warning
                }
                Ok(_) => {
                    // Ignore other message types (we're send-only)
                }
                Err(e) => {
                    debug!("WebSocket error for client {}: {}", client_id, e);
                    break;
                }
            }
        }
    });

    // Wait for either task to complete (disconnect)
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    broadcaster.remove_client(client_id).await;
}
