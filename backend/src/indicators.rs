//! Exponential moving averages.
//!
//! The series never skips warm-up: index `i < period - 1` is `None`, the first
//! value is seeded with the simple average of the first `period` closes, and every
//! later value uses the standard `prev + k * (close - prev)` recursion.

pub const EMA_FAST: usize = 50;
pub const EMA_SLOW: usize = 200;

/// EMA over a close series, same length as the input.
pub fn ema_series(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; closes.len()];
    if period == 0 || closes.len() < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let seed: f64 = closes[..period].iter().sum::<f64>() / period as f64;
    let mut prev = seed;
    out[period - 1] = Some(seed);

    for (i, close) in closes.iter().enumerate().skip(period) {
        let next = prev + k * (close - prev);
        out[i] = Some(next);
        prev = next;
    }
    out
}

/// The last EMA value, if the series is long enough.
pub fn ema_last(closes: &[f64], period: usize) -> Option<f64> {
    ema_series(closes, period).last().copied().flatten()
}

/// Percentage change between two closes, guarding against a zero/absent base.
pub fn pct_change(prev: f64, current: f64) -> Option<f64> {
    if !prev.is_finite() || !current.is_finite() || prev == 0.0 {
        return None;
    }
    Some((current - prev) / prev * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn short_series_has_no_ema() {
        let closes = vec![1.0, 2.0, 3.0];
        assert!(ema_series(&closes, 5).iter().all(|v| v.is_none()));
        assert!(ema_last(&closes, 5).is_none());
    }

    #[test]
    fn first_value_is_the_simple_average() {
        let closes = vec![10.0, 20.0, 30.0, 40.0];
        let ema = ema_series(&closes, 4);
        assert_eq!(ema.len(), closes.len());
        assert!(ema[..3].iter().all(|v| v.is_none()));
        assert!(approx(ema[3].unwrap(), 25.0));
    }

    #[test]
    fn recursion_matches_hand_computation() {
        // period 3 over 1..=6 -> seed = (1+2+3)/3 = 2, k = 0.5
        // 4: 2 + 0.5*(4-2) = 3
        // 5: 3 + 0.5*(5-3) = 4
        // 6: 4 + 0.5*(6-4) = 5
        let closes: Vec<f64> = (1..=6).map(|v| v as f64).collect();
        let ema = ema_series(&closes, 3);
        assert!(approx(ema[2].unwrap(), 2.0));
        assert!(approx(ema[3].unwrap(), 3.0));
        assert!(approx(ema[4].unwrap(), 4.0));
        assert!(approx(ema[5].unwrap(), 5.0));
    }

    #[test]
    fn constant_series_converges_to_the_constant() {
        let closes = vec![42.0; 300];
        let ema = ema_last(&closes, EMA_SLOW).unwrap();
        assert!(approx(ema, 42.0));
    }

    #[test]
    fn rising_series_keeps_ema_below_price() {
        let closes: Vec<f64> = (0..300).map(|i| 100.0 + i as f64).collect();
        let ema = ema_last(&closes, EMA_FAST).unwrap();
        let last = *closes.last().unwrap();
        assert!(ema < last, "ema {ema} should lag price {last}");
        // A 50 period EMA of a straight line lags by a fixed amount, far below the price.
        assert!(last - ema > 10.0);
    }

    #[test]
    fn fast_ema_reacts_before_slow_ema() {
        let mut closes = vec![100.0; 400];
        closes.extend(std::iter::repeat(200.0).take(20));
        let fast = ema_last(&closes, EMA_FAST).unwrap();
        let slow = ema_last(&closes, EMA_SLOW).unwrap();
        assert!(fast > slow, "fast {fast} should be above slow {slow} after a jump");
    }

    #[test]
    fn pct_change_handles_degenerate_input() {
        assert_eq!(pct_change(100.0, 101.0), Some(1.0));
        assert_eq!(pct_change(0.0, 1.0), None);
        assert_eq!(pct_change(f64::NAN, 1.0), None);
    }
}
