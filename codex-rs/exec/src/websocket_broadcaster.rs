use crate::event_processor_with_jsonl_output::EventProcessorWithJsonOutput;
use codex_core::CodexThread;
use futures::stream::StreamExt;
use futures::SinkExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, error, info, warn};

pub type ConnectionId = u64;

const WEBSOCKET_CHANNEL_CAPACITY: usize = 128;

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

/// Spawns the WebSocket server on the specified port.
/// Returns a JoinHandle for the server task.
pub async fn spawn_websocket_server(
    port: u16,
    broadcaster: WebSocketEventBroadcaster,
) -> anyhow::Result<JoinHandle<()>> {
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let listener = TcpListener::bind(&addr).await?;
    info!("WebSocket event streaming server listening on ws://{}", addr);

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

    Ok(handle)
}

/// Handles a single WebSocket client connection.
async fn handle_websocket_client(stream: TcpStream, broadcaster: WebSocketEventBroadcaster) {
    // Upgrade the TCP connection to WebSocket
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
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

/// Spawns a parallel event listener that consumes events from CodexThread
/// and broadcasts them to all WebSocket clients.
pub fn spawn_websocket_event_listener(
    thread: Arc<CodexThread>,
    broadcaster: WebSocketEventBroadcaster,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Create a dedicated event processor for converting protocol events to JSONL
        let mut event_processor = EventProcessorWithJsonOutput::new(None);

        loop {
            match thread.next_event().await {
                Ok(event) => {
                    // Convert protocol event to ThreadEvent(s)
                    let thread_events = event_processor.collect_thread_events(&event);

                    // Serialize and broadcast each ThreadEvent
                    for thread_event in thread_events {
                        match serde_json::to_string(&thread_event) {
                            Ok(json) => {
                                broadcaster.broadcast(json).await;
                            }
                            Err(e) => {
                                error!("Failed to serialize event for WebSocket: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("WebSocket event listener stopped: {}", e);
                    break;
                }
            }
        }
    })
}

/// Spawns the complete WebSocket infrastructure: server + event listener.
/// Returns an error if the server fails to start, but does not block TUI execution.
pub async fn spawn_websocket_infrastructure(
    port: u16,
    thread: Arc<CodexThread>,
) -> anyhow::Result<()> {
    let broadcaster = WebSocketEventBroadcaster::new();

    // Spawn WebSocket server
    let _server_handle = spawn_websocket_server(port, broadcaster.clone()).await?;

    // Spawn parallel event listener
    let _listener_handle = spawn_websocket_event_listener(thread, broadcaster);

    info!("WebSocket event streaming infrastructure started on port {}", port);

    Ok(())
}
