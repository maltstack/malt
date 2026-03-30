use anyhow::Result;
use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_gateway::server::build_router;
use std::sync::{Arc, Mutex};

pub fn run_daemon(port: u16) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default())));
        let backend = Arc::new(DaemonBackend::new(coordinator));
        let router = build_router(backend);

        let addr = format!("127.0.0.1:{port}");
        println!("malt daemon listening on {addr}");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        println!("daemon stopped");
        Ok(())
    })
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
    println!("\nshutting down...");
}
