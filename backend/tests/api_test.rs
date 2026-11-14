//! HTTP contract tests: shapes the dashboard relies on, plus error handling.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::{day_start, test_db, trend_bars, FakeSource, FixedResolver};
use http_body_util::BodyExt;
use market_watcher::api::{self, AppState};
use market_watcher::ingest::{now_ms, open_bucket, sync_series, Engine};
use market_watcher::db;
use market_watcher::models::{find_symbol, Candle, Timeframe};
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

fn build_app(pool: SqlitePool, source: FakeSource) -> Router {
    build_app_with(pool, Arc::new(source))
}

fn build_app_with(pool: SqlitePool, source: Arc<FakeSource>) -> Router {
    let resolver = Arc::new(FixedResolver::shared(source));
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
    let names: Vec<&str> = symbols
        .iter()
        .map(|row| row["symbol"].as_str().unwrap())
        .collect();
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
    let source = FakeSource::new(trend_bars(
        260,
        day_start(),
        Timeframe::D1.bucket_ms(),
        100.0,
        1.0,
    ));
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
    // The 200 EMA exists from bar 200 on, so the trend is 61 bars old.
    assert_eq!(d1["trend_bars"], 61);
    assert_eq!(
        d1["trend_since"],
        day_start() + 199 * Timeframe::D1.bucket_ms(),
        "the run starts at the first bar with both EMAs"
    );
    assert!(d1["last_bar_time"].as_i64().unwrap() > 0);
    assert!(d1["last_sync_at"].as_i64().unwrap() > 0);

    // H4 was never synced.
    assert_eq!(row["timeframes"]["H4"]["bars"], 0);
}

#[tokio::test]
async fn candles_returns_an_ascending_series_with_emas() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(
        260,
        day_start(),
        Timeframe::H4.bucket_ms(),
        10.0,
        0.1,
    ));
    let spec = find_symbol("BTCUSDT").unwrap();
    sync_series(&db_handle.pool, &source, spec, Timeframe::H4).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(
        &app,
        "GET",
        "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50",
    )
    .await;
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
    let source = FakeSource::new(trend_bars(
        20,
        day_start(),
        Timeframe::D1.bucket_ms(),
        5.0,
        0.1,
    ));
    let spec = find_symbol("ES1").unwrap();
    sync_series(&db_handle.pool, &source, spec, Timeframe::D1).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(
        &app,
        "GET",
        "/api/candles?symbol=es1&timeframe=1d&limit=9999",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["symbol"], "ES1");
    assert_eq!(
        body["points"].as_array().unwrap().len(),
        20,
        "never more than stored"
    );
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
    let source = FakeSource::new(trend_bars(
        220,
        day_start(),
        Timeframe::H4.bucket_ms(),
        30.0,
        0.05,
    ));
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
    let source = FakeSource::new(trend_bars(
        210,
        day_start(),
        Timeframe::D1.bucket_ms(),
        1.0,
        0.001,
    ));
    let app = build_app(db_handle.pool.clone(), source);

    let (status, body) = call(&app, "POST", "/api/refresh?symbol=eurusd").await;
    assert_eq!(status, StatusCode::OK);
    let reports = body["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|report| report["symbol"] == "EURUSD"));

    let (_, markets) = call(&app, "GET", "/api/markets").await;
    assert_eq!(markets["symbols"][2]["timeframes"]["D1"]["bars"], 210);
    assert_eq!(
        markets["symbols"][0]["timeframes"]["D1"]["bars"], 0,
        "others untouched"
    );
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
    assert!(reports[0]["message"]
        .as_str()
        .unwrap()
        .contains("upstream unavailable"));
}

#[tokio::test]
async fn refresh_rejects_an_unknown_symbol() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, _) = call(&app, "POST", "/api/refresh?symbol=GBPJPY").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Nine daily bars: peak at index 2 (high 16), trough at 4 (low 8), peak at 6 (17).
async fn seed_zigzag(pool: &SqlitePool) {
    let closes = [10.0, 12.0, 15.0, 12.0, 9.0, 12.0, 16.0, 12.0, 7.0];
    let bars: Vec<Candle> = closes
        .iter()
        .enumerate()
        .map(|(idx, close)| {
            Candle::new(
                day_start() + idx as i64 * Timeframe::D1.bucket_ms(),
                *close,
                close + 1.0,
                close - 1.0,
                *close,
                10.0,
            )
        })
        .collect();
    db::upsert_candles(pool, "BTCUSDT", Timeframe::D1, &bars)
        .await
        .expect("seed daily bars");
}

