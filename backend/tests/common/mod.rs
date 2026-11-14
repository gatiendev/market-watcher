//! Shared helpers for the integration tests: a throwaway database, deterministic
//! candle generators, and a fake candle source so no network is needed.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;
use market_watcher::db;
use market_watcher::models::{Candle, SymbolSpec, Timeframe};
use market_watcher::providers::{CandleSource, SourceResolver};
use sqlx::SqlitePool;
use tempfile::TempDir;

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 3_600_000;

pub struct TestDb {
    _dir: TempDir,
    pub pool: SqlitePool,
}

/// A fresh on-disk SQLite database (temp dir removed when the test ends).
pub async fn test_db() -> TestDb {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("market.db");
    let pool = db::open(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .expect("open test db");
    TestDb { _dir: dir, pool }
}

/// Bars spaced `step_ms` apart starting at `start`, price drifting with `rate`.
pub fn trend_bars(count: usize, start: i64, step_ms: i64, base: f64, rate: f64) -> Vec<Candle> {
    (0..count)
        .map(|idx| {
            let open = base + rate * idx as f64;
            let close = open + rate;
            let high = open.max(close) + 0.5;
            let low = open.min(close) - 0.5;
            Candle::new(start + idx as i64 * step_ms, open, high, low, close, 100.0 + idx as f64)
        })
        .collect()
}

/// A source whose response is fully controlled by the test.
pub struct FakeSource {
    bars: Mutex<Vec<Candle>>,
    failure: Mutex<Option<String>>,
    calls: AtomicUsize,
    last_since: Mutex<Option<i64>>,
    last_timeframe: Mutex<Option<Timeframe>>,
}

impl FakeSource {
    pub fn new(bars: Vec<Candle>) -> Self {
        Self {
            bars: Mutex::new(bars),
            failure: Mutex::new(None),
            calls: AtomicUsize::new(0),
            last_since: Mutex::new(None),
            last_timeframe: Mutex::new(None),
        }
    }

    pub fn set_bars(&self, bars: Vec<Candle>) {
        *self.bars.lock().unwrap() = bars;
    }

    pub fn fail_with(&self, message: &str) {
        *self.failure.lock().unwrap() = Some(message.to_string());
    }

    pub fn heal(&self) {
        *self.failure.lock().unwrap() = None;
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn last_since(&self) -> Option<i64> {
        *self.last_since.lock().unwrap()
    }

    pub fn last_timeframe(&self) -> Option<Timeframe> {
        *self.last_timeframe.lock().unwrap()
    }
}

#[async_trait]
impl CandleSource for FakeSource {
    async fn fetch(
        &self,
        _spec: &SymbolSpec,
        timeframe: Timeframe,
        since: Option<i64>,
    ) -> anyhow::Result<Vec<Candle>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_since.lock().unwrap() = since;
        *self.last_timeframe.lock().unwrap() = Some(timeframe);

        if let Some(message) = self.failure.lock().unwrap().clone() {
            return Err(anyhow!(message));
        }

        let bars = self.bars.lock().unwrap();
        let mut out: Vec<Candle> = match since {
            Some(ts) => bars.iter().filter(|bar| bar.open_time >= ts).cloned().collect(),
            None => bars.clone(),
        };
        out.sort_by_key(|bar| bar.open_time);
        Ok(out)
    }
}

/// Resolver that hands the same fake source to every symbol.
pub struct FixedResolver {
    pub source: FakeSource,
}

impl FixedResolver {
    pub fn new(source: FakeSource) -> Self {
        Self { source }
    }
}

impl SourceResolver for FixedResolver {
    fn source_for(&self, _spec: &SymbolSpec) -> &dyn CandleSource {
        &self.source
    }
}

/// D1-aligned start time, far enough in the past to hold long series.
pub fn day_start() -> i64 {
    1_700_000_000_000 - (1_700_000_000_000 % 86_400_000)
}

pub fn h4_start() -> i64 {
    let bucket = Timeframe::H4.bucket_ms();
    1_700_000_000_000 - (1_700_000_000_000 % bucket)
}
