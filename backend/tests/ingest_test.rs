//! End-to-end sync behaviour with a controlled data source: what gets stored,
//! what gets re-fetched, and what happens when a provider is down.

mod common;

use common::{day_start, test_db, trend_bars, FakeSource, FixedResolver, HOUR_MS};
use market_watcher::db;
use market_watcher::indicators::{ema_series, EMA_FAST, EMA_SLOW};
use market_watcher::ingest::{recompute_indicators, sync_series, Engine};
use market_watcher::models::{find_symbol, Timeframe};
use std::sync::Arc;

const D1: Timeframe = Timeframe::D1;

#[tokio::test]
async fn first_sync_stores_everything_and_computes_emas() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let bars = trend_bars(260, day_start(), D1.bucket_ms(), 100.0, 0.5);
    let source = FakeSource::new(bars.clone());

    let report = sync_series(&db_handle.pool, &source, spec, D1).await;

    assert_eq!(report.status, "ok", "report: {report:?}");
    assert_eq!(report.fetched, 260);
    assert_eq!(report.bars, 260);
    assert_eq!(
        source.last_since(),
        None,
        "first sync must not send a cursor"
    );

    let latest = db::latest_series_row(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .expect("latest");
    assert!(latest.ema50.is_some() && latest.ema200.is_some());

    // The stored EMAs must equal a straight computation over the stored closes.
    let closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
    let expected_50 = ema_series(&closes, EMA_FAST)
        .last()
        .copied()
        .flatten()
        .unwrap();
    let expected_200 = ema_series(&closes, EMA_SLOW)
        .last()
        .copied()
        .flatten()
        .unwrap();
    assert!((latest.ema50.unwrap() - expected_50).abs() < 1e-9);
    assert!((latest.ema200.unwrap() - expected_200).abs() < 1e-9);
}

#[tokio::test]
async fn short_history_leaves_the_slow_ema_empty() {
    let db_handle = test_db().await;
    let spec = find_symbol("EURUSD").unwrap();
    let source = FakeSource::new(trend_bars(120, day_start(), D1.bucket_ms(), 1.1, 0.0001));

    sync_series(&db_handle.pool, &source, spec, D1).await;

    let latest = db::latest_series_row(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .unwrap();
    assert!(
        latest.ema50.is_some(),
        "50 EMA needs 50 bars, 120 are available"
    );
    assert!(
        latest.ema200.is_none(),
        "200 EMA must stay empty with only 120 bars"
    );
}

#[tokio::test]
async fn second_sync_resumes_from_the_last_bucket() {
    let db_handle = test_db().await;
    let spec = find_symbol("BTCUSDT").unwrap();
    let start = day_start();
    let first = trend_bars(250, start, D1.bucket_ms(), 100.0, 0.2);
    let source = FakeSource::new(first.clone());

    sync_series(&db_handle.pool, &source, spec, D1).await;
    let second = sync_series(&db_handle.pool, &source, spec, D1).await;

    assert_eq!(second.status, "ok");
    let last_time = first.last().unwrap().open_time;
    assert_eq!(
        source.last_since(),
        Some(last_time),
        "sync must resume from the newest stored bar so the live bar is refreshed"
    );
    assert_eq!(second.bars, 250, "no duplication");
}

#[tokio::test]
async fn new_bars_are_appended_and_only_those_are_fetched() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let start = day_start();
    let mut bars = trend_bars(240, start, D1.bucket_ms(), 100.0, 0.1);
    let source = FakeSource::new(bars.clone());
    sync_series(&db_handle.pool, &source, spec, D1).await;

    // Two fresh sessions arrive.
    let last = bars.last().unwrap().open_time;
    bars.push(market_watcher::models::Candle::new(
        last + D1.bucket_ms(),
        130.0,
        132.0,
        129.0,
        131.0,
        5.0,
    ));
    bars.push(market_watcher::models::Candle::new(
        last + 2 * D1.bucket_ms(),
        131.0,
        133.0,
        130.0,
        132.5,
        6.0,
    ));
    source.set_bars(bars.clone());

    let report = sync_series(&db_handle.pool, &source, spec, D1).await;
    assert_eq!(report.bars, 242);
    assert_eq!(
        report.fetched, 3,
        "the live bar is refreshed and the two new bars arrive"
    );

    let stored = db::load_candles(&db_handle.pool, spec.symbol, D1, 5)
        .await
        .unwrap();
    assert_eq!(stored.len(), 5);
    assert_eq!(stored.last().unwrap().close, 132.5);
}