#[tokio::test]
async fn pivots_returns_daily_swing_highs_and_lows() {
    let db_handle = test_db().await;
    seed_zigzag(&db_handle.pool).await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));

    let (status, body) = call(&app, "GET", "/api/pivots?symbol=BTCUSDT&lookback=2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["symbol"], "BTCUSDT");
    assert_eq!(body["timeframe"], "D1", "pivots are always measured on daily bars");
    assert_eq!(body["left"], 2);
    assert_eq!(body["right"], 2);
    assert_eq!(body["bars"], 9);

    let pivots = body["pivots"].as_array().unwrap();
    let shape: Vec<(String, f64)> = pivots
        .iter()
        .map(|p| {
            (
                p["kind"].as_str().unwrap().to_string(),
                p["price"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("high".to_string(), 16.0),
            ("low".to_string(), 8.0),
            ("high".to_string(), 17.0),
        ]
    );
    assert_eq!(pivots[0]["time"].as_i64().unwrap(), day_start() / 1000 + 2 * 86_400);
    assert_eq!(pivots[1]["bar"].as_u64().unwrap(), 4);
}

#[tokio::test]
async fn pivots_default_to_fifteen_bars_each_side_and_clamp_requests() {
    let db_handle = test_db().await;
    seed_zigzag(&db_handle.pool).await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));

    let (status, body) = call(&app, "GET", "/api/pivots?symbol=BTCUSDT").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["left"], 15);
    assert_eq!(body["right"], 15);
    assert!(body["pivots"].as_array().unwrap().is_empty(), "9 bars cannot hold a 15/15 pivot");

    let (_, body) = call(&app, "GET", "/api/pivots?symbol=BTCUSDT&lookback=0").await;
    assert_eq!(body["left"], 1, "a zero lookback falls back to the minimum");
    let (_, body) = call(&app, "GET", "/api/pivots?symbol=BTCUSDT&lookback=999").await;
    assert_eq!(body["left"], 60, "the lookback is capped");
    let (_, body) = call(&app, "GET", "/api/pivots?symbol=BTCUSDT&left=1&right=1").await;
    assert_eq!(body["left"], 1);
    assert_eq!(body["right"], 1);
    assert_eq!(body["pivots"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn pivots_rejects_unknown_symbols() {
    let db_handle = test_db().await;
    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));
    let (status, body) = call(&app, "GET", "/api/pivots?symbol=DOGE").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("unknown symbol"));
}

/// Switching instruments or timeframes must never return another series: each
/// symbol keeps its own candles and its own daily pivots.
#[tokio::test]
async fn switching_symbols_and_timeframes_never_mixes_series() {
    let db_handle = test_db().await;
    let sol = find_symbol("SOLUSDT").unwrap();
    let btc = find_symbol("BTCUSDT").unwrap();

    let sol_h4 = FakeSource::new(trend_bars(40, day_start(), Timeframe::H4.bucket_ms(), 10.0, 0.1));
    sync_series(&db_handle.pool, &sol_h4, sol, Timeframe::H4).await;
    let btc_h4 = FakeSource::new(trend_bars(40, day_start(), Timeframe::H4.bucket_ms(), 100.0, 0.5));
    sync_series(&db_handle.pool, &btc_h4, btc, Timeframe::H4).await;
    let sol_d1 = FakeSource::new(trend_bars(40, day_start(), Timeframe::D1.bucket_ms(), 1000.0, 1.0));
    sync_series(&db_handle.pool, &sol_d1, sol, Timeframe::D1).await;

    let app = build_app(db_handle.pool.clone(), FakeSource::new(Vec::new()));

    for (symbol, timeframe, base, bucket) in [
        ("SOLUSDT", "H4", 10.0, Timeframe::H4.bucket_ms() / 1000),
        ("BTCUSDT", "H4", 100.0, Timeframe::H4.bucket_ms() / 1000),
        ("SOLUSDT", "D1", 1000.0, Timeframe::D1.bucket_ms() / 1000),
        ("BTCUSDT", "H4", 100.0, Timeframe::H4.bucket_ms() / 1000),
        ("SOLUSDT", "H4", 10.0, Timeframe::H4.bucket_ms() / 1000),
        ("SOLUSDT", "D1", 1000.0, Timeframe::D1.bucket_ms() / 1000),
    ] {
        let path = format!("/api/candles?symbol={symbol}&timeframe={timeframe}&limit=20");
        let (status, body) = call(&app, "GET", &path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body["symbol"], symbol, "{path}");
        assert_eq!(body["timeframe"], timeframe, "{path}");

        let points = body["points"].as_array().unwrap();
        assert!(!points.is_empty(), "{path}");
        let first = points[0]["close"].as_f64().unwrap();
        assert!(
            (first - base).abs() < base * 0.3,
            "{path}: first close {first} is not {symbol} {timeframe} data"
        );
        let step = points[1]["time"].as_i64().unwrap() - points[0]["time"].as_i64().unwrap();
        assert_eq!(step, bucket, "{path}: bars are not {timeframe} buckets");
    }

    // Pivots follow the instrument too, never the previously viewed one.
    for (symbol, low, high) in [("SOLUSDT", 9.0, 12.0), ("BTCUSDT", 95.0, 125.0)] {
        let (status, body) =
            call(&app, "GET", &format!("/api/pivots?symbol={symbol}&lookback=5")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["symbol"], symbol);
        for pivot in body["pivots"].as_array().unwrap() {
            let price = pivot["price"].as_f64().unwrap();
            assert!(
                price >= low && price <= high,
                "{symbol} pivot {price} belongs to another instrument"
            );
        }
    }
}

