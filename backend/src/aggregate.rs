//! Resampling: turn finer bars into coarser ones.
//!
//! Yahoo only serves 1h bars for EURUSD / ES1, so H4 charts are built here from
//! hourly data. Buckets are aligned to the UTC epoch, which makes the 4h grid line
//! up with 00:00 / 04:00 / 08:00 ... UTC exactly like exchange candles do.

use crate::models::Candle;

/// The UTC-aligned start of the bucket a timestamp belongs to.
pub fn bucket_start(ts_ms: i64, bucket_ms: i64) -> i64 {
    if bucket_ms <= 0 {
        return ts_ms;
    }
    ts_ms - ts_ms.rem_euclid(bucket_ms)
}

/// Aggregate ascending candles into `bucket_ms` wide bars.
///
/// open = first open in the bucket, high/low = extremes, close = last close,
/// volume = sum. Empty buckets are skipped (markets close at the weekend), and
/// candles that are not inside a complete bucket still produce a (partial) bar so
/// the chart can show the live one.
pub fn resample(candles: &[Candle], bucket_ms: i64) -> Vec<Candle> {
    if candles.is_empty() || bucket_ms <= 0 {
        return Vec::new();
    }
    let mut out: Vec<Candle> = Vec::new();
    let mut current: Option<Candle> = None;
    let mut current_bucket = i64::MIN;

    for candle in candles.iter().filter(|c| c.is_valid()) {
        let bucket = bucket_start(candle.open_time, bucket_ms);
        match current.as_mut() {
            Some(bar) if bucket == current_bucket => {
                bar.high = bar.high.max(candle.high);
                bar.low = bar.low.min(candle.low);
                bar.close = candle.close;
                bar.volume += candle.volume;
            }
            _ => {
                if let Some(bar) = current.take() {
                    out.push(bar);
                }
                current_bucket = bucket;
                current = Some(Candle::new(
                    bucket,
                    candle.open,
                    candle.high,
                    candle.low,
                    candle.close,
                    candle.volume,
                ));
            }
        }
    }
    if let Some(bar) = current {
        out.push(bar);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Timeframe;

    const HOUR: i64 = 3_600_000;

    fn hour_bar(hour: i64, o: f64, h: f64, l: f64, c: f64, v: f64) -> Candle {
        Candle::new(hour * HOUR, o, h, l, c, v)
    }

    #[test]
    fn bucket_start_is_utc_aligned() {
        let four_h = Timeframe::H4.bucket_ms();
        assert_eq!(bucket_start(0, four_h), 0);
        assert_eq!(bucket_start(3 * HOUR, four_h), 0);
        assert_eq!(bucket_start(4 * HOUR, four_h), 4 * HOUR);
        assert_eq!(bucket_start(7 * HOUR + 1, four_h), 4 * HOUR);
        // 2026-01-01T00:00:00Z is a day boundary too
        assert_eq!(
            bucket_start(1_767_225_600_000, Timeframe::D1.bucket_ms()),
            1_767_225_600_000
        );
    }

    #[test]
    fn four_hourly_bars_merge_open_close_and_extremes() {
        let bars = vec![
            hour_bar(0, 10.0, 11.0, 9.0, 10.5, 1.0),
            hour_bar(1, 10.5, 12.0, 10.0, 11.5, 2.0),
            hour_bar(2, 11.5, 11.8, 8.0, 9.0, 3.0),
            hour_bar(3, 9.0, 9.5, 8.5, 9.2, 4.0),
        ];
        let out = resample(&bars, Timeframe::H4.bucket_ms());
        assert_eq!(out.len(), 1);
        let bar = &out[0];
        assert_eq!(bar.open_time, 0);
        assert_eq!(bar.open, 10.0);
        assert_eq!(bar.high, 12.0);
        assert_eq!(bar.low, 8.0);
        assert_eq!(bar.close, 9.2);
        assert!((bar.volume - 10.0).abs() < 1e-9);
    }

    #[test]
    fn gaps_produce_one_bar_per_populated_bucket_only() {
        // Friday 20:00, 21:00 then Monday 00:00 (weekend gap)
        let bars = vec![
            hour_bar(0, 1.0, 1.0, 1.0, 1.0, 1.0),
            hour_bar(1, 1.0, 1.0, 1.0, 1.0, 1.0),
            hour_bar(72, 2.0, 2.0, 2.0, 2.0, 1.0),
        ];
        let out = resample(&bars, Timeframe::H4.bucket_ms());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].open_time, 0);
        assert_eq!(out[1].open_time, 72 * HOUR);
        assert_eq!(out[1].open, 2.0);
    }

    #[test]
    fn partial_last_bucket_is_kept() {
        let bars = vec![
            hour_bar(4, 5.0, 6.0, 4.0, 5.5, 1.0),
            hour_bar(5, 5.5, 7.0, 5.0, 6.5, 1.0),
        ];
        let out = resample(&bars, Timeframe::H4.bucket_ms());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].open_time, 4 * HOUR);
        assert_eq!(out[0].close, 6.5);
    }

    #[test]
    fn resample_is_idempotent_on_already_coarse_bars() {
        let coarse = vec![
            hour_bar(0, 10.0, 11.0, 9.0, 10.5, 1.0),
            hour_bar(4, 10.5, 12.0, 10.0, 11.0, 1.0),
        ];
        let once = resample(&coarse, Timeframe::H4.bucket_ms());
        let twice = resample(&once, Timeframe::H4.bucket_ms());
        assert_eq!(once, twice);
    }

    #[test]
    fn invalid_bars_are_dropped_before_bucketing() {
        let bars = vec![
            hour_bar(0, 10.0, 11.0, 9.0, 10.5, 1.0),
            Candle::new(HOUR, f64::NAN, 1.0, 1.0, 1.0, 1.0),
        ];
        let out = resample(&bars, Timeframe::H4.bucket_ms());
        assert_eq!(out.len(), 1);
        assert!(out[0].is_valid());
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert!(resample(&[], Timeframe::H4.bucket_ms()).is_empty());
        assert!(resample(&[hour_bar(0, 1.0, 1.0, 1.0, 1.0, 1.0)], 0).is_empty());
    }
}
