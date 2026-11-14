//! Market data providers.
//!
//! Crypto pairs come straight from Binance (native 4h and 1d candles). EURUSD and
//! the S&P E-mini come from Yahoo Finance, which serves 1h and 1d bars; 4h candles
//! for those are built by resampling the hourly stream.

pub mod binance;
pub mod yahoo;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::models::{Candle, Source, SymbolSpec, Timeframe};

/// Anything that can hand back candles for a symbol.
#[async_trait]
pub trait CandleSource: Send + Sync {
    /// `since` is the bucket start (epoch ms) to resume from; `None` means
    /// "give me as much history as you reasonably can".
    async fn fetch(
        &self,
        spec: &SymbolSpec,
        timeframe: Timeframe,
        since: Option<i64>,
    ) -> Result<Vec<Candle>>;
}

/// Picks the provider for a symbol. Real code uses `Sources`; tests plug in fakes.
pub trait SourceResolver: Send + Sync {
    fn source_for(&self, spec: &SymbolSpec) -> &dyn CandleSource;
}

/// A pair of providers, dispatched by `SymbolSpec::source`.
pub struct Sources {
    pub binance: binance::BinanceSource,
    pub yahoo: yahoo::YahooSource,
}

impl Sources {
    pub fn new() -> Result<Self> {
        Ok(Self {
            binance: binance::BinanceSource::new(build_client()?),
            yahoo: yahoo::YahooSource::new(build_client()?),
        })
    }

    pub fn for_spec(&self, spec: &SymbolSpec) -> &dyn CandleSource {
        match spec.source {
            Source::Binance => &self.binance,
            Source::Yahoo => &self.yahoo,
        }
    }
}

impl SourceResolver for Sources {
    fn source_for(&self, spec: &SymbolSpec) -> &dyn CandleSource {
        Sources::for_spec(self, spec)
    }
}

pub fn build_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        // Some public endpoints reject the default reqwest agent.
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

/// GET a JSON document, retrying transient failures (429, 5xx, network blips).
pub async fn get_json(client: &reqwest::Client, url: &str, attempts: u32) -> Result<Value> {
    let attempts = attempts.max(1);
    let mut last_error = anyhow!("no attempt made for {url}");

    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt - 1))).await;
        }
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.json::<Value>().await {
                        Ok(json) => return Ok(json),
                        Err(err) => last_error = anyhow!("bad json from {url}: {err}"),
                    }
                } else if status.as_u16() == 429 || status.is_server_error() {
                    last_error = anyhow!("{url} returned HTTP {status}");
                } else {
                    // 4xx other than 429 will not fix themselves.
                    let body = response.text().await.unwrap_or_default();
                    return Err(anyhow!(
                        "{url} returned HTTP {status}: {}",
                        body.chars().take(200).collect::<String>()
                    ));
                }
            }
            Err(err) => last_error = anyhow!("request to {url} failed: {err}"),
        }
    }
    Err(last_error)
}

/// Floored bucket start for a provider timestamp, so bars always key on the same grid.
pub fn align_to_bucket(ts_ms: i64, bucket_ms: i64) -> i64 {
    if bucket_ms <= 0 {
        ts_ms
    } else {
        ts_ms - ts_ms.rem_euclid(bucket_ms)
    }
}

/// Milliseconds -> ISO-8601 for log lines.
pub fn iso(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| ts_ms.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_matches_bucket_grid() {
        assert_eq!(align_to_bucket(3_600_000, 14_400_000), 0);
        assert_eq!(align_to_bucket(14_400_000, 14_400_000), 14_400_000);
        assert_eq!(align_to_bucket(17_000_000, 14_400_000), 14_400_000);
        assert_eq!(align_to_bucket(0, 86_400_000), 0);
        assert_eq!(align_to_bucket(17_000_000, 0), 17_000_000);
    }

    #[test]
    fn iso_is_readable() {
        assert_eq!(iso(0), "1970-01-01 00:00 UTC");
    }
}
