//! Sync orchestration: fetch -> store -> recompute EMAs -> record status.

use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::db;
use crate::indicators::{ema_series, EMA_FAST, EMA_SLOW};
use crate::models::{SymbolSpec, Timeframe, SYMBOLS};
use crate::providers::{CandleSource, SourceResolver};

/// What one `(symbol, timeframe)` sync did.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshReport {
    pub symbol: String,
    pub timeframe: String,
    /// Bars the provider handed back (updated bars included).
    pub fetched: usize,
    /// Bars physically written; 0 means "already up to date".
    pub written: usize,
    pub bars: i64,
    pub last_bar: Option<i64>,
    pub status: String,
    pub message: Option<String>,
}

impl RefreshReport {
    fn failed(spec: &SymbolSpec, timeframe: Timeframe, bars: i64, message: String) -> Self {
        Self {
            symbol: spec.symbol.to_string(),
            timeframe: timeframe.as_str().to_string(),
            fetched: 0,
            written: 0,
            bars,
            last_bar: None,
            status: "error".to_string(),
            message: Some(message),
        }
    }
}

/// Sync a single series using any candle source (Binance, Yahoo, or a test double).
pub async fn sync_series(
    pool: &SqlitePool,
    source: &dyn CandleSource,
    spec: &SymbolSpec,
    timeframe: Timeframe,
) -> RefreshReport {
    let existing_bars = db::count_candles(pool, spec.symbol, timeframe)
        .await
        .unwrap_or(0);
    let last_stored = db::last_bar_time(pool, spec.symbol, timeframe)
        .await
        .ok()
        .flatten();

    // Resume from the newest stored bucket: that bar is usually still forming and
    // gets rewritten with the fresher values instead of being skipped.
    let since = last_stored;

    let fetched = match source.fetch(spec, timeframe, since).await {
        Ok(candles) => candles,
        Err(err) => {
            let message = format!("{err:#}");
            let _ = db::record_sync(
                pool,
                spec.symbol,
                timeframe,
                existing_bars,
                "error",
                Some(&message),
            )
            .await;
            tracing::warn!(symbol = spec.symbol, tf = timeframe.as_str(), %message, "sync failed");
            let mut report = RefreshReport::failed(spec, timeframe, existing_bars, message);
            report.last_bar = last_stored;
            return report;
        }
    };

    let fetched_count = fetched.len();
    let written = match db::upsert_candles(pool, spec.symbol, timeframe, &fetched).await {
        Ok(count) => count,
        Err(err) => {
            let message = format!("database write failed: {err:#}");
            let _ = db::record_sync(
                pool,
                spec.symbol,
                timeframe,
                existing_bars,
                "error",
                Some(&message),
            )
            .await;
            let mut report = RefreshReport::failed(spec, timeframe, existing_bars, message);
            report.fetched = fetched_count;
            report.last_bar = last_stored;
            return report;
        }
    };

    // EMAs depend on the whole history, so recompute the full series each time.
    if let Err(err) = recompute_indicators(pool, spec, timeframe).await {
        let message = format!("indicator recompute failed: {err:#}");
        let _ = db::record_sync(
            pool,
            spec.symbol,
            timeframe,
            existing_bars,
            "error",
            Some(&message),
        )
        .await;
        let mut report = RefreshReport::failed(spec, timeframe, existing_bars, message);
        report.fetched = fetched_count;
        report.written = written;
        return report;
    }

    let bars = db::count_candles(pool, spec.symbol, timeframe)
        .await
        .unwrap_or(0);
    let last_bar = db::last_bar_time(pool, spec.symbol, timeframe)
        .await
        .ok()
        .flatten();
    let message = if written == 0 {
        Some("already up to date".to_string())
    } else {
        None
    };

    let _ = db::record_sync(
        pool,
        spec.symbol,
        timeframe,
        bars,
        "ok",
        message.as_deref(),
    )
    .await;

    tracing::info!(
        symbol = spec.symbol,
        tf = timeframe.as_str(),
        fetched = fetched_count,
        written,
        bars,
        last_bar = last_bar.map(crate::providers::iso).unwrap_or_default(),
        "sync ok"
    );

    RefreshReport {
        symbol: spec.symbol.to_string(),
        timeframe: timeframe.as_str().to_string(),
        fetched: fetched_count,
        written,
        bars,
        last_bar,
        status: "ok".to_string(),
        message,
    }
}

/// Recompute EMA50/EMA200 for a stored series and persist them.
pub async fn recompute_indicators(
    pool: &SqlitePool,
    spec: &SymbolSpec,
    timeframe: Timeframe,
) -> Result<()> {
    let candles = db::load_all_candles(pool, spec.symbol, timeframe).await?;
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let ema50 = ema_series(&closes, EMA_FAST);
    let ema200 = ema_series(&closes, EMA_SLOW);

    let values: Vec<(i64, Option<f64>, Option<f64>)> = candles
        .iter()
        .enumerate()
        .map(|(idx, candle)| (candle.open_time, ema50[idx], ema200[idx]))
        .collect();

    db::replace_indicators(pool, spec.symbol, timeframe, &values).await
}

/// Shared refresh coordinator used by the HTTP layer and the background loop.
pub struct Engine {
    pub pool: SqlitePool,
    pub sources: Arc<dyn SourceResolver>,
    refresh_lock: Mutex<()>,
}

impl Engine {
    pub fn new(pool: SqlitePool, sources: Arc<dyn SourceResolver>) -> Self {
        Self {
            pool,
            sources,
            refresh_lock: Mutex::new(()),
        }
    }

    /// Refresh every symbol on every timeframe. Concurrent calls wait for the
    /// running one instead of hammering the providers.
    pub async fn refresh_all(&self) -> Vec<RefreshReport> {
        let _guard = self.refresh_lock.lock().await;
        let mut reports = Vec::with_capacity(SYMBOLS.len() * Timeframe::ALL.len());
        for spec in SYMBOLS.iter() {
            for timeframe in Timeframe::ALL {
                let source = self.sources.source_for(spec);
                reports.push(sync_series(&self.pool, source, spec, timeframe).await);
            }
        }
        reports
    }

    /// Refresh one symbol on both timeframes (used by the UI's per-symbol retry).
    pub async fn refresh_symbol(&self, spec: &SymbolSpec) -> Vec<RefreshReport> {
        let _guard = self.refresh_lock.lock().await;
        let mut reports = Vec::with_capacity(Timeframe::ALL.len());
        for timeframe in Timeframe::ALL {
            let source = self.sources.source_for(spec);
            reports.push(sync_series(&self.pool, source, spec, timeframe).await);
        }
        reports
    }
}
