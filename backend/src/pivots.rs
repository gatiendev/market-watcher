//! Fractal pivots on the daily series.
//!
//! A bar is a **pivot high** when its high is strictly greater than the high of
//! every bar `left` bars before it and `right` bars after it; a **pivot low**
//! mirrors that on the low. Equal highs on both sides form a plateau, which is
//! not a pivot: the feature marks clean swing points, not every shoulder. A bar
//! wide enough to engulf both neighbourhoods is reported as both kinds.
//!
//! The newest `right` bars can never be pivots - the move that confirms them has
//! not happened yet - so the last possible pivot sits `right` bars back.

use serde::Serialize;

use crate::models::Candle;

/// Bars required on each side when the caller does not ask for its own width.
pub const DEFAULT_LOOKBACK: usize = 15;
/// Below this the "pivot" is just local noise.
pub const MIN_LOOKBACK: usize = 1;
/// Above this a 4-year daily series holds almost nothing.
pub const MAX_LOOKBACK: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PivotKind {
    High,
    Low,
}

impl PivotKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PivotKind::High => "high",
            PivotKind::Low => "low",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pivot {
    pub kind: PivotKind,
    /// Bucket start of the pivot bar, epoch milliseconds.
    pub open_time: i64,
    /// The pivot's extreme: the bar high for a pivot high, the low for a pivot low.
    pub price: f64,
    /// Index of that bar inside the scanned slice.
    pub index: usize,
}

/// Every confirmed pivot in `bars`, oldest first.
pub fn find_pivots(bars: &[Candle], left: usize, right: usize) -> Vec<Pivot> {
    let left = left.max(MIN_LOOKBACK);
    let right = right.max(MIN_LOOKBACK);
    let mut out = Vec::new();
    if bars.len() <= left + right {
        return out;
    }

    for index in left..bars.len() - right {
        let bar = &bars[index];
        let before = &bars[index - left..index];
        let after = &bars[index + 1..index + 1 + right];

        let is_high = before.iter().all(|other| bar.high > other.high)
            && after.iter().all(|other| bar.high > other.high);
        let is_low = before.iter().all(|other| bar.low < other.low)
            && after.iter().all(|other| bar.low < other.low);

        if is_high {
            out.push(Pivot {
                kind: PivotKind::High,
                open_time: bar.open_time,
                price: bar.high,
                index,
            });
        }
        if is_low {
            out.push(Pivot {
                kind: PivotKind::Low,
                open_time: bar.open_time,
                price: bar.low,
                index,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zigzag: high = close + 1, low = close - 1.
    fn zigzag(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(idx, close)| {
                Candle::new(
                    idx as i64 * 86_400_000,
                    *close,
                    close + 1.0,
                    close - 1.0,
                    *close,
                    10.0,
                )
            })
            .collect()
    }

    /// 10 12 15 12 9 12 16 12 7 -> peaks at index 2 (16) and 6 (17), trough at 4 (8).
    fn fixture() -> Vec<Candle> {
        zigzag(&[10.0, 12.0, 15.0, 12.0, 9.0, 12.0, 16.0, 12.0, 7.0])
    }

    #[test]
    fn finds_swings_two_bars_each_side() {
        let bars = fixture();
        let found = find_pivots(&bars, 2, 2);
        let seen: Vec<(PivotKind, f64)> = found.iter().map(|p| (p.kind, p.price)).collect();
        assert_eq!(
            seen,
            vec![
                (PivotKind::High, 16.0),
                (PivotKind::Low, 8.0),
                (PivotKind::High, 17.0),
            ]
        );
        assert_eq!(found[0].index, 2);
        assert_eq!(found[1].index, 4);
        assert_eq!(found[2].open_time, 6 * 86_400_000);
    }

    #[test]
    fn a_wider_window_finds_fewer_pivots() {
        let bars = fixture();
        let wide = find_pivots(&bars, 3, 3);
        assert_eq!(wide.len(), 1, "only the deep trough survives 3 bars each side");
        assert_eq!(wide[0].kind, PivotKind::Low);
        assert_eq!(wide[0].price, 8.0);
    }

    #[test]
    fn the_newest_bars_are_never_pivots() {
        let bars = fixture();
        // Index 8 is the series low, but nothing has traded after it yet.
        assert!(find_pivots(&bars, 2, 2).iter().all(|p| p.index <= 8 - 2));
    }

    #[test]
    fn equal_neighbours_are_not_a_pivot() {
        // Two adjacent highs of 16: neither bar beats the other, so no pivot.
        let bars = zigzag(&[10.0, 13.0, 15.0, 15.0, 13.0, 10.0]);
        assert!(find_pivots(&bars, 1, 1)
            .iter()
            .all(|p| p.kind != PivotKind::High));
    }

    #[test]
    fn a_short_series_yields_nothing() {
        let bars = zigzag(&[10.0, 12.0, 11.0]);
        assert!(find_pivots(&bars, 15, 15).is_empty());
    }
}
