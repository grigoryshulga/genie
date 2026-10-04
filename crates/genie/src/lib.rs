//! Genie server and CLI.
//!
//! `genie serve` runs the web UI and API, the agent runtime, the automation
//! engine and the delivery channels in one process. See docs/platform/backend.md.

pub mod agent_config;
pub mod budget;
pub mod channels;
pub mod cli;
pub mod config;
pub mod context;
pub mod doctor;
pub mod engine;
pub mod git;
pub mod http;
pub mod knowledge;
pub mod llm_key;
pub mod mcp_gateway;
pub mod notify;
pub mod ops;
pub mod orchestrate;
pub mod outcome;
pub mod questions;
pub mod runtime;
pub mod sandbox;
pub mod sessions;
pub mod spend;
pub mod state;
pub mod stats;
pub mod tasks;
pub mod vault_sync;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use state::App;

/// Default data directory: `$GENIE_DATA` or `~/.local/share/genie`.
pub fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GENIE_DATA").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/genie")
}

/// How long open requests get to finish after a stop signal.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Run the server until Ctrl-C or SIGTERM.
pub async fn serve(app: Arc<App>) -> Result<(), String> {
    let addr: SocketAddr = format!("{}:{}", app.cfg.bind, app.cfg.port).parse().map_err(|e| format!("bind address: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("{addr}: {e}"))?;
    println!("genie serve: http://{addr} (data {})", app.data.display());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = serve_on(app, listener, async {
        let _ = stopped.await;
    });
    tokio::pin!(server);
    tokio::select! {
        done = &mut server => done,
        () = stop_signal() => {
            let _ = stop.send(());
            // A graceful shutdown waits for every open connection, and a browser tab holds its live
            // stream (SSE) open for hours: give requests a moment, then stop anyway.
            tokio::time::timeout(SHUTDOWN_GRACE, &mut server).await.unwrap_or(Ok(()))
        }
    }
}

/// Completes on Ctrl-C (SIGINT) or, where the platform has it, SIGTERM (`docker stop`, systemd).
async fn stop_signal() {
    #[cfg(unix)]
    if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Serve on an already bound listener until `shutdown` completes.
pub async fn serve_on(
    app: Arc<App>,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let router = http::router(app);
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| e.to_string())
}
