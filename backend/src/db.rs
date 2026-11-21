//! SQLite persistence.
//!
//! Candles are stored once per `(symbol, timeframe, open_time)` and re-fetched only
//! from the last stored bucket onwards, so a restart never re-downloads history.
//! EMA values live in their own table and are recomputed whenever bars change.

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

use crate::models::{Candle, Timeframe};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS candles (
    symbol     TEXT    NOT NULL,
    timeframe  TEXT    NOT NULL,
    open_time  INTEGER NOT NULL,
    open       REAL    NOT NULL,
    high       REAL    NOT NULL,
    low        REAL    NOT NULL,
    close      REAL    NOT NULL,
    volume     REAL    NOT NULL,
    PRIMARY KEY (symbol, timeframe, open_time)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS indicators (
    symbol     TEXT    NOT NULL,
    timeframe  TEXT    NOT NULL,
    open_time  INTEGER NOT NULL,
    ema50      REAL,
    ema200     REAL,
    PRIMARY KEY (symbol, timeframe, open_time)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS sync_state (
    symbol       TEXT    NOT NULL,
    timeframe    TEXT    NOT NULL,
    last_sync_at INTEGER NOT NULL,
    last_bar     INTEGER,
    bars         INTEGER NOT NULL,
    status       TEXT    NOT NULL,
    message      TEXT,
    PRIMARY KEY (symbol, timeframe)
) WITHOUT ROWID;
"#;

/// A candle together with the EMA values computed for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesRow {
    pub candle: Candle,
    pub ema50: Option<f64>,
    pub ema200: Option<f64>,
}

/// Last outcome of syncing one `(symbol, timeframe)` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncState {
    pub last_sync_at: i64,
    pub last_bar: Option<i64>,
    pub bars: i64,
    pub status: String,
    pub message: Option<String>,
}

/// Open (creating if needed) the database file and make sure the schema exists.
pub async fn open(path: &str) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(path)
        .with_context(|| format!("invalid sqlite url: {path}"))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(10))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(20))
        .connect_with(options)
        .await
        .with_context(|| format!("cannot open database {path}"))?;

    migrate(&pool).await?;
    Ok(pool)
}

/// Create tables. Safe to call on every start.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    for statement in SCHEMA.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

/// Insert or update bars. Re-upserting the same bucket overwrites it, which is how
/// the live (still forming) candle stays correct between refreshes.
/// Returns the number of bars handed to the database.
pub async fn upsert_candles(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    candles: &[Candle],
) -> Result<usize> {
    let valid: Vec<&Candle> = candles.iter().filter(|c| c.is_valid()).collect();
    if valid.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for candle in &valid {
        sqlx::query(
            "INSERT INTO candles (symbol, timeframe, open_time, open, high, low, close, volume)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(symbol, timeframe, open_time) DO UPDATE SET
                open = excluded.open,
                high = excluded.high,
                low = excluded.low,
                close = excluded.close,
                volume = excluded.volume",
        )
        .bind(symbol)
        .bind(timeframe.as_str())
        .bind(candle.open_time)
        .bind(candle.open)
        .bind(candle.high)
        .bind(candle.low)
        .bind(candle.close)
        .bind(candle.volume)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(valid.len())
}

/// Newest stored bucket start, used as the incremental download cursor.
pub async fn last_bar_time(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT MAX(open_time) AS last FROM candles WHERE symbol = ? AND timeframe = ?",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row.try_get::<Option<i64>, _>("last")?)
}

/// Oldest stored bar, used to walk history backwards until the series is complete.
pub async fn first_bar_time(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT MIN(open_time) AS first FROM candles WHERE symbol = ? AND timeframe = ?",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row.try_get::<Option<i64>, _>("first")?)
}

/// Where the current EMA trend started, and how many bars it has lasted.
///
/// The trend is `ema50 > ema200`; this walks the indicator series and finds the
/// most recent flip, so the UI can say how long the trend has been in place.
pub async fn trend_run(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<(i64, i64)>> {
    let row = sqlx::query(
        r#"
        WITH series AS (
            SELECT open_time,
                   CASE WHEN ema50 > ema200 THEN 1 ELSE 0 END AS up,
                   LAG(CASE WHEN ema50 > ema200 THEN 1 ELSE 0 END)
                       OVER (ORDER BY open_time) AS prev_up
            FROM indicators
            WHERE symbol = ? AND timeframe = ? AND ema50 IS NOT NULL AND ema200 IS NOT NULL
        ),
        start AS (
            SELECT COALESCE(
                (SELECT MAX(open_time) FROM series WHERE prev_up IS NOT NULL AND prev_up <> up),
                (SELECT MIN(open_time) FROM series)
            ) AS since
        )
        SELECT start.since AS since,
               (SELECT COUNT(*) FROM series WHERE open_time >= start.since) AS bars
        FROM start
        WHERE start.since IS NOT NULL
        "#,
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| (row.get::<i64, _>("since"), row.get::<i64, _>("bars"))))
}

pub async fn count_candles(pool: &SqlitePool, symbol: &str, timeframe: Timeframe) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM candles WHERE symbol = ? AND timeframe = ?")
        .bind(symbol)
        .bind(timeframe.as_str())
        .fetch_one(pool)
        .await?;
    Ok(row.try_get::<i64, _>("n")?)
}

