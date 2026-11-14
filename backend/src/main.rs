use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use market_watcher::api::{self, AppState};
use market_watcher::ingest::Engine;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn")),
        )
        .init();

    let database_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/market.db".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8791);
    let refresh_secs: u64 = std::env::var("REFRESH_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(300);
    let sync_on_start = std::env::var("SYNC_ON_START")
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(true);

    let (pool, engine) = market_watcher::bootstrap(&database_path).await?;
    tracing::info!(database = %database_path, "database ready");

    spawn_refresh_loop(engine.clone(), refresh_secs, sync_on_start);

    let state = AppState {
        pool,
        engine: engine.clone(),
    };
    let app = api::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, refresh_secs, "market-watcher listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Refresh in the background: once at boot, then every `refresh_secs`.
fn spawn_refresh_loop(engine: Arc<Engine>, refresh_secs: u64, sync_on_start: bool) {
    tokio::spawn(async move {
        if sync_on_start {
            let reports = engine.refresh_all().await;
            let ok = reports.iter().filter(|r| r.status == "ok").count();
            tracing::info!(ok, total = reports.len(), "initial sync finished");
        }
        if refresh_secs == 0 {
            tracing::info!("background refresh disabled");
            return;
        }
        let mut ticker = tokio::time::interval(Duration::from_secs(refresh_secs));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            let reports = engine.refresh_all().await;
            let failed: Vec<String> = reports
                .iter()
                .filter(|r| r.status != "ok")
                .map(|r| format!("{}/{}", r.symbol, r.timeframe))
                .collect();
            if failed.is_empty() {
                tracing::info!(series = reports.len(), "background refresh ok");
            } else {
                tracing::warn!(failed = ?failed, "background refresh had failures");
            }
        }
    });
}
