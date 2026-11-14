// Stand-in for lightweight-charts in tests: the real library needs a canvas, the
// fake records what the chart component asked it to do so tests can assert on the
// switching behaviour (data replaced, markers cleared, price lines removed).
import { vi } from 'vitest';

export const ColorType = { Solid: 'solid' };
export const CrosshairMode = { Normal: 0, Magnet: 1 };
export const LineStyle = { Solid: 0, Dotted: 1, Dashed: 2, LargeDashed: 3, SparseDotted: 4 };

class FakeSeries {
  constructor(kind) {
    this.kind = kind;
    this.options = {};
    this.data = [];
    this.markers = null;
    this.priceLines = [];
    this.setData = vi.fn((data) => {
      this.data = data;
    });
    this.applyOptions = vi.fn((options) => {
      this.options = { ...this.options, ...options };
    });
    this.setMarkers = vi.fn((markers) => {
      this.markers = markers;
    });
    this.createPriceLine = vi.fn((options) => {
      const line = { options, removed: false };
      this.priceLines.push(line);
      return line;
    });
    this.removePriceLine = vi.fn((line) => {
      line.removed = true;
      this.priceLines = this.priceLines.filter((existing) => existing !== line);
    });
  }
}

class FakeChart {
  constructor(container) {
    this.container = container;
    this.candles = new FakeSeries('candles');
    this.lines = [];
    this.histograms = [];
    this.fitContent = vi.fn();
    this.removed = false;
  }

  addCandlestickSeries(options) {
    this.candles.options = options;
    return this.candles;
  }

  addLineSeries(options) {
    const series = new FakeSeries('line');
    series.options = options;
    this.lines.push(series);
    return series;
  }

  addHistogramSeries(options) {
    const series = new FakeSeries('histogram');
    series.options = options;
    this.histograms.push(series);
    return series;
  }

  priceScale() {
    return { applyOptions: vi.fn() };
  }

  timeScale() {
    return { fitContent: this.fitContent, applyOptions: vi.fn() };
  }

  remove() {
    this.removed = true;
  }
}

/** Every chart created since the last reset, oldest first. */
export const charts = [];

export function createChart(container) {
  const chart = new FakeChart(container);
  charts.push(chart);
  return chart;
}

export function resetCharts() {
  charts.length = 0;
}

/** The chart the component under test is drawing on. */
export function latestChart() {
  return charts[charts.length - 1] ?? null;
}
