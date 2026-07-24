use anyhow::Result;
use malt_config::paths;
use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_gateway::auth::TokenStore;
use malt_gateway::rate_limit::RateLimiter;
use malt_gateway::server::build_router;
use malt_gateway::with_auth;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Requests allowed per client per rate-limit window. No per-endpoint or
/// global tuning yet (see `docs/BACKLOG.md`'s Gateway-hardening items) --
/// generous enough not to trip on normal interactive/CLI use.
const RATE_LIMIT_PER_WINDOW: usize = 100;

pub fn run_daemon(port: u16) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let data_dir = paths::data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let store = DebouncedStore::new(SessionStore::new(data_dir));
        let coordinator = Arc::new(Mutex::new(Coordinator::try_new(PoolConfig::default(), store)?));

        // Start VNP socket listener on port+1 (e.g. 7701 when HTTP is 7700)
        let vnp_coord = coordinator.clone();
        let vnp_port = port + 1;
        let client_counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
        std::thread::spawn(move || {
            let vnp_addr = format!("127.0.0.1:{vnp_port}");
            malt_daemon::vnp_listener::start_vnp_listener(&vnp_addr, vnp_coord, client_counter);
        });

        let token_store = Arc::new(TokenStore::new());
        let default_token = token_store.load_or_generate_default();
        println!("API token: {default_token}");
        let rate_limiter = Arc::new(RateLimiter::new(RATE_LIMIT_PER_WINDOW));

        let backend = Arc::new(DaemonBackend::new(coordinator.clone()));

        // Shutdown channel: POST /shutdown sends signal
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shutdown_tx = Arc::new(shutdown_tx);

        // Add shutdown route to the router, then apply auth enforcement --
        // with_auth() must be the last step so it also covers /shutdown,
        // not just the routes build_router() defines itself (axum's
        // .layer() only wraps routes registered before it's called).
        let shutdown_sender = shutdown_tx.clone();
        let router = build_router(backend).route(
            "/shutdown",
            axum::routing::post(move || async move {
                let _ = shutdown_sender.send(true);
                axum::Json(serde_json::json!({"ok": true, "data": "shutting down"}))
            }),
        );
        let router = with_auth(router, token_store, rate_limiter);

        let addr = format!("127.0.0.1:{port}");
        println!("malt daemon listening on {addr}");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal(shutdown_rx))
            .await?;

        // Explicitly persist all active sessions before process exit.
        // The Drop impl also calls shutdown_graceful as a safety net.
        {
            let mut coord = coordinator.lock().unwrap_or_else(|p| p.into_inner());
            coord.shutdown_graceful();
        }

        tracing::info!("daemon stopped");
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
