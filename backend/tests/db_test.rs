//! Storage guarantees: no duplicate bars, stable ordering, indicator join.

mod common;

use common::{day_start, test_db, trend_bars, MINUTE_MS};
use market_watcher::db;
use market_watcher::models::{Candle, Timeframe};

#[tokio::test]
async fn schema_is_created_and_reopenable() {
    let db_handle = test_db().await;
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(&db_handle.pool)
            .await
            .unwrap();
    for expected in ["candles", "indicators", "sync_state"] {
        assert!(
            tables.contains(&expected.to_string()),
            "missing table {expected}"
        );
    }

    // Re-running the migration must not fail.
    db::migrate(&db_handle.pool).await.expect("migrate twice");
}

#[tokio::test]
async fn candles_round_trip_in_chart_order() {
    let db_handle = test_db().await;
    let bars = trend_bars(5, day_start(), 86_400_000, 100.0, 1.0);

    let written = db::upsert_candles(&db_handle.pool, "SOLUSDT", Timeframe::D1, &bars)
        .await
        .unwrap();
    assert_eq!(written, 5);

    let loaded = db::load_candles(&db_handle.pool, "SOLUSDT", Timeframe::D1, 100)
        .await
        .unwrap();
    assert_eq!(
        loaded, bars,
        "stored bars must come back identical and ascending"
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, "SOLUSDT", Timeframe::D1)
            .await
            .unwrap(),
        5
    );
}

#[tokio::test]
async fn reupserting_a_bar_updates_it_instead_of_duplicating() {
    let db_handle = test_db().await;
    let mut bars = trend_bars(3, day_start(), 86_400_000, 100.0, 0.0);
    db::upsert_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, &bars)
        .await
        .unwrap();

    // The live bar moves as the session progresses.
    let last = bars.len() - 1;
    bars[last].close = 123.0;
    bars[last].high = 130.0;
    db::upsert_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, &bars)
        .await
        .unwrap();

    let count = db::count_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1)
        .await
        .unwrap();
    assert_eq!(count, 3, "no duplicate rows");

    let loaded = db::load_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, 10)
        .await
        .unwrap();
    assert_eq!(loaded.last().unwrap().close, 123.0);
    assert_eq!(loaded.last().unwrap().high, 130.0);
}

#[tokio::test]
async fn last_bar_time_is_the_incremental_cursor() {
    let db_handle = test_db().await;
    assert_eq!(
        db::last_bar_time(&db_handle.pool, "EURUSD", Timeframe::H4)
            .await
            .unwrap(),
        None
    );

    let start = day_start();
    let bars = trend_bars(4, start, MINUTE_MS, 1.0, 0.0);
    db::upsert_candles(&db_handle.pool, "EURUSD", Timeframe::H4, &bars)
        .await
        .unwrap();
    assert_eq!(
        db::last_bar_time(&db_handle.pool, "EURUSD", Timeframe::H4)
            .await
            .unwrap(),
        Some(start + 3 * MINUTE_MS)
    );
}

