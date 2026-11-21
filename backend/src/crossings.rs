//! EMA 50 / EMA 200 crossings and what price did afterwards.
//!
//! A crossing is the bar where the fast EMA moves from one side of the slow EMA
//! to the other - the same rule the watchlist calls the trend, so the newest
//! crossing of a series is always the bar `trend_since` reports.
//!
//! A **run** lasts from its crossing bar to the bar before the opposite crossing
//! (or to the newest bar while it is still the current trend), because that is
//! the trade the crossing implies. Inside a run:
//!
//! - `peak_pct` is the best favourable excursion measured from the crossing close,
//! - `days_to_peak` is how long the run took to get there,
//! - `trough_pct` is the worst adverse excursion seen *before* that peak,
//! - `trough_to_peak_pct` is the swing from that adverse extreme up to the peak.
//!
//! Every number is direction-relative: for a down-cross "favourable" means price
//! fell, so a positive percentage always means the signal worked.

use serde::Serialize;

use crate::models::Timeframe;

const DAY_MS: f64 = 86_400_000.0;

/// One bar as this module needs it: prices plus the two EMAs, which are `None`
/// until each average has warmed up.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesBar {
    pub open_time: i64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub ema50: Option<f64>,
    pub ema200: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Crossing {
    pub direction: Direction,
    /// Index of the crossing bar inside the scanned slice.
    pub index: usize,
    /// Bucket start of the crossing bar, epoch milliseconds.
    pub open_time: i64,
    /// Close of the crossing bar: the price the measurement starts from.
    pub price: f64,
    pub ema50: f64,
    pub ema200: f64,
    /// Best favourable excursion from `price`, in percent.
    pub peak_pct: f64,
    /// Bars from the crossing bar to the peak bar.
    pub peak_bars: usize,
    /// `peak_bars` expressed in days (H4: 1/6 day per bar).
    pub days_to_peak: f64,
    /// Worst adverse excursion from `price` before the peak, in percent (<= 0).
    pub trough_pct: f64,
    /// Favourable swing from that adverse extreme to the peak, in percent.
    pub trough_to_peak_pct: f64,
    /// Bars the run lasted: crossing bar up to the last bar of the run.
    pub run_bars: usize,
    /// `run_bars` expressed in days.
    pub run_days: f64,
    /// Close-to-close result of the run, direction-relative, in percent.
    pub exit_pct: f64,
    /// True while this is the current trend (the run has no opposite crossing yet).
    pub open: bool,
}

/// Every confirmed crossing in `bars`, oldest first.
pub fn find_crossings(bars: &[SeriesBar], timeframe: Timeframe) -> Vec<Crossing> {
    let mut out = Vec::new();
    // Start of the run in progress, and which side the fast EMA is on.
    let mut run: Option<(usize, Direction)> = None;

    for (index, bar) in bars.iter().enumerate() {
        let (Some(fast), Some(slow)) = (bar.ema50, bar.ema200) else {
            continue;
        };
        let direction = if fast > slow {
            Direction::Up
        } else {
            Direction::Down
        };

        match run {
            // The first bar with both EMAs opens a run without a crossing: there
            // was nothing to cross from yet.
            None => run = Some((index, direction)),
            Some((start, previous)) if previous != direction => {
                if let Some(crossing) = measure(bars, start, previous, index, timeframe, false) {
                    out.push(crossing);
                }
                run = Some((index, direction));
            }
            Some(_) => {}
        }
    }

    if let Some((start, direction)) = run {
        if let Some(crossing) = measure(bars, start, direction, bars.len(), timeframe, true) {
            out.push(crossing);
        }
    }

    out
}

