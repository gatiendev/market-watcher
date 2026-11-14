//! Core domain types: what we watch, on which timeframe, and one candle of data.

use serde::{Deserialize, Serialize};

/// The two timeframes the dashboard offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Timeframe {
    H4,
    D1,
}

impl Timeframe {
    pub const ALL: [Timeframe; 2] = [Timeframe::H4, Timeframe::D1];

    pub fn as_str(&self) -> &'static str {
        match self {
            Timeframe::H4 => "H4",
            Timeframe::D1 => "D1",
        }
    }

    /// Accepts "H4"/"4h"/"D1"/"1d" so query strings are forgiving.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "H4" | "4H" => Some(Timeframe::H4),
            "D1" | "1D" => Some(Timeframe::D1),
            _ => None,
        }
    }

    /// Width of one candle, in milliseconds.
    pub fn bucket_ms(&self) -> i64 {
        match self {
            Timeframe::H4 => 4 * 60 * 60 * 1000,
            Timeframe::D1 => 24 * 60 * 60 * 1000,
        }
    }

    /// How much history a fresh sync should ask for. Comfortably more than the
    /// 200 bars the slow EMA needs to warm up.
    pub fn initial_history_days(&self) -> i64 {
        match self {
            Timeframe::H4 => 730,
            Timeframe::D1 => 1500,
        }
    }

    /// Minimum bars we want stored before the series is considered "warm".
    pub fn min_bars(&self) -> usize {
        200
    }
}

/// One OHLCV bar. `open_time` is the bucket start in epoch milliseconds (UTC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub open_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Candle {
    pub fn new(open_time: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Self {
        Self {
            open_time,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// A bar is usable only if every price field is finite and the range is sane.
    pub fn is_valid(&self) -> bool {
        self.open.is_finite()
            && self.high.is_finite()
            && self.low.is_finite()
            && self.close.is_finite()
            && self.volume.is_finite()
            && self.high >= self.low
            && self.high >= self.open
            && self.high >= self.close
            && self.low <= self.open
            && self.low <= self.close
            && self.open_time >= 0
    }
}

/// Where the bars come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Binance,
    Yahoo,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Binance => "Binance",
            Source::Yahoo => "Yahoo Finance",
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            Source::Binance => "binance",
            Source::Yahoo => "yahoo",
        }
    }
}

/// A symbol the app tracks, plus how to fetch it.
#[derive(Debug, Clone, Copy)]
pub struct SymbolSpec {
    /// Name the app uses everywhere (and what the UI shows in monospace).
    pub symbol: &'static str,
    /// Human label for the dashboard.
    pub label: &'static str,
    /// "crypto" | "fx" | "index".
    pub kind: &'static str,
    pub source: Source,
    /// Ticker the provider expects (Binance uses the same string, Yahoo does not).
    pub provider_symbol: &'static str,
    /// Price decimals used by the UI.
    pub decimals: u8,
}

pub const SYMBOLS: [SymbolSpec; 4] = [
    SymbolSpec {
        symbol: "SOLUSDT",
        label: "SOL / USDT",
        kind: "crypto",
        source: Source::Binance,
        provider_symbol: "SOLUSDT",
        decimals: 2,
    },
    SymbolSpec {
        symbol: "BTCUSDT",
        label: "BTC / USDT",
        kind: "crypto",
        source: Source::Binance,
        provider_symbol: "BTCUSDT",
        decimals: 2,
    },
    SymbolSpec {
        symbol: "EURUSD",
        label: "EUR / USD",
        kind: "fx",
        source: Source::Yahoo,
        provider_symbol: "EURUSD=X",
        decimals: 5,
    },
    SymbolSpec {
        symbol: "ES1",
        label: "S&P 500 E-mini",
        kind: "index",
        source: Source::Yahoo,
        provider_symbol: "ES=F",
        decimals: 2,
    },
];

/// Look up a symbol case-insensitively.
pub fn find_symbol(raw: &str) -> Option<&'static SymbolSpec> {
    let needle = raw.trim().to_ascii_uppercase();
    SYMBOLS
        .iter()
        .find(|spec| spec.symbol.eq_ignore_ascii_case(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbol_is_unique_and_parsable() {
        let mut seen = std::collections::HashSet::new();
        for spec in SYMBOLS {
            assert!(seen.insert(spec.symbol), "duplicate symbol {}", spec.symbol);
            assert!(find_symbol(spec.symbol).is_some());
            assert!(find_symbol(&spec.symbol.to_lowercase()).is_some());
        }
    }

    #[test]
    fn timeframe_parsing_is_forgiving() {
        assert_eq!(Timeframe::parse("h4"), Some(Timeframe::H4));
        assert_eq!(Timeframe::parse(" 4h "), Some(Timeframe::H4));
        assert_eq!(Timeframe::parse("D1"), Some(Timeframe::D1));
        assert_eq!(Timeframe::parse("1d"), Some(Timeframe::D1));
        assert_eq!(Timeframe::parse("M15"), None);
    }

    #[test]
    fn candle_validation_rejects_broken_bars() {
        assert!(Candle::new(0, 1.0, 2.0, 0.5, 1.5, 10.0).is_valid());
        // high below low
        assert!(!Candle::new(0, 1.0, 0.4, 0.5, 1.5, 10.0).is_valid());
        // close above high
        assert!(!Candle::new(0, 1.0, 2.0, 0.5, 2.5, 10.0).is_valid());
        assert!(!Candle::new(0, f64::NAN, 2.0, 0.5, 1.5, 10.0).is_valid());
    }
}
