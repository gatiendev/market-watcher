//! HTTP API consumed by the Svelte dashboard.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/markets", get(markets))
        .route("/api/candles", get(candles))
        .route("/api/pivots", get(pivot_points))
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