#[tokio::test]
async fn a_provider_failure_keeps_the_existing_data() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let source = FakeSource::new(trend_bars(220, day_start(), D1.bucket_ms(), 100.0, 0.1));
    sync_series(&db_handle.pool, &source, spec, D1).await;

    source.fail_with("provider exploded");
    let report = sync_series(&db_handle.pool, &source, spec, D1).await;

    assert_eq!(report.status, "error");
    assert!(report.message.unwrap().contains("provider exploded"));
    assert_eq!(
        db::count_candles(&db_handle.pool, spec.symbol, D1)
            .await
            .unwrap(),
        220
    );
    let state = db::get_sync(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.status, "error");

    // Recovery: the next healthy sync reports ok again.
    source.heal();
    let report = sync_series(&db_handle.pool, &source, spec, D1).await;
    assert_eq!(report.status, "ok");
}

#[tokio::test]
async fn an_empty_provider_response_is_not_an_error() {
    let db_handle = test_db().await;
    let spec = find_symbol("ES1").unwrap();
    let source = FakeSource::new(Vec::new());
    let report = sync_series(&db_handle.pool, &source, spec, D1).await;
    assert_eq!(report.status, "ok");
    assert_eq!(report.bars, 0);
}

#[tokio::test]
async fn recomputing_indicators_is_stable() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let source = FakeSource::new(trend_bars(230, day_start(), D1.bucket_ms(), 50.0, 1.5));
    sync_series(&db_handle.pool, &source, spec, D1).await;

    let before = db::latest_series_row(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .unwrap();
    recompute_indicators(&db_handle.pool, spec, D1)
        .await
        .unwrap();
    let after = db::latest_series_row(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.ema50, after.ema50);
    assert_eq!(before.ema200, after.ema200);
}

#[tokio::test]
async fn engine_refreshes_all_symbols_and_timeframes() {
    let db_handle = test_db().await;
    let h4_bars = trend_bars(220, day_start(), Timeframe::H4.bucket_ms(), 10.0, 0.01);
    let source = FakeSource::new(h4_bars);
    let resolver = Arc::new(FixedResolver::new(source));
    let engine = Engine::new(db_handle.pool.clone(), resolver);

    let reports = engine.refresh_all().await;
    assert_eq!(reports.len(), 8, "4 symbols x 2 timeframes");
    assert!(reports.iter().all(|r| r.status == "ok"), "{reports:?}");
    assert_eq!(
        db::count_candles(&db_handle.pool, "BTCUSDT", Timeframe::H4)
            .await
            .unwrap(),
        220
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, "ES1", Timeframe::D1)
            .await
            .unwrap(),
        220
    );
}

#[tokio::test]
async fn engine_can_refresh_a_single_symbol() {
    let db_handle = test_db().await;
    let source = FakeSource::new(trend_bars(210, day_start(), HOUR_MS, 1.0, 0.01));
    let resolver = Arc::new(FixedResolver::new(source));
    let engine = Engine::new(db_handle.pool.clone(), resolver);

    let spec = find_symbol("eurusd").unwrap();
    let reports = engine.refresh_symbol(spec).await;
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|report| report.symbol == "EURUSD"));

    assert_eq!(
        db::count_candles(&db_handle.pool, "EURUSD", Timeframe::H4)
            .await
            .unwrap(),
        210
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, "SOLUSDT", Timeframe::H4)
            .await
            .unwrap(),
        0
    );
}

// --- history backfill -------------------------------------------------------

/// 300 daily bars plus six 4h bars per day: the H4 series must reach as far back
/// as the daily one, walking several provider pages to get there.
fn backfill_fixture() -> (
    Vec<market_watcher::models::Candle>,
    Vec<market_watcher::models::Candle>,
) {
    let start = day_start();
    let days = 300;
    let daily = trend_bars(days, start, D1.bucket_ms(), 100.0, 0.1);
    let four_hour = trend_bars(days * 6, start, Timeframe::H4.bucket_ms(), 100.0, 0.02);
    (daily, four_hour)
}

#[tokio::test]
async fn h4_backfills_back_to_the_daily_history() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let (daily, four_hour) = backfill_fixture();
    let source = FakeSource::new(Vec::new())
        .with_timeframe_bars(D1, daily.clone())
        .with_timeframe_bars(Timeframe::H4, four_hour.clone())
        .with_page_size(300);
    let engine = Engine::new(db_handle.pool.clone(), Arc::new(FixedResolver::new(source)));

    let reports = engine.refresh_symbol(spec).await;
    let h4_report = reports
        .iter()
        .find(|report| report.timeframe == "H4")
        .expect("H4 report");

    assert_eq!(h4_report.status, "ok", "{h4_report:?}");
    assert!(
        h4_report.backfilled > 0,
        "the H4 pass must recover older bars: {h4_report:?}"
    );

    let d1_oldest = db::first_bar_time(&db_handle.pool, spec.symbol, D1)
        .await
        .unwrap()
        .unwrap();
    let h4_oldest = db::first_bar_time(&db_handle.pool, spec.symbol, Timeframe::H4)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(d1_oldest, daily[0].open_time);
    assert_eq!(
        h4_oldest, daily[0].open_time,
        "H4 must start where the daily series starts"
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, spec.symbol, Timeframe::H4)
            .await
            .unwrap(),
        four_hour.len() as i64,
        "every 4h bar is stored exactly once"
    );
    assert_eq!(h4_report.history_start, Some(h4_oldest));
}

