use anyhow::Context;
use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
use tracing::error;
use tracing::info;

/// Spawns an HTTP server that serves static files from the given directory.
/// Returns a JoinHandle for the server task.
pub async fn spawn_http_server(port: u16, dist_path: PathBuf) -> anyhow::Result<JoinHandle<()>> {
    // Verify dist_path exists
    if !dist_path.exists() {
        anyhow::bail!(
            "UI dist directory does not exist: {}. \n\
            Build the UI with: cd ../agentcanvas-ui && npm run build:jsonl",
            dist_path.display()
        );
    }

    if !dist_path.is_dir() {
        anyhow::bail!("UI dist path is not a directory: {}", dist_path.display());
    }

    let app = Router::new().fallback_service(ServeDir::new(&dist_path));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind HTTP server to {}", addr))?;

    info!(
        "HTTP server serving AgentCanvas UI on http://127.0.0.1:{}",
        port
    );
    info!("Serving files from: {}", dist_path.display());

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("HTTP server error: {}", e);
        }
    });

    Ok(handle)
}
