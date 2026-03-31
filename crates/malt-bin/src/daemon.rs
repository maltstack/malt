use anyhow::Result;
use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_gateway::server::build_router;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

pub fn run_daemon(port: u16) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default())));
        let backend = Arc::new(DaemonBackend::new(coordinator));

        // Shutdown channel: POST /shutdown sends signal
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shutdown_tx = Arc::new(shutdown_tx);

        // Add shutdown route to the router
        let shutdown_sender = shutdown_tx.clone();
        let router = build_router(backend)
            .route("/shutdown", axum::routing::post(move || async move {
                let _ = shutdown_sender.send(true);
                axum::Json(serde_json::json!({"ok": true, "data": "shutting down"}))
            }));

        let addr = format!("127.0.0.1:{port}");
        println!("malt daemon listening on {addr}");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal(shutdown_rx))
            .await?;

        println!("daemon stopped");
        Ok(())
    })
}

async fn shutdown_signal(mut rx: watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nshutting down (ctrl+c)...");
        }
        _ = async { while !*rx.borrow_and_update() { rx.changed().await.ok(); } } => {
            println!("\nshutting down (api request)...");
        }
    }
}