#[tokio::test]
async fn a_second_sync_only_fetches_the_live_bar() {
    let db_handle = test_db().await;
    let spec = find_symbol("BTCUSDT").unwrap();
    let (daily, four_hour) = backfill_fixture();
    let source = FakeSource::new(Vec::new())
        .with_timeframe_bars(D1, daily)
        .with_timeframe_bars(Timeframe::H4, four_hour)
        .with_page_size(300);
    let engine = Engine::new(db_handle.pool.clone(), Arc::new(FixedResolver::new(source)));

    engine.refresh_symbol(spec).await;
    let reports = engine.refresh_symbol(spec).await;
    let h4_report = reports
        .iter()
        .find(|report| report.timeframe == "H4")
        .expect("H4 report");

    assert_eq!(h4_report.backfilled, 0, "history is complete: no backfill");
    assert_eq!(
        h4_report.fetched, 1,
        "only the forming bar is re-fetched: {h4_report:?}"
    );
    assert_eq!(h4_report.bars, 1800, "nothing is duplicated or dropped");
}

#[tokio::test]
async fn backfill_stops_cleanly_when_the_provider_runs_out() {
    let db_handle = test_db().await;
    let spec = find_symbol("SOLUSDT").unwrap();
    let (daily, _) = backfill_fixture();
    // The H4 feed only has the newest 300 bars; the daily series reaches further back.
    let newest_h4 = daily.last().unwrap().open_time + 5 * Timeframe::H4.bucket_ms();
    let start = newest_h4 - 299 * Timeframe::H4.bucket_ms();
    let short_h4 = trend_bars(300, start, Timeframe::H4.bucket_ms(), 100.0, 0.02);
    let source = FakeSource::new(Vec::new())
        .with_timeframe_bars(D1, daily)
        .with_timeframe_bars(Timeframe::H4, short_h4.clone())
        .with_page_size(120);
    let engine = Engine::new(db_handle.pool.clone(), Arc::new(FixedResolver::new(source)));

    let reports = engine.refresh_symbol(spec).await;
    let h4_report = reports
        .iter()
        .find(|report| report.timeframe == "H4")
        .expect("H4 report");

    assert_eq!(
        h4_report.status, "ok",
        "running out of history is not an error"
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, spec.symbol, Timeframe::H4)
            .await
            .unwrap(),
        300
    );
    assert_eq!(
        db::first_bar_time(&db_handle.pool, spec.symbol, Timeframe::H4)
            .await
            .unwrap(),
        Some(short_h4[0].open_time)
    );
}

#[tokio::test]
async fn hourly_feeds_stop_at_the_provider_history_limit() {
    let db_handle = test_db().await;
    let spec = find_symbol("EURUSD").unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let day_ms = D1.bucket_ms();
    let cap = now - 730 * day_ms;
    let bucket = Timeframe::H4.bucket_ms();
    let align = |ts: i64| ts - ts.rem_euclid(bucket);

    // Daily history reaches back four years.
    let daily = trend_bars(1200, align(now - 1200 * day_ms), day_ms, 100.0, 0.1);
    // The hourly feed is generous: 900 days of 4h bars, more than Yahoo ever gives.
    let h4 = trend_bars(5400, align(cap - 200 * day_ms), bucket, 100.0, 0.02);
    let source = FakeSource::new(Vec::new())
        .with_timeframe_bars(D1, daily)
        .with_timeframe_bars(Timeframe::H4, h4)
        .with_page_size_for(Timeframe::H4, 400);
    let engine = Engine::new(db_handle.pool.clone(), Arc::new(FixedResolver::new(source)));

    let reports = engine.refresh_symbol(spec).await;
    let h4_report = reports
        .iter()
        .find(|report| report.timeframe == "H4")
        .expect("H4 report");

    assert_eq!(h4_report.status, "ok");
    let start = h4_report.history_start.expect("history start");
    assert!(
        start <= cap,
        "the series must reach the 730 day limit: {h4_report:?}"
    );
    assert!(
        start > cap - 70 * day_ms,
        "and stop there instead of taking the extra history: {h4_report:?}"
    );
    assert!(
        h4_report.bars < 5400,
        "bars older than the limit are left alone: {h4_report:?}"
    );
}