#[tokio::test]
async fn timeframes_and_symbols_are_isolated() {
    let db_handle = test_db().await;
    let start = day_start();
    db::upsert_candles(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::D1,
        &trend_bars(2, start, 86_400_000, 10.0, 0.0),
    )
    .await
    .unwrap();
    db::upsert_candles(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::H4,
        &trend_bars(6, start, Timeframe::H4.bucket_ms(), 20.0, 0.0),
    )
    .await
    .unwrap();

    assert_eq!(
        db::count_candles(&db_handle.pool, "SOLUSDT", Timeframe::D1)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, "SOLUSDT", Timeframe::H4)
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        db::count_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn invalid_bars_are_never_stored() {
    let db_handle = test_db().await;
    let bars = vec![
        Candle::new(day_start(), 100.0, 110.0, 90.0, 105.0, 1.0),
        Candle::new(day_start() + 86_400_000, f64::NAN, 110.0, 90.0, 105.0, 1.0),
        Candle::new(day_start() + 172_800_000, 100.0, 90.0, 110.0, 105.0, 1.0),
    ];
    let written = db::upsert_candles(&db_handle.pool, "ES1", Timeframe::D1, &bars)
        .await
        .unwrap();
    assert_eq!(written, 1);
    assert_eq!(
        db::count_candles(&db_handle.pool, "ES1", Timeframe::D1)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn indicators_join_onto_candles() {
    let db_handle = test_db().await;
    let start = day_start();
    let bars = trend_bars(4, start, 86_400_000, 100.0, 1.0);
    db::upsert_candles(&db_handle.pool, "SOLUSDT", Timeframe::D1, &bars)
        .await
        .unwrap();

    db::replace_indicators(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::D1,
        &[
            (start, None, None),
            (start + 86_400_000, Some(101.0), None),
            (start + 172_800_000, Some(102.0), Some(99.0)),
            (start + 259_200_000, Some(103.0), Some(100.0)),
        ],
    )
    .await
    .unwrap();

    let series = db::load_series(&db_handle.pool, "SOLUSDT", Timeframe::D1, 10)
        .await
        .unwrap();
    assert_eq!(series.len(), 4);
    assert_eq!(series[0].ema50, None);
    assert_eq!(series[1].ema50, Some(101.0));
    assert_eq!(series[1].ema200, None);
    assert_eq!(series[3].ema200, Some(100.0));

    let latest = db::latest_series_row(&db_handle.pool, "SOLUSDT", Timeframe::D1)
        .await
        .unwrap()
        .expect("latest row");
    assert_eq!(latest.ema50, Some(103.0));

    // Replacing indicators must not leave stale rows behind.
    db::replace_indicators(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::D1,
        &[(start, Some(1.0), None)],
    )
    .await
    .unwrap();
    let series = db::load_series(&db_handle.pool, "SOLUSDT", Timeframe::D1, 10)
        .await
        .unwrap();
    assert_eq!(series[1].ema50, None);
}

#[tokio::test]
async fn previous_close_returns_the_bar_before_the_last() {
    let db_handle = test_db().await;
    let start = day_start();
    let bars = vec![
        Candle::new(start, 10.0, 10.0, 10.0, 10.0, 1.0),
        Candle::new(start + 86_400_000, 11.0, 11.0, 11.0, 12.0, 1.0),
    ];
    db::upsert_candles(&db_handle.pool, "ES1", Timeframe::D1, &bars)
        .await
        .unwrap();
    assert_eq!(
        db::previous_close(&db_handle.pool, "ES1", Timeframe::D1)
            .await
            .unwrap(),
        Some(10.0)
    );
    assert_eq!(
        db::previous_close(&db_handle.pool, "NOPE", Timeframe::D1)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn sync_state_round_trips() {
    let db_handle = test_db().await;
    assert!(db::get_sync(&db_handle.pool, "SOLUSDT", Timeframe::H4)
        .await
        .unwrap()
        .is_none());

    db::upsert_candles(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::H4,
        &trend_bars(3, day_start(), Timeframe::H4.bucket_ms(), 5.0, 0.0),
    )
    .await
    .unwrap();

    db::record_sync(&db_handle.pool, "SOLUSDT", Timeframe::H4, 3, "ok", None)
        .await
        .unwrap();
    let state = db::get_sync(&db_handle.pool, "SOLUSDT", Timeframe::H4)
        .await
        .unwrap()
        .expect("sync state");
    assert_eq!(state.status, "ok");
    assert_eq!(state.bars, 3);
    assert!(state.last_bar.is_some());

    db::record_sync(
        &db_handle.pool,
        "SOLUSDT",
        Timeframe::H4,
        3,
        "error",
        Some("boom"),
    )
    .await
    .unwrap();
    let state = db::get_sync(&db_handle.pool, "SOLUSDT", Timeframe::H4)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.status, "error");
    assert_eq!(state.message.as_deref(), Some("boom"));
}

#[tokio::test]
async fn limit_is_clamped_and_returns_the_newest_bars() {
    let db_handle = test_db().await;
    let start = day_start();
    let bars = trend_bars(50, start, 86_400_000, 100.0, 1.0);
    db::upsert_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, &bars)
        .await
        .unwrap();

    let tail = db::load_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, 10)
        .await
        .unwrap();
    assert_eq!(tail.len(), 10);
    assert_eq!(tail.last().unwrap().open_time, start + 49 * 86_400_000);

    // A zero/negative limit still returns something usable instead of nothing.
    let clamped = db::load_candles(&db_handle.pool, "BTCUSDT", Timeframe::D1, 0)
        .await
        .unwrap();
    assert_eq!(clamped.len(), 1);
}

#[tokio::test]
async fn trend_run_reports_the_last_flip_and_its_length() {
    let db_handle = test_db().await;
    let step = Timeframe::D1.bucket_ms();
    let start = day_start();

    // 10 bearish bars, then 6 bullish ones: the flip is on bar 10.
    let values: Vec<(i64, Option<f64>, Option<f64>)> = (0..16)
        .map(|idx| {
            let open_time = start + idx * step;
            if idx < 10 {
                (open_time, Some(9.0), Some(10.0))
            } else {
                (open_time, Some(11.0), Some(10.0))
            }
        })
        .collect();
    db::replace_indicators(&db_handle.pool, "SOLUSDT", Timeframe::D1, &values)
        .await
        .unwrap();

    let run = db::trend_run(&db_handle.pool, "SOLUSDT", Timeframe::D1)
        .await
        .unwrap();
    assert_eq!(run, Some((start + 10 * step, 6)));
}

#[tokio::test]
async fn trend_run_is_empty_without_indicators() {
    let db_handle = test_db().await;
    let run = db::trend_run(&db_handle.pool, "BTCUSDT", Timeframe::H4)
        .await
        .unwrap();
    assert_eq!(run, None);
}

#[tokio::test]
async fn trend_run_covers_a_series_that_never_flipped() {
    let db_handle = test_db().await;
    let step = Timeframe::H4.bucket_ms();
    let start = day_start();
    let values: Vec<(i64, Option<f64>, Option<f64>)> = (0..5)
        .map(|idx| (start + idx * step, Some(11.0), Some(10.0)))
        .collect();
    db::replace_indicators(&db_handle.pool, "ES1", Timeframe::H4, &values)
        .await
        .unwrap();

    let run = db::trend_run(&db_handle.pool, "ES1", Timeframe::H4)
        .await
        .unwrap();
    assert_eq!(run, Some((start, 5)), "the trend is as old as the series");
}