/// Measure the run that starts at `start` and ends before `end` (exclusive).
fn measure(
    bars: &[SeriesBar],
    start: usize,
    direction: Direction,
    end: usize,
    timeframe: Timeframe,
    open: bool,
) -> Option<Crossing> {
    if end <= start || end > bars.len() {
        return None;
    }
    let anchor = &bars[start];
    let price = anchor.close;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }

    let up = direction == Direction::Up;
    // Favourable extreme of the run, keeping the earliest bar on a tie: the
    // first time the run reached its best price is the honest "days to peak".
    let mut peak_index = start;
    let mut peak_extreme = if up {
        bars[start].high
    } else {
        bars[start].low
    };
    for (offset, bar) in bars[start..end].iter().enumerate() {
        let index = start + offset;
        let extreme = if up { bar.high } else { bar.low };
        let better = if up {
            extreme > peak_extreme
        } else {
            extreme < peak_extreme
        };
        if better {
            peak_extreme = extreme;
            peak_index = index;
        }
    }

    // Worst adverse price before (and including) the peak bar.
    let mut adverse = if up { f64::INFINITY } else { f64::NEG_INFINITY };
    for bar in &bars[start..=peak_index] {
        let extreme = if up { bar.low } else { bar.high };
        adverse = if up {
            adverse.min(extreme)
        } else {
            adverse.max(extreme)
        };
    }

    let last = end - 1;
    let exit_price = bars[last].close;

    Some(Crossing {
        direction,
        index: start,
        open_time: anchor.open_time,
        price,
        ema50: anchor.ema50?,
        ema200: anchor.ema200?,
        peak_pct: if up {
            (peak_extreme / price - 1.0) * 100.0
        } else {
            (1.0 - peak_extreme / price) * 100.0
        },
        peak_bars: peak_index - start,
        days_to_peak: (peak_index - start) as f64 * timeframe.bucket_ms() as f64 / DAY_MS,
        trough_pct: if up {
            (adverse / price - 1.0) * 100.0
        } else {
            (1.0 - adverse / price) * 100.0
        },
        trough_to_peak_pct: if up {
            (peak_extreme / adverse - 1.0) * 100.0
        } else {
            (1.0 - peak_extreme / adverse) * 100.0
        },
        run_bars: last - start,
        run_days: (last - start) as f64 * timeframe.bucket_ms() as f64 / DAY_MS,
        exit_pct: if up {
            (exit_price / price - 1.0) * 100.0
        } else {
            (1.0 - exit_price / price) * 100.0
        },
        open,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    /// A daily bar with explicit EMA values, so crossings are exact.
    fn bar(
        index: usize,
        close: f64,
        high: f64,
        low: f64,
        ema50: Option<f64>,
        ema200: Option<f64>,
    ) -> SeriesBar {
        SeriesBar {
            open_time: index as i64 * DAY,
            high,
            low,
            close,
            ema50,
            ema200,
        }
    }

    /// Warm-up, one up run that peaks then closes, then an open down run.
    ///
    /// Cross up at bar 1 (close 100, low 99), peak 108 at bar 3, a deeper low at
    /// bar 4 that must NOT count as the trough, cross down at bar 5 (close 103),
    /// then a falling run whose lowest low is the newest bar.
    fn fixture() -> Vec<SeriesBar> {
        vec![
            bar(0, 100.0, 101.0, 99.0, None, None),
            bar(1, 100.0, 101.0, 99.0, Some(101.0), Some(100.0)),
            bar(2, 102.0, 104.0, 101.0, Some(102.0), Some(100.0)),
            bar(3, 105.0, 108.0, 103.0, Some(104.0), Some(101.0)),
            bar(4, 103.0, 105.0, 90.0, Some(105.0), Some(102.0)),
            bar(5, 103.0, 104.0, 102.0, Some(100.0), Some(103.0)),
            bar(6, 99.0, 101.0, 97.0, Some(98.0), Some(102.0)),
            bar(7, 100.0, 102.0, 96.0, Some(96.0), Some(101.0)),
        ]
    }

    #[test]
    fn an_up_cross_measures_peak_trough_and_days_to_peak() {
        let crossings = find_crossings(&fixture(), Timeframe::D1);
        assert_eq!(crossings.len(), 2, "one up run, one down run");

        let up = &crossings[0];
        assert_eq!(up.direction, Direction::Up);
        assert_eq!(up.index, 1);
        assert_eq!(up.open_time, DAY);
        assert_eq!(up.price, 100.0);
        assert!(
            (up.peak_pct - 8.0).abs() < 1e-9,
            "high 108 from a close of 100"
        );
        assert_eq!(up.peak_bars, 2);
        assert_eq!(up.days_to_peak, 2.0);
        assert!(
            (up.trough_pct + 1.0).abs() < 1e-9,
            "low 99 before the peak, not the 90 printed after it"
        );
        assert!((up.trough_to_peak_pct - 9.090_909).abs() < 1e-4);
        assert_eq!(
            up.run_bars, 3,
            "bars 1..4, the run ends before the down cross"
        );
        assert!((up.exit_pct - 3.0).abs() < 1e-9, "close 103 from 100");
        assert!(!up.open);
    }

    #[test]
    fn a_down_cross_is_measured_in_the_trade_direction() {
        let crossings = find_crossings(&fixture(), Timeframe::D1);
        let down = &crossings[1];

        assert_eq!(down.direction, Direction::Down);
        assert_eq!(down.index, 5);
        assert_eq!(down.price, 103.0);
        assert!((down.peak_pct - (1.0 - 96.0 / 103.0) * 100.0).abs() < 1e-9);
        assert_eq!(down.peak_bars, 2, "the newest bar is the low so far");
        assert!(
            (down.trough_pct - (1.0 - 104.0 / 103.0) * 100.0).abs() < 1e-9,
            "bar 5 high 104 is the worst adverse print"
        );
        assert!((down.trough_to_peak_pct - (1.0 - 96.0 / 104.0) * 100.0).abs() < 1e-9);
        assert_eq!(down.run_bars, 2);
        assert!(down.open, "no opposite crossing yet");
    }

    #[test]
    fn the_first_comparable_bar_is_a_start_not_a_crossing() {
        let bars = vec![
            bar(0, 100.0, 101.0, 99.0, None, None),
            bar(1, 100.0, 101.0, 99.0, Some(99.0), Some(100.0)),
            bar(2, 100.0, 101.0, 99.0, Some(99.0), Some(100.0)),
        ];
        let crossings = find_crossings(&bars, Timeframe::D1);
        assert_eq!(crossings.len(), 1, "only the open run, no crossing");
        assert_eq!(crossings[0].index, 1);
        assert_eq!(crossings[0].direction, Direction::Down);
        assert!(crossings[0].open);
    }

    #[test]
    fn equal_emas_count_as_bearish_like_the_watchlist_trend() {
        let bars = vec![
            bar(0, 100.0, 101.0, 99.0, Some(101.0), Some(100.0)),
            bar(1, 100.0, 101.0, 99.0, Some(100.0), Some(100.0)),
        ];
        let crossings = find_crossings(&bars, Timeframe::D1);
        assert_eq!(crossings.len(), 2);
        assert_eq!(crossings[1].direction, Direction::Down);
        assert_eq!(crossings[1].index, 1);
    }

    #[test]
    fn an_h4_run_reports_days_not_bars() {
        let mut bars = fixture();
        for (index, bar) in bars.iter_mut().enumerate() {
            bar.open_time = index as i64 * Timeframe::H4.bucket_ms();
        }
        let crossings = find_crossings(&bars, Timeframe::H4);
        assert_eq!(crossings[0].peak_bars, 2);
        assert!(
            (crossings[0].days_to_peak - 2.0 / 6.0).abs() < 1e-9,
            "two H4 bars are a third of a day"
        );
    }

    #[test]
    fn a_series_without_emas_yields_nothing() {
        let bars = vec![bar(0, 100.0, 101.0, 99.0, Some(100.0), None)];
        assert!(find_crossings(&bars, Timeframe::D1).is_empty());
    }

    #[test]
    fn a_flat_run_has_no_excursion() {
        let bars = vec![
            bar(0, 100.0, 100.0, 100.0, Some(101.0), Some(100.0)),
            bar(1, 100.0, 100.0, 100.0, Some(102.0), Some(100.0)),
        ];
        let crossings = find_crossings(&bars, Timeframe::D1);
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].peak_pct, 0.0);
        assert_eq!(crossings[0].trough_pct, 0.0);
        assert_eq!(crossings[0].trough_to_peak_pct, 0.0);
        assert_eq!(crossings[0].days_to_peak, 0.0);
    }
}
