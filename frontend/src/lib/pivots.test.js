import { describe, expect, it } from 'vitest';
import { keyLevels, levelDistance, pivotMarkers, snapDown } from './pivots.js';

const HOUR = 3600;
const DAY = 86_400;
const barTimes = Array.from({ length: 40 }, (_, idx) => 1_700_000_000 + idx * 4 * HOUR);

describe('snapDown', () => {
  it('finds the greatest bar time not after the target', () => {
    expect(snapDown(barTimes, barTimes[3] + 1)).toBe(barTimes[3]);
    expect(snapDown(barTimes, barTimes[3])).toBe(barTimes[3]);
  });

  it('returns null before the first bar and on junk input', () => {
    expect(snapDown(barTimes, barTimes[0] - 1)).toBeNull();
    expect(snapDown([], 10)).toBeNull();
    expect(snapDown(null, 10)).toBeNull();
  });
});

describe('pivotMarkers', () => {
  const T0 = barTimes[0];
  // Four H4 bars per day: only the second one carries the day's high.
  function barsAround(time, high, low, offsetIndex = 1) {
    return Array.from({ length: 6 }, (_, idx) => {
      const rising = idx === offsetIndex;
      return {
        time: time + idx * 4 * HOUR,
        open: 10,
        high: rising ? high : high - 1,
        low: rising ? low : low + 1,
        close: 10,
      };
    });
  }

  it('puts the marker on the bar that made the swing, not on the close', () => {
    const bars = barsAround(T0, 12.5, 9.5);
    const pivot = { time: T0, price: 12.5, kind: 'high' };
    const markers = pivotMarkers([pivot], bars);
    // Second bar of the day holds the high, so the arrow belongs there.
    expect(markers).toEqual([{ time: T0 + 4 * HOUR, price: 12.5, kind: 'high' }]);
  });

  it('finds the swing low bar the same way', () => {
    const bars = barsAround(T0, 12.5, 9.5, 3);
    const pivot = { time: T0, price: 9.5, kind: 'low' };
    expect(pivotMarkers([pivot], bars)[0].time).toBe(T0 + 12 * HOUR);
  });

  it('falls back to the closest bar when no bar matches the extreme exactly', () => {
    // Resampled or rounded feeds: the daily high is 12.5 but the closest bar tops
    // out at 12.4, so the arrow goes on that bar.
    const bars = barsAround(T0, 12.5, 9.5).map((bar, idx) => ({
      ...bar,
      high: idx === 2 ? 12.4 : 12.0,
    }));
    const pivot = { time: T0, price: 12.5, kind: 'high' };
    expect(pivotMarkers([pivot], bars)[0].time).toBe(T0 + 8 * HOUR);
  });

  it('keeps a daily pivot on its own daily bar', () => {
    const dayBars = [
      { time: T0, open: 10, high: 12.5, low: 9.5, close: 11 },
      { time: T0 + DAY, open: 11, high: 11.5, low: 10, close: 10.5 },
    ];
    const pivot = { time: T0, price: 12.5, kind: 'high' };
    expect(pivotMarkers([pivot], dayBars)).toEqual([{ time: T0, price: 12.5, kind: 'high' }]);
  });

  it('drops pivots older than the first stored bar', () => {
    const pivot = { time: T0 - DAY, price: 9, kind: 'low' };
    expect(pivotMarkers([pivot], barsAround(T0, 12.5, 9.5))).toEqual([]);
  });

  it('keeps a single marker per bar, sorted oldest first', () => {
    const bars = [
      ...barsAround(T0, 12, 8),
      ...barsAround(T0 + DAY, 20, 6),
    ];
    const pivots = [
      { time: T0 + DAY, price: 20, kind: 'high' },
      { time: T0, price: 8, kind: 'low' },
    ];
    const markers = pivotMarkers(pivots, bars);
    expect(markers.map((m) => m.time)).toEqual([T0 + 4 * HOUR, T0 + DAY + 4 * HOUR]);
    expect(markers[0]).toMatchObject({ kind: 'low', price: 8 });
  });

  it('ignores junk and empty input', () => {
    expect(pivotMarkers([], barsAround(T0, 12, 8))).toEqual([]);
    expect(pivotMarkers([{ time: null, price: 3 }], barsAround(T0, 12, 8))).toEqual([]);
    expect(pivotMarkers([{ time: T0, price: 3 }], [])).toEqual([]);
  });
});

describe('keyLevels', () => {
  const pivots = [
    { time: 1, price: 100.0, kind: 'low' },
    { time: 2, price: 100.3, kind: 'low' },
    { time: 3, price: 110.0, kind: 'high' },
    { time: 4, price: 120.2, kind: 'high' },
    { time: 5, price: 90.0, kind: 'low' },
    { time: 6, price: 95.0, kind: 'low' },
  ];

  it('merges pivots sitting at the same price into one level', () => {
    const { all } = keyLevels(pivots, 105);
    const around100 = all.filter((level) => Math.abs(level.price - 100.15) < 0.5);
    expect(around100).toHaveLength(1);
    expect(around100[0].touches).toBe(2);
    expect(around100[0].time).toBe(2);
  });

  it('splits resistance above and support below, nearest first', () => {
    const { above, below } = keyLevels(pivots, 105);
    expect(above.map((l) => Math.round(l.price))).toEqual([110, 120]);
    expect(below.map((l) => Math.round(l.price))).toEqual([100, 95, 90]);
  });

  it('caps the number of drawn levels', () => {
    const { all } = keyLevels(pivots, 105, { max: 2 });
    expect(all).toHaveLength(2);
  });

  it('leaves out levels sitting on top of the last price', () => {
    const tight = [
      { time: 1, price: 100.0, kind: 'low' },
      { time: 2, price: 105.05, kind: 'high' },
      { time: 3, price: 120.0, kind: 'high' },
    ];
    const { above, below, all } = keyLevels(tight, 105);
    expect(all.map((l) => Math.round(l.price))).toEqual([120, 100]);
    expect(above).toHaveLength(1);
    expect(below).toHaveLength(1);
  });

  it('fills both sides when one side runs out', () => {
    const onlyBelow = pivots.filter((p) => p.price < 105);
    const { above, below, all } = keyLevels(onlyBelow, 105, { max: 3 });
    expect(above).toHaveLength(0);
    expect(below).toHaveLength(3);
    expect(all).toHaveLength(3);
  });

  it('falls back to the most recent swings without a price', () => {
    const { above, below } = keyLevels(pivots, null, { max: 4 });
    expect([...above, ...below]).toHaveLength(4);
  });

  it('returns empty buckets for no pivots', () => {
    expect(keyLevels([], 100)).toEqual({ above: [], below: [], all: [], markers: [] });
  });
});

describe('levelDistance', () => {
  it('measures the gap in percent of the last price', () => {
    expect(levelDistance(110, 100)).toBeCloseTo(10);
    expect(levelDistance(95, 100)).toBeCloseTo(-5);
    expect(levelDistance(null, 100)).toBeNull();
    expect(levelDistance(100, 0)).toBeNull();
  });
});
