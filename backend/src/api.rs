//! HTTP API consumed by the Svelte dashboard.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::crossings::{find_crossings, SeriesBar};
use crate::db;
use crate::indicators::pct_change;
use crate::ingest::{Engine, RefreshReport};
use crate::models::{find_symbol, Candle, SymbolSpec, Timeframe, SYMBOLS};
use crate::pivots::{find_pivots, DEFAULT_LOOKBACK, MAX_LOOKBACK, MIN_LOOKBACK};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub engine: Arc<Engine>,
}

/// One timeframe's view of a symbol: last price, EMAs and freshness.
#[derive(Debug, Clone, Serialize)]
pub struct TimeframeSnapshot {
    pub timeframe: String,
    pub last_close: Option<f64>,
    pub prev_close: Option<f64>,
    pub change_pct: Option<f64>,
    pub ema50: Option<f64>,
    pub ema200: Option<f64>,
    /// "up" when EMA50 > EMA200, "down" otherwise, `None` before the 200 EMA warms up.
    pub trend: Option<String>,
    /// Bar where the current trend started, and how many bars it has lasted.
    pub trend_since: Option<i64>,
    pub trend_bars: Option<i64>,
    pub bars: i64,
    pub last_bar_time: Option<i64>,
    pub last_sync_at: Option<i64>,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketRow {
    pub symbol: String,
    pub label: String,
    pub kind: String,
    pub source: String,
    pub decimals: u8,
    pub timeframes: BTreeMap<String, TimeframeSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketsResponse {
    pub generated_at: i64,
    pub symbols: Vec<MarketRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandlePoint {
    /// Epoch seconds (what the charting library expects).
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub ema50: Option<f64>,
    pub ema200: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandlesResponse {
    pub symbol: String,
    pub label: String,
    pub timeframe: String,
    pub source: String,
    pub decimals: u8,
    pub points: Vec<CandlePoint>,
    pub summary: TimeframeSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefreshResponse {
    pub reports: Vec<RefreshReport>,
}

/// How long a chart request may wait for its tail top-up before it is answered
/// with whatever is stored.
const TOP_UP_TIMEOUT: Duration = Duration::from_secs(5);

/// One daily swing point, in the shape the chart wants it.
#[derive(Debug, Clone, Serialize)]
pub struct PivotPoint {
    /// Epoch seconds of the pivot bar (what the charting library expects).
    pub time: i64,
    pub price: f64,
    /// "high" or "low".
    pub kind: String,
    /// Index of the bar inside the daily series, oldest first.
    pub bar: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PivotsResponse {
    pub symbol: String,
    pub label: String,
    pub decimals: u8,
    /// Pivots are always measured on the daily series, whatever the chart shows.
    pub timeframe: String,
    /// Bars required each side of the pivot bar.
    pub left: usize,
    pub right: usize,
    pub bars: i64,
    pub pivots: Vec<PivotPoint>,
}

/// One past EMA 50 / EMA 200 crossing and what price did inside the run it opened.
#[derive(Debug, Clone, Serialize)]
pub struct CrossingRow {
    pub symbol: String,
    pub label: String,
    pub decimals: u8,
    pub timeframe: String,
    /// "up" or "down" for the side the fast EMA crossed to.
    pub direction: String,
    /// Epoch seconds of the crossing bar (what the charting library expects).
    pub crossed_at: i64,
    /// Close of the crossing bar: the price every measurement starts from.
    pub price: f64,
    pub ema50: f64,
    pub ema200: f64,
    /// Best favourable excursion from `price`, in percent.
    pub peak_pct: f64,
    pub peak_bars: usize,
    pub days_to_peak: f64,
    /// Worst adverse excursion before the peak, in percent (<= 0).
    pub trough_pct: f64,
    /// Swing from that adverse extreme to the peak, in percent.
    pub trough_to_peak_pct: f64,
    pub run_bars: usize,
    pub run_days: f64,
    /// Close-to-close result of the run, direction-relative, in percent.
    pub exit_pct: f64,
    /// True while this is the current trend.
    pub open: bool,
}

/// Aggregate of crossings for one timeframe, for one asset or all of them.
#[derive(Debug, Clone, Serialize)]
pub struct CrossingSummary {
    pub timeframe: String,
    /// `None` when the numbers cover every asset.
    pub symbol: Option<String>,
    pub crossings: usize,
    pub ups: usize,
    pub downs: usize,
    pub open_runs: usize,
    pub avg_peak_pct: Option<f64>,
    pub median_days_to_peak: Option<f64>,
    pub avg_trough_to_peak_pct: Option<f64>,
    /// Average result of the runs that already ended with an opposite crossing.
    pub avg_exit_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossingsResponse {
    pub generated_at: i64,
    pub rows: Vec<CrossingRow>,
    pub summaries: Vec<CrossingSummary>,
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        ApiError::internal(format!("database error: {err}"))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        tracing::error!(error = %err, "internal error");
        ApiError::internal(format!("{err:#}"))
    }
}

#[derive(Debug, Deserialize)]
pub struct CandlesQuery {
    pub symbol: Option<String>,
    pub timeframe: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RefreshQuery {
    pub symbol: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PivotsQuery {
    pub symbol: Option<String>,
    /// Shorthand for `left` and `right` together.
    pub lookback: Option<usize>,
    pub left: Option<usize>,
    pub right: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CrossingsQuery {
    pub symbol: Option<String>,
    pub timeframe: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/markets", get(markets))
        .route("/api/candles", get(candles))
        .route("/api/pivots", get(pivot_points))
        .route("/api/crossings", get(crossing_overview))
        .route("/api/refresh", post(refresh))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .is_ok();
    let code = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(serde_json::json!({
            "status": if ok { "ok" } else { "degraded" },
            "database": if ok { "up" } else { "down" },
            "time": chrono::Utc::now().timestamp_millis(),
        })),
    )
}

async fn markets(State(state): State<AppState>) -> Result<Json<MarketsResponse>, ApiError> {
    let mut rows = Vec::with_capacity(SYMBOLS.len());
    for spec in SYMBOLS.iter() {
        let mut timeframes = BTreeMap::new();
        for timeframe in Timeframe::ALL {
            let snapshot = snapshot_for(&state.pool, spec, timeframe).await?;
            timeframes.insert(timeframe.as_str().to_string(), snapshot);
        }
        rows.push(MarketRow {
            symbol: spec.symbol.to_string(),
            label: spec.label.to_string(),
            kind: spec.kind.to_string(),
            source: spec.source.slug().to_string(),
            decimals: spec.decimals,
            timeframes,
        });
    }

    Ok(Json(MarketsResponse {
        generated_at: chrono::Utc::now().timestamp_millis(),
        symbols: rows,
    }))
}

async fn candles(
    State(state): State<AppState>,
    Query(query): Query<CandlesQuery>,
) -> Result<Json<CandlesResponse>, ApiError> {
    let spec = query
        .symbol
        .as_deref()
        .and_then(find_symbol)
        .ok_or_else(|| ApiError::not_found("unknown symbol"))?;

    let timeframe = query
        .timeframe
        .as_deref()
        .and_then(Timeframe::parse)
        .ok_or_else(|| ApiError::bad_request("timeframe must be H4 or D1"))?;

    // Opening a chart expects current bars: top up the tail first, fetching only
    // the bars that are missing. Failures are ignored - stale data beats no data.
    let _ = tokio::time::timeout(TOP_UP_TIMEOUT, state.engine.top_up(spec, timeframe)).await;

    let limit = query.limit.unwrap_or(320).clamp(10, 2000);
    let series = db::load_series(&state.pool, spec.symbol, timeframe, limit).await?;

    let points: Vec<CandlePoint> = series
        .iter()
        .map(|row| CandlePoint {
            time: row.candle.open_time.div_euclid(1000),
            open: row.candle.open,
            high: row.candle.high,
            low: row.candle.low,
            close: row.candle.close,
            volume: row.candle.volume,
            ema50: row.ema50,
            ema200: row.ema200,
        })
        .collect();

    let summary = snapshot_for(&state.pool, spec, timeframe).await?;

    Ok(Json(CandlesResponse {
        symbol: spec.symbol.to_string(),
        label: spec.label.to_string(),
        timeframe: timeframe.as_str().to_string(),
        source: spec.source.slug().to_string(),
        decimals: spec.decimals,
        points,
        summary,
    }))
}

/// Daily pivot highs and lows: the swing points a trader draws support and
/// resistance from. `lookback` (or `left`/`right`) sets how many daily bars the
/// pivot must beat on each side; the default 15 finds major swings, 5 finds many
/// minors.
async fn pivot_points(
    State(state): State<AppState>,
    Query(query): Query<PivotsQuery>,
) -> Result<Json<PivotsResponse>, ApiError> {
    let spec = query
        .symbol
        .as_deref()
        .and_then(find_symbol)
        .ok_or_else(|| ApiError::not_found("unknown symbol"))?;

    let lookback = query
        .lookback
        .unwrap_or(DEFAULT_LOOKBACK)
        .clamp(MIN_LOOKBACK, MAX_LOOKBACK);
    let left = query
        .left
        .unwrap_or(lookback)
        .clamp(MIN_LOOKBACK, MAX_LOOKBACK);
    let right = query
        .right
        .unwrap_or(lookback)
        .clamp(MIN_LOOKBACK, MAX_LOOKBACK);

    // Pivots are read from the daily series, so its tail has to be current too,
    // otherwise yesterday's swing is missing from today's chart.
    let _ = tokio::time::timeout(TOP_UP_TIMEOUT, state.engine.top_up(spec, Timeframe::D1)).await;

    // Pivots read the whole daily series, not the chart window: a level found
    // three years back is still a level.
    let series = db::load_series(&state.pool, spec.symbol, Timeframe::D1, 5000).await?;
    let bars: Vec<Candle> = series.into_iter().map(|row| row.candle).collect();
    let found = find_pivots(&bars, left, right);

    Ok(Json(PivotsResponse {
        symbol: spec.symbol.to_string(),
        label: spec.label.to_string(),
        decimals: spec.decimals,
        timeframe: Timeframe::D1.as_str().to_string(),
        left,
        right,
        bars: bars.len() as i64,
        pivots: found
            .into_iter()
            .map(|pivot| PivotPoint {
                time: pivot.open_time.div_euclid(1000),
                price: pivot.price,
                kind: pivot.kind.as_str().to_string(),
                bar: pivot.index,
            })
            .collect(),
    }))
}

/// Every past EMA 50 / EMA 200 crossing, with the performance of the run each
/// one opened. Defaults to all four assets on both timeframes; `symbol` and
/// `timeframe` narrow it.
async fn crossing_overview(
    State(state): State<AppState>,
    Query(query): Query<CrossingsQuery>,
) -> Result<Json<CrossingsResponse>, ApiError> {
    let specs: Vec<&SymbolSpec> = match query.symbol.as_deref() {
        Some(raw) => vec![find_symbol(raw).ok_or_else(|| ApiError::not_found("unknown symbol"))?],
        None => SYMBOLS.iter().collect(),
    };
    let timeframes: Vec<Timeframe> = match query.timeframe.as_deref() {
        Some(raw) => vec![Timeframe::parse(raw)
            .ok_or_else(|| ApiError::bad_request("timeframe must be H4 or D1"))?],
        None => Timeframe::ALL.to_vec(),
    };

    let mut rows = Vec::new();
    for spec in &specs {
        for timeframe in &timeframes {
            // The whole stored history: a crossing three years back still counts.
            let series = db::load_series_all(&state.pool, spec.symbol, *timeframe).await?;
            let bars: Vec<SeriesBar> = series
                .iter()
                .map(|row| SeriesBar {
                    open_time: row.candle.open_time,
                    high: row.candle.high,
                    low: row.candle.low,
                    close: row.candle.close,
                    ema50: row.ema50,
                    ema200: row.ema200,
                })
                .collect();

            // Newest first, so the page reads as a timeline.
            for crossing in find_crossings(&bars, *timeframe).iter().rev() {
                rows.push(CrossingRow {
                    symbol: spec.symbol.to_string(),
                    label: spec.label.to_string(),
                    decimals: spec.decimals,
                    timeframe: timeframe.as_str().to_string(),
                    direction: crossing.direction.as_str().to_string(),
                    crossed_at: crossing.open_time.div_euclid(1000),
                    price: crossing.price,
                    ema50: crossing.ema50,
                    ema200: crossing.ema200,
                    peak_pct: crossing.peak_pct,
                    peak_bars: crossing.peak_bars,
                    days_to_peak: crossing.days_to_peak,
                    trough_pct: crossing.trough_pct,
                    trough_to_peak_pct: crossing.trough_to_peak_pct,
                    run_bars: crossing.run_bars,
                    run_days: crossing.run_days,
                    exit_pct: crossing.exit_pct,
                    open: crossing.open,
                });
            }
        }
    }

    let summaries = summarise(&rows, &specs, &timeframes);

    Ok(Json(CrossingsResponse {
        generated_at: chrono::Utc::now().timestamp_millis(),
        rows,
        summaries,
    }))
}

/// One summary per timeframe across every asset in scope, then one per asset.
fn summarise(
    rows: &[CrossingRow],
    specs: &[&SymbolSpec],
    timeframes: &[Timeframe],
) -> Vec<CrossingSummary> {
    let mut out = Vec::new();
    for timeframe in timeframes {
        let in_scope: Vec<&CrossingRow> = rows
            .iter()
            .filter(|row| row.timeframe == timeframe.as_str())
            .collect();
        if in_scope.is_empty() {
            continue;
        }
        out.push(summarise_one(timeframe, None, &in_scope));
        for spec in specs {
            let own: Vec<&CrossingRow> = in_scope
                .iter()
                .copied()
                .filter(|row| row.symbol == spec.symbol)
                .collect();
            if !own.is_empty() {
                out.push(summarise_one(timeframe, Some(spec.symbol), &own));
            }
        }
    }
    out
}

fn summarise_one(
    timeframe: &Timeframe,
    symbol: Option<&str>,
    rows: &[&CrossingRow],
) -> CrossingSummary {
    let peak: Vec<f64> = rows.iter().map(|row| row.peak_pct).collect();
    let days: Vec<f64> = rows.iter().map(|row| row.days_to_peak).collect();
    let swing: Vec<f64> = rows.iter().map(|row| row.trough_to_peak_pct).collect();
    let closed: Vec<f64> = rows
        .iter()
        .filter(|row| !row.open)
        .map(|row| row.exit_pct)
        .collect();

    CrossingSummary {
        timeframe: timeframe.as_str().to_string(),
        symbol: symbol.map(str::to_string),
        crossings: rows.len(),
        ups: rows.iter().filter(|row| row.direction == "up").count(),
        downs: rows.iter().filter(|row| row.direction == "down").count(),
        open_runs: rows.iter().filter(|row| row.open).count(),
        avg_peak_pct: mean(&peak),
        median_days_to_peak: median(days),
        avg_trough_to_peak_pct: mean(&swing),
        avg_exit_pct: mean(&closed),
    }
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[middle - 1] + values[middle]) / 2.0)
    } else {
        Some(values[middle])
    }
}

async fn refresh(
    State(state): State<AppState>,
    Query(query): Query<RefreshQuery>,
) -> Result<Json<RefreshResponse>, ApiError> {
    let reports = match query.symbol.as_deref() {
        Some(raw) => {
            let spec = find_symbol(raw).ok_or_else(|| ApiError::not_found("unknown symbol"))?;
            state.engine.refresh_symbol(spec).await
        }
        None => state.engine.refresh_all().await,
    };

    let failed = reports.iter().filter(|r| r.status != "ok").count();
    if failed > 0 {
        tracing::warn!(
            failed,
            total = reports.len(),
            "refresh finished with errors"
        );
    }
    Ok(Json(RefreshResponse { reports }))
}

/// Read the newest bar + EMAs + sync status for one series.
async fn snapshot_for(
    pool: &SqlitePool,
    spec: &SymbolSpec,
    timeframe: Timeframe,
) -> Result<TimeframeSnapshot, ApiError> {
    let latest = db::latest_series_row(pool, spec.symbol, timeframe).await?;
    let prev_close = db::previous_close(pool, spec.symbol, timeframe).await?;
    let bars = db::count_candles(pool, spec.symbol, timeframe).await?;
    let sync = db::get_sync(pool, spec.symbol, timeframe).await?;
    let trend_run = db::trend_run(pool, spec.symbol, timeframe).await?;

    let last_close = latest.as_ref().map(|row| row.candle.close);
    let ema50 = latest.as_ref().and_then(|row| row.ema50);
    let ema200 = latest.as_ref().and_then(|row| row.ema200);
    let trend = match (ema50, ema200) {
        (Some(fast), Some(slow)) if fast > slow => Some("up".to_string()),
        (Some(_), Some(_)) => Some("down".to_string()),
        _ => None,
    };

    Ok(TimeframeSnapshot {
        timeframe: timeframe.as_str().to_string(),
        last_close,
        prev_close,
        change_pct: prev_close
            .zip(last_close)
            .and_then(|(prev, last)| pct_change(prev, last)),
        ema50,
        ema200,
        trend,
        trend_since: trend_run.map(|run| run.0),
        trend_bars: trend_run.map(|run| run.1),
        bars,
        last_bar_time: latest.as_ref().map(|row| row.candle.open_time),
        last_sync_at: sync.as_ref().map(|state| state.last_sync_at),
        status: sync
            .as_ref()
            .map(|state| state.status.clone())
            .unwrap_or_else(|| "pending".to_string()),
        message: sync.and_then(|state| state.message),
    })
}
