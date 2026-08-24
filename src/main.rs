use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tg_bridge::config;
use tg_bridge::build_router;
use tg_bridge::{AppState, SharedState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg_path =
        std::env::var("TGB_CONFIG").unwrap_or_else(|_| "config/tg-bridge.toml".to_owned());
    let cfg = config::load(&cfg_path)?;
    tracing::info!(
        listen = %cfg.server.listen,
        clients = cfg.clients.len(),
        bots = cfg.bots.len(),
        actions = cfg.actions.len(),
        "config loaded"
    );

    let listen = cfg.server.listen.clone();
    let http = reqwest::Client::builder()
        .timeout(cfg.server.request_timeout)
        .pool_idle_timeout(Duration::from_secs(90))
        .build()?;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(cfg.rate_limit.requests_per_minute),
        metrics: tg_bridge::metrics::Metrics::default(),
        http,
        cfg,
    });

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
