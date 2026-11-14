//! Binance REST provider (public klines endpoint, no API key needed).

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{align_to_bucket, get_json, CandleSource};
use crate::models::{Candle, SymbolSpec, Timeframe};

const DEFAULT_BASE: &str = "https://api.binance.com";
const PAGE_LIMIT: usize = 1000;
const MAX_PAGES: usize = 8;

pub struct BinanceSource {
    client: reqwest::Client,
    base: String,
}

impl BinanceSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            base: DEFAULT_BASE.to_string(),
        }
    }

    /// Used by tests to point at a local mock server.
    pub fn with_base(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into(),
        }
    }

    fn interval(timeframe: Timeframe) -> &'static str {
        match timeframe {
            Timeframe::H4 => "4h",
            Timeframe::D1 => "1d",
        }
    }

    fn url(&self, symbol: &str, interval: &str, start: Option<i64>) -> String {
        let mut url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&limit={}",
            self.base, symbol, interval, PAGE_LIMIT
        );
        if let Some(start) = start {
            url.push_str(&format!("&startTime={start}"));
        }
        url
    }
}

#[async_trait]
impl CandleSource for BinanceSource {
    async fn fetch(
        &self,
        spec: &SymbolSpec,
        timeframe: Timeframe,
        since: Option<i64>,
    ) -> Result<Vec<Candle>> {
        let interval = Self::interval(timeframe);
        let bucket = timeframe.bucket_ms();
        let mut cursor = since.map(|ts| align_to_bucket(ts, bucket));
        let mut out: Vec<Candle> = Vec::new();

        for _ in 0..MAX_PAGES {
            let url = self.url(spec.provider_symbol, interval, cursor);
            let json = get_json(&self.client, &url, 3)
                .await
                .map_err(|err| anyhow!("binance {} {}: {err}", spec.symbol, interval))?;
            let page = parse_klines(&json)?;

            let page_len = page.len();
            let mut pushed = 0usize;
            for candle in page {
                if let Some(last) = out.last() {
                    if candle.open_time <= last.open_time {
                        // Overlap from the resumed cursor: keep the fresher bar.
                        if candle.open_time == last.open_time {
                            *out.last_mut().unwrap() = candle;
                        }
                        continue;
                    }
                }
                out.push(candle);
                pushed += 1;
            }

            // Fewer bars than a full page means we reached the live edge.
            if page_len < PAGE_LIMIT || pushed == 0 {
                break;
            }
            cursor = out.last().map(|c| c.open_time);
            if cursor.is_none() {
                break;
            }
        }

        Ok(out)
    }
}

fn parse_klines(json: &Value) -> Result<Vec<Candle>> {
    let rows = json
        .as_array()
        .ok_or_else(|| anyhow!("unexpected binance payload: {json}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let fields = row
            .as_array()
            .ok_or_else(|| anyhow!("binance kline is not an array"))?;
        if fields.len() < 6 {
            continue;
        }
        let open_time = fields[0]
            .as_i64()
            .or_else(|| fields[0].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| anyhow!("binance kline without open time"))?;
        let number = |idx: usize| -> f64 {
            fields[idx]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| fields[idx].as_f64())
                .unwrap_or(f64::NAN)
        };
        let candle = Candle::new(
            open_time,
            number(1),
            number(2),
            number(3),
            number(4),
            number(5),
        );
        if candle.is_valid() {
            out.push(candle);
        }
    }
    out.sort_by_key(|c| c.open_time);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_kline_shape() {
        let payload = serde_json::json!([
            [1700000000000i64, "100.0", "110.0", "90.0", "105.0", "1234.5", 1700014399999i64, "0", 10, "0", "0", "0"],
            [1700014400000i64, "105.0", "112.0", "101.0", "109.0", "222.0", 1700028799999i64, "0", 10, "0", "0", "0"]
        ]);
        let candles = parse_klines(&payload).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(candles[0].high, 110.0);
        assert_eq!(candles[1].close, 109.0);
        assert!(candles[0].is_valid() && candles[1].is_valid());
    }

    #[test]
    fn skips_broken_rows_and_keeps_the_good_ones() {
        let payload = serde_json::json!([
            [1i64, "100.0", "90.0", "110.0", "105.0", "1.0"],   // high < low -> invalid
            [2i64, "abc", "110.0", "90.0", "105.0", "1.0"],     // unparsable open
            [3i64, "100.0", "110.0", "90.0", "105.0", "1.0"]    // fine
        ]);
        let candles = parse_klines(&payload).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 3);
    }

    #[test]
    fn rejects_a_non_array_payload() {
        assert!(parse_klines(&serde_json::json!({"code": -1121, "msg": "Invalid symbol"})).is_err());
    }

    #[test]
    fn interval_names_match_binance() {
        assert_eq!(BinanceSource::interval(Timeframe::H4), "4h");
        assert_eq!(BinanceSource::interval(Timeframe::D1), "1d");
    }

    #[test]
    fn url_carries_the_resume_cursor() {
        let source = BinanceSource::with_base(reqwest::Client::new(), "http://mock");
        let url = source.url("SOLUSDT", "4h", Some(1_700_000_000_000));
        assert!(url.contains("symbol=SOLUSDT"));
        assert!(url.contains("interval=4h"));
        assert!(url.contains("startTime=1700000000000"));
        assert!(!source.url("SOLUSDT", "4h", None).contains("startTime"));
    }
}
