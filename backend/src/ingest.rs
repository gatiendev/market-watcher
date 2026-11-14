//! Sync orchestration: fetch -> store -> recompute EMAs -> record status.

use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::db;
use crate::indicators::{ema_series, EMA_FAST, EMA_SLOW};
use crate::models::{Candle, Source, SymbolSpec, Timeframe, SYMBOLS};
use crate::providers::{CandleSource, SourceResolver};

/// Daily runs first: the H4 backfill uses the daily history as its floor.
const SYNC_ORDER: [Timeframe; 2] = [Timeframe::D1, Timeframe::H4];

/// What one `(symbol, timeframe)` sync did.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshReport {
    pub symbol: String,
    pub timeframe: String,
    /// Bars the provider handed back (updated bars included).
    pub fetched: usize,
    /// Bars physically written; 0 means "already up to date".
    pub written: usize,
    /// Older bars recovered by the history backfill during this sync.
    pub backfilled: usize,
    pub bars: i64,
    pub last_bar: Option<i64>,
    /// Oldest bar stored after this sync.
    pub history_start: Option<i64>,
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
            backfilled: 0,
            bars,
            last_bar: None,
            history_start: None,
            status: "error".to_string(),
            message: Some(message),
        }
    }
}

/// Backwards pages one sync may pull. Enough to reach the daily history in a
/// single pass, and the loop exits as soon as the floor is reached.
const BACKFILL_PAGES: usize = 24;

/// Yahoo refuses hourly bars older than 730 days, so an H4 series resampled from
/// them cannot start earlier than that. Used as the backfill floor for those
/// symbols so we stop asking for history that does not exist.
const YAHOO_HOURLY_HISTORY_DAYS: i64 = 730;

/// How far back a timeframe should reach: the H4 series follows the daily one so
/// both charts cover the same period.
async fn history_floor(pool: &SqlitePool, spec: &SymbolSpec, timeframe: Timeframe) -> Option<i64> {
    if timeframe != Timeframe::H4 {
        return None;
    }
    let daily_start = db::first_bar_time(pool, spec.symbol, Timeframe::D1)
        .await
        .ok()
        .flatten()?;

    Some(match spec.source {
        Source::Yahoo => {
            let cap = chrono::Utc::now().timestamp_millis()
                - YAHOO_HOURLY_HISTORY_DAYS * crate::providers::DAY_MS;
            daily_start.max(cap)
        }
        Source::Binance => daily_start,
    })
}

/// Walk backwards from the oldest stored bar until the floor is reached or the
/// provider runs out of history. Returns (bars fetched, bars written).
async fn backfill_history(
    pool: &SqlitePool,
    source: &dyn CandleSource,
    spec: &SymbolSpec,
    timeframe: Timeframe,
    floor: Option<i64>,
) -> Result<(usize, usize)> {
    let Some(floor) = floor else {
        return Ok((0, 0));
    };

    let mut fetched = 0usize;
    let mut written = 0usize;

    for _ in 0..BACKFILL_PAGES {
        let Some(oldest) = db::first_bar_time(pool, spec.symbol, timeframe).await? else {
            break;
        };
        if oldest <= floor {
            break;
        }

        let page = source.fetch(spec, timeframe, None, Some(oldest)).await?;
        let mut older: Vec<Candle> = page
            .into_iter()
            .filter(|candle| candle.open_time < oldest)
            .collect();
        older.sort_by_key(|candle| candle.open_time);
        if older.is_empty() {
            // The provider has nothing before this point (or refuses that window).
            break;
        }
        fetched += older.len();
        let new_oldest = older
            .first()
            .map(|candle| candle.open_time)
            .unwrap_or(oldest);
        written += db::upsert_candles(pool, spec.symbol, timeframe, &older).await?;

        if new_oldest >= oldest {
            // No progress possible; stop instead of asking the same window again.
            break;
        }
    }

    Ok((fetched, written))
}

/// Sync a single series using any candle source (Binance, Yahoo, or a test double).
///
/// Two passes: the live edge (only the bars that are missing) and then a backwards
/// backfill so the H4 series reaches as far as the daily one.
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
    let floor = history_floor(pool, spec, timeframe).await;

    // Resume from the newest stored bucket: that bar is usually still forming and
    // gets rewritten with the fresher values instead of being skipped.
    let since = last_stored;

    let fetched = match source.fetch(spec, timeframe, since, None).await {
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

    let mut fetched_count = fetched.len();
    let mut written = match db::upsert_candles(pool, spec.symbol, timeframe, &fetched).await {
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

    let mut backfilled = 0usize;
    match backfill_history(pool, source, spec, timeframe, floor).await {
        Ok((backfill_fetched, backfill_written)) => {
            fetched_count += backfill_fetched;
            written += backfill_written;
            backfilled = backfill_written;
        }
        Err(err) => {
            let message = format!("history backfill failed: {err:#}");
            tracing::warn!(
                symbol = spec.symbol,
                tf = timeframe.as_str(),
                %message,
                "backfill failed"
            );
        }
    }

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
    let history_start = db::first_bar_time(pool, spec.symbol, timeframe)
        .await
        .ok()
        .flatten();
    let message = if written == 0 {
        Some("already up to date".to_string())
    } else {
        None
    };

    let _ = db::record_sync(pool, spec.symbol, timeframe, bars, "ok", message.as_deref()).await;

    tracing::info!(
        symbol = spec.symbol,
        tf = timeframe.as_str(),
        fetched = fetched_count,
        written,
        bars,
        last_bar = last_bar.map(crate::providers::iso).unwrap_or_default(),
        history_start = history_start.map(crate::providers::iso).unwrap_or_default(),
        "sync ok"
    );

    RefreshReport {
        symbol: spec.symbol.to_string(),
        timeframe: timeframe.as_str().to_string(),
        fetched: fetched_count,
        written,
        backfilled,
        bars,
        last_bar,
        history_start,
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
            for timeframe in SYNC_ORDER {
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
        for timeframe in SYNC_ORDER {
            let source = self.sources.source_for(spec);
            reports.push(sync_series(&self.pool, source, spec, timeframe).await);
        }
        reports
    }
}
