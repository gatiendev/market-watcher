//! Yahoo Finance provider for symbols that have no public exchange API
//! (EURUSD, the S&P E-mini future). Serves 1h and 1d bars; 4h bars are resampled
//! from the hourly stream.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{align_to_bucket, get_json, CandleSource};
use crate::aggregate::resample;
use crate::models::{Candle, SymbolSpec, Timeframe};

const DEFAULT_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const DAY_MS: i64 = 86_400_000;

pub struct YahooSource {
    client: reqwest::Client,
    base: String,
}

impl YahooSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            base: DEFAULT_BASE.to_string(),
        }
    }

    pub fn with_base(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into(),
        }
    }

    /// Yahoo interval requested for a timeframe. H4 is built from 1h bars.
    fn interval(timeframe: Timeframe) -> &'static str {
        match timeframe {
            Timeframe::H4 => "1h",
            Timeframe::D1 => "1d",
        }
    }

    fn url(&self, symbol: &str, interval: &str, period1: i64, period2: i64) -> String {
        format!(
            "{}/{}?interval={}&period1={}&period2={}",
            self.base,
            urlencoding::encode(symbol),
            interval,
            period1,
            period2
        )
    }
}

#[async_trait]
impl CandleSource for YahooSource {
    async fn fetch(
        &self,
        spec: &SymbolSpec,
        timeframe: Timeframe,
        since: Option<i64>,
    ) -> Result<Vec<Candle>> {
        let bucket = timeframe.bucket_ms();
        let interval = Self::interval(timeframe);
        let now = chrono::Utc::now().timestamp_millis();

        // Resume one bar early so the newest (still forming) bar is refetched.
        let start_ms = match since {
            Some(ts) => align_to_bucket(ts, bucket) - bucket,
            None => now - timeframe.initial_history_days() * DAY_MS,
        };
        let period1 = start_ms.max(0) / 1000;
        let period2 = now / 1000 + 3600;

        let url = self.url(spec.provider_symbol, interval, period1, period2);
        let json = get_json(&self.client, &url, 3)
            .await
            .map_err(|err| anyhow!("yahoo {} {}: {err}", spec.symbol, interval))?;

        let raw = parse_chart(&json)?;
        let mut candles = match timeframe {
            // Resample hourly bars onto the UTC 4h grid.
            Timeframe::H4 => resample(&raw, bucket),
            // Daily bars are kept one per calendar day (UTC), whatever the session open was.
            Timeframe::D1 => raw
                .into_iter()
                .map(|mut candle| {
                    candle.open_time = align_to_bucket(candle.open_time, DAY_MS);
                    candle
                })
                .collect(),
        };

        // Drop anything older than the cursor: those bars are already stored.
        if let Some(ts) = since {
            candles.retain(|candle| candle.open_time > align_to_bucket(ts, bucket) - bucket);
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        Ok(candles)
    }
}

/// Pull `timestamp` + quote arrays out of Yahoo's chart envelope.
fn parse_chart(json: &Value) -> Result<Vec<Candle>> {
    let chart = json
        .get("chart")
        .ok_or_else(|| anyhow!("unexpected yahoo payload: {json}"))?;

    if let Some(error) = chart.get("error") {
        if !error.is_null() {
            return Err(anyhow!("yahoo error: {error}"));
        }
    }

    let result = chart
        .get("result")
        .and_then(|value| value.as_array())
        .and_then(|rows| rows.first())
        .ok_or_else(|| anyhow!("yahoo payload without chart result"))?;

    let timestamps = result
        .get("timestamp")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let quote = result
        .get("indicators")
        .and_then(|value| value.get("quote"))
        .and_then(|value| value.as_array())
        .and_then(|rows| rows.first())
        .ok_or_else(|| anyhow!("yahoo payload without quote data"))?;

    let series = |field: &str| -> Vec<Option<f64>> {
        quote
            .get(field)
            .and_then(|value| value.as_array())
            .map(|values| values.iter().map(|v| v.as_f64()).collect())
            .unwrap_or_default()
    };

    let opens = series("open");
    let highs = series("high");
    let lows = series("low");
    let closes = series("close");
    let volumes = series("volume");

    let mut out = Vec::with_capacity(timestamps.len());
    for (idx, ts) in timestamps.iter().enumerate() {
        let ts_ms = match ts.as_i64() {
            Some(sec) => sec * 1000,
            None => continue,
        };
        let pick = |values: &Vec<Option<f64>>| values.get(idx).copied().flatten();
        let (Some(open), Some(high), Some(low), Some(close)) = (
            pick(&opens),
            pick(&highs),
            pick(&lows),
            pick(&closes),
        ) else {
            // Market was closed for this slot; Yahoo pads with nulls.
            continue;
        };
        let candle = Candle::new(ts_ms, open, high, low, close, pick(&volumes).unwrap_or(0.0));
        if candle.is_valid() {
            out.push(candle);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(timestamps: Vec<i64>, quotes: Value) -> Value {
        serde_json::json!({
            "chart": {
                "error": null,
                "result": [{
                    "timestamp": timestamps,
                    "indicators": { "quote": [quotes] }
                }]
            }
        })
    }

    #[test]
    fn parses_a_normal_payload_and_drops_null_slots() {
        let payload = envelope(
            vec![1_700_000_000, 1_700_003_600, 1_700_007_200],
            serde_json::json!({
                "open": [100.0, null, 102.0],
                "high": [101.0, null, 103.0],
                "low": [99.0, null, 101.0],
                "close": [100.5, null, 102.5],
                "volume": [10, null, 20]
            }),
        );
        let candles = parse_chart(&payload).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(candles[1].close, 102.5);
    }

    #[test]
    fn missing_volume_defaults_to_zero() {
        let payload = envelope(
            vec![1_700_000_000],
            serde_json::json!({
                "open": [1.0], "high": [1.1], "low": [0.9], "close": [1.05], "volume": [null]
            }),
        );
        let candles = parse_chart(&payload).unwrap();
        assert_eq!(candles[0].volume, 0.0);
    }

    #[test]
    fn surfaces_yahoo_errors() {
        let payload = serde_json::json!({
            "chart": {"result": null, "error": {"code": "Not Found", "description": "No data found"}}
        });
        let err = parse_chart(&payload).unwrap_err().to_string();
        assert!(err.contains("No data found"), "got: {err}");
    }

    #[test]
    fn rejects_payload_without_result() {
        let payload = serde_json::json!({"chart": {"result": [], "error": null}});
        assert!(parse_chart(&payload).is_err());
    }

    #[test]
    fn interval_choice_matches_the_resampling_plan() {
        assert_eq!(YahooSource::interval(Timeframe::H4), "1h");
        assert_eq!(YahooSource::interval(Timeframe::D1), "1d");
    }

    #[test]
    fn url_encodes_the_yahoo_ticker() {
        let source = YahooSource::with_base(reqwest::Client::new(), "http://mock");
        let url = source.url("EURUSD=X", "1h", 10, 20);
        assert!(url.contains("EURUSD%3DX"), "got: {url}");
        assert!(url.contains("interval=1h"));
        assert!(url.contains("period1=10"));
    }
}
