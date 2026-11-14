//! HTTP contract tests: shapes the dashboard relies on, plus error handling.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::{day_start, test_db, trend_bars, FakeSource, FixedResolver};
use http_body_util::BodyExt;
use market_watcher::api::{self, AppState};
use market_watcher::ingest::{sync_series, Engine};
use market_watcher::models::{find_symbol, Timeframe};
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

fn build_app(pool: SqlitePool, source: FakeSource) -> Router {
    let resolver = Arc::new(FixedResolver::new(source));
    let engine = Arc::new(Engine::new(pool.clone(), resolver));
    api::router(AppState { pool, engine })
}

async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request failed");

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    (status, json)
}

#[tokio::test]
async fn health_reports_the_database() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(&app, "GET", "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["database"], "up");
}

#[tokio::test]
async fn markets_lists_all_four_symbols_on_both_timeframes() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));

    let (status, body) = call(&app, "GET", "/api/markets").await;
    assert_eq!(status, StatusCode::OK);

    let symbols = body["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 4);
    let names: Vec<&str> = symbols.iter().map(|row| row["symbol"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["SOLUSDT", "BTCUSDT", "EURUSD", "ES1"]);

    for row in symbols {
        for tf in ["H4", "D1"] {
            let snapshot = &row["timeframes"][tf];
            assert!(!snapshot.is_null(), "missing {tf} for {}", row["symbol"]);
            assert_eq!(snapshot["status"], "pending");
            assert!(snapshot["last_close"].is_null());
            assert_eq!(snapshot["bars"], 0);
        }
    }
    assert_eq!(symbols[2]["decimals"], 5, "FX needs 5 decimals");
    assert_eq!(symbols[1]["source"], "binance");
    assert_eq!(symbols[3]["source"], "yahoo");
}

#[tokio::test]
async fn markets_exposes_prices_emas_and_trend() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(260, day_start(), Timeframe::D1.bucket_ms(), 100.0, 1.0));
    let spec = find_symbol("SOLUSDT").unwrap();
    sync_series(&db_handle.pool, &source, spec, Timeframe::D1).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (_, body) = call(&app, "GET", "/api/markets").await;

    let row = &body["symbols"][0];
    assert_eq!(row["symbol"], "SOLUSDT");
    let d1 = &row["timeframes"]["D1"];
    assert_eq!(d1["status"], "ok");
    assert_eq!(d1["bars"], 260);
    assert!(d1["last_close"].as_f64().unwrap() > 100.0);
    assert!(d1["ema50"].as_f64().is_some());
    assert!(d1["ema200"].as_f64().is_some());
    assert_eq!(d1["trend"], "up", "a rising series puts EMA50 above EMA200");
    assert!(d1["last_bar_time"].as_i64().unwrap() > 0);
    assert!(d1["last_sync_at"].as_i64().unwrap() > 0);

    // H4 was never synced.
    assert_eq!(row["timeframes"]["H4"]["bars"], 0);
}

#[tokio::test]
async fn candles_returns_an_ascending_series_with_emas() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(260, day_start(), Timeframe::H4.bucket_ms(), 10.0, 0.1));
    let spec = find_symbol("BTCUSDT").unwrap();
    sync_series(&db_handle.pool, &source, spec, Timeframe::H4).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["symbol"], "BTCUSDT");
    assert_eq!(body["timeframe"], "H4");
    assert_eq!(body["decimals"], 2);

    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 50);
    let first = points[0]["time"].as_i64().unwrap();
    let last = points.last().unwrap()["time"].as_i64().unwrap();
    assert!(last > first, "oldest bar first");
    assert!(points.last().unwrap()["ema200"].as_f64().is_some());
    assert!(body["summary"]["ema50"].as_f64().is_some());
}

#[tokio::test]
async fn candles_accepts_lowercase_timeframe_and_caps_the_limit() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(20, day_start(), Timeframe::D1.bucket_ms(), 5.0, 0.1));
    let spec = find_symbol("ES1").unwrap();
    sync_series(&db_handle.pool, &source, spec, Timeframe::D1).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(&app, "GET", "/api/candles?symbol=es1&timeframe=1d&limit=9999").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["symbol"], "ES1");
    assert_eq!(body["points"].as_array().unwrap().len(), 20, "never more than stored");
}

#[tokio::test]
async fn candles_rejects_unknown_symbols_and_timeframes() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));

    let (status, body) = call(&app, "GET", "/api/candles?symbol=DOGE&timeframe=H4").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("unknown symbol"));

    let (status, body) = call(&app, "GET", "/api/candles?symbol=SOLUSDT&timeframe=M15").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("H4 or D1"));

    let (status, _) = call(&app, "GET", "/api/candles").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "missing symbol is a 404");
}

#[tokio::test]
async fn refresh_downloads_every_series_then_serves_it() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(220, day_start(), Timeframe::H4.bucket_ms(), 30.0, 0.05));
    let app = build_app(db_handle.pool.clone(), source);

    let (status, body) = call(&app, "POST", "/api/refresh").await;
    assert_eq!(status, StatusCode::OK);
    let reports = body["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 8);
    assert!(reports.iter().all(|report| report["status"] == "ok"));

    let (_, markets) = call(&app, "GET", "/api/markets").await;
    let sol = &markets["symbols"][0]["timeframes"]["H4"];
    assert_eq!(sol["bars"], 220);
    assert_eq!(sol["status"], "ok");
    assert!(sol["trend"].as_str().is_some());
}

#[tokio::test]
async fn refresh_can_target_one_symbol() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(210, day_start(), Timeframe::D1.bucket_ms(), 1.0, 0.001));
    let app = build_app(db_handle.pool.clone(), source);

    let (status, body) = call(&app, "POST", "/api/refresh?symbol=eurusd").await;
    assert_eq!(status, StatusCode::OK);
    let reports = body["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|report| report["symbol"] == "EURUSD"));

    let (_, markets) = call(&app, "GET", "/api/markets").await;
    assert_eq!(markets["symbols"][2]["timeframes"]["D1"]["bars"], 210);
    assert_eq!(markets["symbols"][0]["timeframes"]["D1"]["bars"], 0, "others untouched");
}

#[tokio::test]
async fn refresh_reports_provider_failures_without_crashing() {
    let db_handle = test_db().await;
    let source = FakeSource::new(Vec::new());
    source.fail_with("upstream unavailable");
    let app = build_app(db_handle.pool.clone(), source);

    let (status, body) = call(&app, "POST", "/api/refresh?symbol=BTCUSDT").await;
    assert_eq!(status, StatusCode::OK, "the endpoint still answers");
    let reports = body["reports"].as_array().unwrap();
    assert!(reports.iter().all(|report| report["status"] == "error"));
    assert!(reports[0]["message"].as_str().unwrap().contains("upstream unavailable"));
}

#[tokio::test]
async fn refresh_rejects_an_unknown_symbol() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, _) = call(&app, "POST", "/api/refresh?symbol=GBPJPY").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
