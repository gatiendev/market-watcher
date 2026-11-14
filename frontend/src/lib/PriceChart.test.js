// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { latestChart, resetCharts } from '../test/chart-mock.js';

vi.mock('lightweight-charts', () => import('../test/chart-mock.js'));

const { default: PriceChart } = await import('./PriceChart.svelte');

const HOUR = 3600;
const DAY = 86_400;
const START = 1_700_000_000;

/** H4 bars: bar `spike` carries the high, bar `dip` carries the low. */
function bars({ count = 8, from = START, step = 4 * HOUR, spike = 2, dip = 5 } = {}) {
  return Array.from({ length: count }, (_, idx) => {
    const base = 10 + idx * 0.1;
    return {
      time: from + idx * step,
      open: base,
      high: idx === spike ? base + 2 : base + 0.2,
      low: idx === dip ? base - 2 : base - 0.2,
      close: base + 0.05,
      ema50: base - 0.05,
      ema200: base - 0.15,
      volume: 100 + idx,
    };
  });
}

const SWINGS = [
  { time: START, price: 12.1, kind: 'high' },
  { time: START, price: 7.6, kind: 'low' },
];

function props(overrides = {}) {
  return {
    points: bars(),
    decimals: 2,
    timeframe: 'H4',
    resetKey: 'SOLUSDT-H4',
    pivots: [],
    showPivots: false,
    ...overrides,
  };
}

beforeEach(() => {
  resetCharts();
});

describe('PriceChart switching', () => {
  it('draws the first instrument and fits the view once', async () => {
    render(PriceChart, props());
    await tick();

    const chart = latestChart();
    expect(chart.candles.data).toHaveLength(8);
    const first = chart.candles.data[0];
    expect(first.time).toBe(START);
    expect(first.open).toBeCloseTo(10);
    expect(first.high).toBeCloseTo(10.2);
    expect(first.low).toBeCloseTo(9.8);
    expect(first.close).toBeCloseTo(10.05);
    expect(chart.fitContent).toHaveBeenCalledTimes(1);
  });

  it('replaces the series when the symbol changes and refits', async () => {
    const view = render(PriceChart, props());
    await tick();

    const other = bars({ count: 5, from: START + 40 * DAY, step: 4 * HOUR });
    await view.rerender(props({ points: other, resetKey: 'BTCUSDT-H4' }));
    await tick();

    const chart = latestChart();
    expect(chart.candles.data).toHaveLength(5);
    expect(chart.candles.data[0].time).toBe(START + 40 * DAY);
    expect(chart.fitContent).toHaveBeenCalledTimes(2);
  });

  it('keeps the zoom on a background refresh of the same symbol', async () => {
    const view = render(PriceChart, props());
    await tick();

    const refreshed = bars({ count: 9 });
    await view.rerender(props({ points: refreshed }));
    await tick();

    const chart = latestChart();
    expect(chart.candles.data).toHaveLength(9);
    expect(chart.fitContent).toHaveBeenCalledTimes(1);
  });

  it('refits when only the timeframe changes', async () => {
    const view = render(PriceChart, props());
    await tick();

    const daily = bars({ count: 4, step: DAY });
    await view.rerender(props({ points: daily, timeframe: 'D1', resetKey: 'SOLUSDT-D1' }));
    await tick();

    expect(latestChart().fitContent).toHaveBeenCalledTimes(2);
  });

  it('clears the chart when the new timeframe has no stored bars', async () => {
    const view = render(PriceChart, props());
    await tick();

    await view.rerender(props({ points: [], timeframe: 'D1', resetKey: 'ES1-D1' }));
    await tick();

    const chart = latestChart();
    expect(chart.candles.data).toEqual([]);
    expect(chart.lines[0].data).toEqual([]);
    expect(chart.histograms[0].data).toEqual([]);
  });
});

describe('PriceChart pivots', () => {
  it('puts arrows on the candle that made each swing', async () => {
    render(PriceChart, props({ pivots: SWINGS, showPivots: true }));
    await tick();

    const markers = latestChart().candles.markers;
    expect(markers).toHaveLength(2);
    expect(markers[0]).toMatchObject({
      time: START + 2 * 4 * HOUR,
      position: 'aboveBar',
      shape: 'arrowDown',
    });
    expect(markers[1]).toMatchObject({
      time: START + 5 * 4 * HOUR,
      position: 'belowBar',
      shape: 'arrowUp',
    });
  });

  it('draws dashed levels with labels on the price axis', async () => {
    render(PriceChart, props({ pivots: SWINGS, showPivots: true }));
    await tick();

    const lines = latestChart().candles.priceLines;
    expect(lines.length).toBeGreaterThan(0);
    for (const line of lines) {
      expect(line.options.axisLabelVisible).toBe(true);
      expect(line.options.lineStyle).toBe(2);
      expect(['R', 'S']).toContain(line.options.title.slice(0, 1));
    }
  });

  it('removes the old levels before drawing another instrument', async () => {
    const view = render(PriceChart, props({ pivots: SWINGS, showPivots: true }));
    await tick();
    const chart = latestChart();
    const firstLines = [...chart.candles.priceLines];
    expect(firstLines.length).toBeGreaterThan(0);

    const otherBars = bars({ from: START + 40 * DAY });
    await view.rerender(
      props({
        points: otherBars,
        resetKey: 'BTCUSDT-H4',
        pivots: [{ time: START + 40 * DAY, price: 12.1, kind: 'high' }],
        showPivots: true,
      }),
    );
    await tick();

    expect(firstLines.every((line) => line.removed)).toBe(true);
    expect(chart.candles.priceLines.length).toBeGreaterThan(0);
    expect(chart.candles.priceLines.every((line) => !firstLines.includes(line))).toBe(true);
  });

  it('drops markers and levels when pivots are switched off', async () => {
    const view = render(PriceChart, props({ pivots: SWINGS, showPivots: true }));
    await tick();
    expect(latestChart().candles.markers).toHaveLength(2);

    await view.rerender(props({ pivots: SWINGS, showPivots: false }));
    await tick();

    const chart = latestChart();
    expect(chart.candles.markers).toEqual([]);
    expect(chart.candles.priceLines).toHaveLength(0);
  });

  it('clears the old markers when a symbol with no pivots is shown', async () => {
    const view = render(PriceChart, props({ pivots: SWINGS, showPivots: true }));
    await tick();

    await view.rerender(
      props({
        points: bars({ from: START + 40 * DAY }),
        resetKey: 'BTCUSDT-H4',
        pivots: [],
        showPivots: true,
      }),
    );
    await tick();

    expect(latestChart().candles.markers).toEqual([]);
    expect(latestChart().candles.priceLines).toHaveLength(0);
  });
});

describe('PriceChart lifecycle', () => {
  it('removes the chart when the component goes away', async () => {
    const view = render(PriceChart, props());
    await tick();
    const chart = latestChart();

    view.unmount();
    expect(chart.removed).toBe(true);
  });
});