/// A chart loads from the database; when its newest bar is behind, the request
/// tops the tail up - fetching only what follows the newest stored bar.
#[tokio::test]
async fn candles_fetch_only_the_bars_missing_after_the_newest_stored_one() {
    let db_handle = test_db().await;
    let bucket = Timeframe::H4.bucket_ms();
    let current = open_bucket(now_ms(), Timeframe::H4);
    let start = current - 21 * bucket;

    // Stored history stops two buckets short of now.
    let stored = trend_bars(20, start, bucket, 10.0, 0.1);
    db::upsert_candles(&db_handle.pool, "BTCUSDT", Timeframe::H4, &stored)
        .await
        .unwrap();
    assert_eq!(
        db::count_candles(&db_handle.pool, "BTCUSDT", Timeframe::H4)
            .await
            .unwrap(),
        20
    );

    // The provider holds the same bars plus the two that have since appeared.
    let source = Arc::new(FakeSource::new(trend_bars(22, start, bucket, 10.0, 0.1)));
    let app = build_app_with(db_handle.pool.clone(), Arc::clone(&source));

    let (status, body) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK);
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 22, "the two fresh bars were added");
    // The API speaks seconds, the database stores milliseconds.
    assert_eq!(points.last().unwrap()["time"].as_i64(), Some(current / 1000));

    // One call, starting at the newest stored bar: nothing older is re-fetched.
    assert_eq!(source.calls(), 1);
    assert_eq!(source.last_since(), Some(current - 2 * bucket));
    assert_eq!(
        db::count_candles(&db_handle.pool, "BTCUSDT", Timeframe::H4)
            .await
            .unwrap(),
        22
    );

    // Now the tail is current: another load must not touch the provider.
    let (status, body) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["points"].as_array().unwrap().len(), 22);
    assert_eq!(source.calls(), 1, "up-to-date series is not re-fetched");
}

#[tokio::test]
async fn candles_are_served_from_storage_when_the_top_up_fails() {
    let db_handle = test_db().await;
    let bucket = Timeframe::H4.bucket_ms();
    let current = open_bucket(now_ms(), Timeframe::H4);
    let stored = trend_bars(20, current - 21 * bucket, bucket, 10.0, 0.1);
    db::upsert_candles(&db_handle.pool, "BTCUSDT", Timeframe::H4, &stored)
        .await
        .unwrap();

    let source = Arc::new(FakeSource::new(Vec::new()));
    source.fail_with("provider down");
    let app = build_app_with(db_handle.pool.clone(), Arc::clone(&source));

    let (status, body) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK, "stale data beats no data");
    assert_eq!(body["points"].as_array().unwrap().len(), 20);
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn candles_fill_an_empty_series_once() {
    let db_handle = test_db().await;
    let bucket = Timeframe::H4.bucket_ms();
    let current = open_bucket(now_ms(), Timeframe::H4);
    let source = Arc::new(FakeSource::new(trend_bars(30, current - 29 * bucket, bucket, 10.0, 0.1)));
    let app = build_app_with(db_handle.pool.clone(), Arc::clone(&source));

    let (status, body) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["points"].as_array().unwrap().len(), 30);
    assert_eq!(source.last_since(), None, "nothing stored, so nothing to skip");

    // The database now holds them, so a second load is served from storage.
    let (status, _) = call(&app, "GET", "/api/candles?symbol=BTCUSDT&timeframe=H4&limit=50").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(source.calls(), 1);
}