/// The newest `limit` bars, returned oldest first (chart order).
pub async fn load_candles(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    limit: i64,
) -> Result<Vec<Candle>> {
    let rows = sqlx::query(
        "SELECT open_time, open, high, low, close, volume FROM candles
         WHERE symbol = ? AND timeframe = ?
         ORDER BY open_time DESC LIMIT ?",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .bind(limit.clamp(1, 5000))
    .fetch_all(pool)
    .await?;

    let mut candles = Vec::with_capacity(rows.len());
    for row in rows.iter().rev() {
        candles.push(read_candle(row)?);
    }
    Ok(candles)
}

/// Every stored bar for a series, oldest first. Used when recomputing EMAs.
pub async fn load_all_candles(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Vec<Candle>> {
    let rows = sqlx::query(
        "SELECT open_time, open, high, low, close, volume FROM candles
         WHERE symbol = ? AND timeframe = ?
         ORDER BY open_time ASC",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .fetch_all(pool)
    .await?;

    rows.iter().map(read_candle).collect()
}

/// Bars joined with their EMA values, oldest first.
pub async fn load_series(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    limit: i64,
) -> Result<Vec<SeriesRow>> {
    load_series_rows(pool, symbol, timeframe, Some(limit)).await
}

/// Every stored bar with its EMAs, oldest first. Used where the whole history
/// matters (crossings), not just the chart window.
pub async fn load_series_all(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Vec<SeriesRow>> {
    load_series_rows(pool, symbol, timeframe, None).await
}

async fn load_series_rows(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    limit: Option<i64>,
) -> Result<Vec<SeriesRow>> {
    const BASE: &str = "SELECT c.open_time, c.open, c.high, c.low, c.close, c.volume,
                i.ema50 AS ema50, i.ema200 AS ema200
         FROM candles c
         LEFT JOIN indicators i
           ON i.symbol = c.symbol AND i.timeframe = c.timeframe AND i.open_time = c.open_time
         WHERE c.symbol = ? AND c.timeframe = ?
         ORDER BY c.open_time DESC";
    let sql = match limit {
        Some(_) => format!("{BASE} LIMIT ?"),
        None => BASE.to_string(),
    };

    let mut query = sqlx::query(&sql).bind(symbol).bind(timeframe.as_str());
    if let Some(limit) = limit {
        query = query.bind(limit.clamp(1, 5000));
    }
    let rows = query.fetch_all(pool).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows.iter().rev() {
        out.push(SeriesRow {
            candle: read_candle(row)?,
            ema50: row.try_get::<Option<f64>, _>("ema50")?,
            ema200: row.try_get::<Option<f64>, _>("ema200")?,
        });
    }
    Ok(out)
}

/// Newest bar plus its EMAs (drives the market table).
pub async fn latest_series_row(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<SeriesRow>> {
    Ok(load_series(pool, symbol, timeframe, 1).await?.pop())
}

/// The bar just before the newest one, for the change percentage.
pub async fn previous_close(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<f64>> {
    Ok(load_candles(pool, symbol, timeframe, 2)
        .await?
        .first()
        .map(|c| c.close))
}

fn read_candle(row: &sqlx::sqlite::SqliteRow) -> Result<Candle> {
    Ok(Candle {
        open_time: row.try_get::<i64, _>("open_time")?,
        open: row.try_get::<f64, _>("open")?,
        high: row.try_get::<f64, _>("high")?,
        low: row.try_get::<f64, _>("low")?,
        close: row.try_get::<f64, _>("close")?,
        volume: row.try_get::<f64, _>("volume")?,
    })
}

/// Replace every EMA value for a series (cheap: a few thousand rows).
pub async fn replace_indicators(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    values: &[(i64, Option<f64>, Option<f64>)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM indicators WHERE symbol = ? AND timeframe = ?")
        .bind(symbol)
        .bind(timeframe.as_str())
        .execute(&mut *tx)
        .await?;
    for (open_time, ema50, ema200) in values {
        sqlx::query(
            "INSERT INTO indicators (symbol, timeframe, open_time, ema50, ema200)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(symbol)
        .bind(timeframe.as_str())
        .bind(open_time)
        .bind(ema50)
        .bind(ema200)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Record the outcome of a sync attempt (success or failure).
pub async fn record_sync(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
    bars: i64,
    status: &str,
    message: Option<&str>,
) -> Result<()> {
    let last_bar = last_bar_time(pool, symbol, timeframe).await?;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO sync_state (symbol, timeframe, last_sync_at, last_bar, bars, status, message)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(symbol, timeframe) DO UPDATE SET
            last_sync_at = excluded.last_sync_at,
            last_bar = excluded.last_bar,
            bars = excluded.bars,
            status = excluded.status,
            message = excluded.message",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .bind(now)
    .bind(last_bar)
    .bind(bars)
    .bind(status)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_sync(
    pool: &SqlitePool,
    symbol: &str,
    timeframe: Timeframe,
) -> Result<Option<SyncState>> {
    let row = sqlx::query(
        "SELECT last_sync_at, last_bar, bars, status, message FROM sync_state
         WHERE symbol = ? AND timeframe = ?",
    )
    .bind(symbol)
    .bind(timeframe.as_str())
    .fetch_optional(pool)
    .await?;

    Ok(row
        .map(|row| {
            Ok::<_, sqlx::Error>(SyncState {
                last_sync_at: row.try_get("last_sync_at")?,
                last_bar: row.try_get("last_bar")?,
                bars: row.try_get("bars")?,
                status: row.try_get("status")?,
                message: row.try_get("message")?,
            })
        })
        .transpose()?)
}
