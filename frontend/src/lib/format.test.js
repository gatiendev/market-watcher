import { describe, expect, it } from 'vitest';
import {
  changeClass,
  emaSpread,
  formatBarTime,
  formatPct,
  formatPrice,
  formatRelative,
  formatVolume,
  isNumber,
  sortSymbols,
  toCandles,
  toLine,
  toVolume,
  trendLabel,
} from './format.js';

describe('number guards', () => {
  it('recognises real numbers only', () => {
    expect(isNumber(0)).toBe(true);
    expect(isNumber(-1.5)).toBe(true);
    expect(isNumber(NaN)).toBe(false);
    expect(isNumber(null)).toBe(false);
    expect(isNumber(undefined)).toBe(false);
    expect(isNumber('12')).toBe(false);
  });
});

describe('formatPrice', () => {
  it('uses the requested precision', () => {
    expect(formatPrice(1234.5, 2)).toBe('1,234.50');
    expect(formatPrice(1.10253, 5)).toBe('1.10253');
    expect(formatPrice(7583, 2)).toBe('7,583.00');
  });

  it('shows a dash when data is missing', () => {
    expect(formatPrice(null)).toBe('—');
    expect(formatPrice(undefined, 2)).toBe('—');
    expect(formatPrice(NaN)).toBe('—');
  });
});

describe('formatPct', () => {
  it('signed with two decimals', () => {
    expect(formatPct(1.234)).toBe('+1.23%');
    expect(formatPct(-0.5)).toBe('-0.50%');
    expect(formatPct(0)).toBe('0.00%');
  });

  it('dash for missing', () => {
    expect(formatPct(null)).toBe('—');
    expect(formatPct(Infinity)).toBe('—');
  });
});

describe('formatVolume', () => {
  it('compacts large numbers', () => {
    expect(formatVolume(1_234_567)).toBe('1.23M');
    expect(formatVolume(2_500_000_000)).toBe('2.50B');
    expect(formatVolume(12_345)).toBe('12.35K');
    expect(formatVolume(12.3)).toBe('12.30');
  });

  it('dash for zero and missing', () => {
    expect(formatVolume(0)).toBe('—');
    expect(formatVolume(undefined)).toBe('—');
  });
});

describe('labels', () => {
  it('maps trend to a trading word', () => {
    expect(trendLabel('up')).toBe('Bullish');
    expect(trendLabel('down')).toBe('Bearish');
    expect(trendLabel(null)).toBe('No signal');
  });

  it('picks the css class for a change', () => {
    expect(changeClass(2.5)).toBe('up');
    expect(changeClass(-0.1)).toBe('down');
    expect(changeClass(0)).toBe('flat');
    expect(changeClass(null)).toBe('flat');
  });
});

describe('time formatting', () => {
  it('renders the bar time in UTC', () => {
    const ms = Date.UTC(2026, 10, 14, 8, 5);
    expect(formatBarTime(ms)).toBe('14 Nov 2026 08:05 UTC');
    expect(formatBarTime(0)).toBe('—');
    expect(formatBarTime(null)).toBe('—');
  });

  it('renders relative freshness', () => {
    const now = Date.UTC(2026, 10, 14, 12, 0, 0);
    expect(formatRelative(now - 5_000, now)).toBe('just now');
    expect(formatRelative(now - 60_000, now)).toBe('1 min ago');
    expect(formatRelative(now - 7 * 60_000, now)).toBe('7 min ago');
    expect(formatRelative(now - 3 * 3_600_000, now)).toBe('3 h ago');
    expect(formatRelative(now - 2 * 86_400_000, now)).toBe('2 d ago');
    expect(formatRelative(null, now)).toBe('—');
  });
});

describe('chart data mapping', () => {
  const points = [
    { time: 100, open: 1, high: 2, low: 0.5, close: 1.5, volume: 10, ema50: null, ema200: null },
    { time: 200, open: 1.5, high: 3, low: 1, close: 1.2, volume: 20, ema50: 1.4, ema200: null },
    { time: 300, open: 1.2, high: 4, low: 1, close: 3.5, volume: 30, ema50: 1.8, ema200: 1.1 },
  ];

  it('maps candles to the chart shape', () => {
    expect(toCandles(points)).toEqual([
      { time: 100, open: 1, high: 2, low: 0.5, close: 1.5 },
      { time: 200, open: 1.5, high: 3, low: 1, close: 1.2 },
      { time: 300, open: 1.2, high: 4, low: 1, close: 3.5 },
    ]);
    expect(toCandles()).toEqual([]);
  });

  it('skips null EMA values instead of plotting zeroes', () => {
    expect(toLine(points, 'ema50')).toEqual([
      { time: 200, value: 1.4 },
      { time: 300, value: 1.8 },
    ]);
    expect(toLine(points, 'ema200')).toEqual([{ time: 300, value: 1.1 }]);
    expect(toLine(points, 'missing')).toEqual([]);
  });

  it('colours volume by candle direction', () => {
    const volume = toVolume(points);
    expect(volume).toHaveLength(3);
    expect(volume[0].color).toContain('22, 199, 132'); // close above open
    expect(volume[1].color).toContain('234, 57, 67'); // close below open
    expect(volume[0].value).toBe(10);
  });

  it('computes the EMA spread in percent', () => {
    expect(emaSpread(110, 100)).toBeCloseTo(10, 6);
    expect(emaSpread(90, 100)).toBeCloseTo(-10, 6);
    expect(emaSpread(null, 100)).toBeNull();
    expect(emaSpread(100, 0)).toBeNull();
  });
});

describe('sortSymbols', () => {
  it('keeps the desk order regardless of API order', () => {
    const rows = [{ symbol: 'ES1' }, { symbol: 'EURUSD' }, { symbol: 'BTCUSDT' }, { symbol: 'SOLUSDT' }];
    expect(sortSymbols(rows).map((row) => row.symbol)).toEqual([
      'SOLUSDT',
      'BTCUSDT',
      'EURUSD',
      'ES1',
    ]);
  });

  it('puts unknown symbols last without mutating the input', () => {
    const rows = [{ symbol: 'XAUUSD' }, { symbol: 'BTCUSDT' }];
    const sorted = sortSymbols(rows, ['BTCUSDT']);
    expect(sorted.map((row) => row.symbol)).toEqual(['BTCUSDT', 'XAUUSD']);
    expect(rows[0].symbol).toBe('XAUUSD');
  });
});
