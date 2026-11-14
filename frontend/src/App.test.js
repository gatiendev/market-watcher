// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { latestChart, resetCharts } from './test/chart-mock.js';

vi.mock('lightweight-charts', () => import('./test/chart-mock.js'));

const api = vi.hoisted(() => ({
  fetchMarkets: vi.fn(),
  fetchCandles: vi.fn(),
  fetchPivots: vi.fn(),
  triggerRefresh: vi.fn(),
  ApiError: class ApiError extends Error {},
}));
vi.mock('./lib/api.js', () => api);

const { default: App } = await import('./App.svelte');

// This jsdom environment has no `window` and no storage; the app reads the bare
// `localStorage` name, so give it a real in-memory store to exercise the saved
// pivot settings.
const localStore = (() => {
  const store = new Map();
  return {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: (key) => store.delete(key),
    clear: () => store.clear(),
    key: (index) => [...store.keys()][index] ?? null,
    get length() {
      return store.size;
    },
  };
})();

Object.defineProperty(globalThis, 'localStorage', {
  value: localStore,
  configurable: true,
  writable: true,
});

const HOUR = 3600;
const DAY = 86_400;
const T0 = 1_700_000_000;

/** Candles as the API returns them, one H4 bar every 4 hours. */
function h4(count, from = T0, base = 10, step = 4 * HOUR) {
  return Array.from({ length: count }, (_, idx) => {
    const open = base + idx * 0.1;
    return {
      time: from + idx * step,
      open,
      high: open + 0.3,
      low: open - 0.3,
      close: open + 0.05,
      ema50: open - 0.05,
      ema200: open - 0.2,
      volume: 100 + idx,
    };
  });
}

function snapshot(lastClose) {
  return {
    last_close: lastClose,
    change_pct: 1.2,
    ema50: lastClose - 1,
    ema200: lastClose - 2,
    trend: 'up',
    trend_since: T0 * 1000,
    trend_bars: 12,
    bars: 120,
    last_bar_time: T0 * 1000,
    status: 'ok',
  };
}

function markets() {
  const row = (symbol, label, lastClose) => ({
    symbol,
    label,
    decimals: 2,
    timeframes: { H4: snapshot(lastClose), D1: snapshot(lastClose) },
  });
  return {
    symbols: [
      row('SOLUSDT', 'SOL / USDT', 75.5),
      row('BTCUSDT', 'BTC / USDT', 66000),
      row('EURUSD', 'EUR / USD', 1.1),
    ],
  };
}

/** A fetch mock whose calls are settled by the test, abort-aware like fetch. */
function pendingFetches() {
  const calls = [];
  const impl = vi.fn((...args) => {
    const signal = args.find((arg) => arg && typeof arg === 'object' && typeof arg.aborted === 'boolean');
    const call = { args, signal, aborted: false };
    const promise = new Promise((resolve, reject) => {
      call.resolve = resolve;
      call.reject = reject;
    });
    signal?.addEventListener('abort', () => {
      call.aborted = true;
      const err = new Error('The operation was aborted.');
      err.name = 'AbortError';
      call.reject(err);
    });
    calls.push(call);
    return promise;
  });
  return { calls, impl };
}

/** Let every queued promise callback and Svelte update settle. */
async function flush() {
  await tick();
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await tick();
}

let candles;
let pivots;

function storage() {
  return globalThis.localStorage;
}

function rowButton(container, symbol) {
  return container.querySelector(`[aria-label="Show ${symbol} chart"]`);
}

function timeframeButton(container, label) {
  return [...container.querySelectorAll('.segmented button')].find(
    (button) => button.textContent.trim() === label,
  );
}

async function boot({ pivotsOn = false } = {}) {
  api.fetchMarkets.mockResolvedValue(markets());
  candles = pendingFetches();
  pivots = pendingFetches();
  api.fetchCandles.mockImplementation(candles.impl);
  api.fetchPivots.mockImplementation(pivots.impl);

  if (pivotsOn) {
    storage()?.setItem('market-watcher:pivots', JSON.stringify({ on: true, lookback: 15 }));
  }

  const view = render(App);
  await flush();
  candles.calls[0].resolve({ symbol: 'SOLUSDT', timeframe: 'H4', points: h4(6) });
  await flush();
  // With pivots on, the boot sequence fetches them only after the candles land.
  await flush();
  return view;
}

beforeEach(() => {
  resetCharts();
  storage()?.clear();
  api.fetchMarkets.mockReset();
  api.fetchCandles.mockReset();
  api.fetchPivots.mockReset();
  api.triggerRefresh.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('App chart switching', () => {
  it('loads the default symbol and timeframe on boot', async () => {
    await boot();
    expect(candles.calls).toHaveLength(1);
    expect(candles.calls[0].args[0]).toBe('SOLUSDT');
    expect(candles.calls[0].args[1]).toBe('H4');
    expect(latestChart().candles.data).toHaveLength(6);
  });

  it('switches instrument on a watchlist click', async () => {
    const view = await boot();
    rowButton(view.container, 'BTCUSDT').click();
    await flush();

    expect(candles.calls).toHaveLength(2);
    expect(candles.calls[1].args[0]).toBe('BTCUSDT');
    expect(candles.calls[1].args[1]).toBe('H4');
    // The old chart stays on screen until the new series arrives, so there is no
    // empty flash while switching.
    expect(latestChart().candles.data).toHaveLength(6);

    candles.calls[1].resolve({ symbol: 'BTCUSDT', timeframe: 'H4', points: h4(9, T0, 60000) });
    await flush();
    expect(latestChart().candles.data).toHaveLength(9);
    expect(view.container.querySelector('.instrument .ticker').textContent).toBe('BTCUSDT');
  });

  it('ignores a slow response for a symbol the user already left', async () => {
    const view = await boot();

    rowButton(view.container, 'BTCUSDT').click();
    await flush();
    rowButton(view.container, 'EURUSD').click();
    await flush();
    expect(candles.calls[2].args[0]).toBe('EURUSD');
    expect(candles.calls[1].aborted).toBe(true);

    // BTC lands late; it must not paint over EUR.
    candles.calls[1].resolve({ symbol: 'BTCUSDT', timeframe: 'H4', points: h4(9, T0, 60000) });
    await flush();
    expect(latestChart().candles.data).toHaveLength(6);

    candles.calls[2].resolve({ symbol: 'EURUSD', timeframe: 'H4', points: h4(4, T0, 1.1) });
    await flush();
    expect(latestChart().candles.data).toHaveLength(4);
    expect(latestChart().candles.data[0].close).toBeCloseTo(1.15);
  });

  it('switches timeframe without refetching pivots', async () => {
    const view = await boot({ pivotsOn: true });
    pivots.calls[0].resolve({ symbol: 'SOLUSDT', pivots: [] });
    await flush();

    timeframeButton(view.container, 'D1').click();
    await flush();

    expect(candles.calls).toHaveLength(2);
    expect(candles.calls[1].args[0]).toBe('SOLUSDT');
    expect(candles.calls[1].args[1]).toBe('D1');
    expect(api.fetchPivots).toHaveBeenCalledTimes(1);

    candles.calls[1].resolve({ symbol: 'SOLUSDT', timeframe: 'D1', points: h4(3, T0, 10, DAY) });
    await flush();
    expect(latestChart().fitContent).toHaveBeenCalledTimes(2);
  });

  it('keeps the chosen timeframe when another symbol is picked', async () => {
    const view = await boot();

    timeframeButton(view.container, 'D1').click();
    await flush();
    candles.calls[1].resolve({ symbol: 'SOLUSDT', timeframe: 'D1', points: h4(3, T0, 10, DAY) });
    await flush();

    rowButton(view.container, 'BTCUSDT').click();
    await flush();
    expect(candles.calls[2].args[1]).toBe('D1');
  });

  it('surfaces a failed switch without losing the chart', async () => {
    const view = await boot();

    rowButton(view.container, 'BTCUSDT').click();
    await flush();
    candles.calls[1].reject(new Error('backend down'));
    await flush();

    expect(view.container.querySelector('.error-bar').textContent).toContain('backend down');
    expect(latestChart().candles.data).toHaveLength(6);
  });
});

describe('App pivots while switching', () => {
  const swing = { time: T0 + 2 * 4 * HOUR, price: 10.6, kind: 'high' };

  it('fetches swings when the toggle is turned on and keeps them on a timeframe change', async () => {
    const view = await boot();
    view.container.querySelector('.pivot-toggle').click();
    await flush();

    expect(api.fetchPivots).toHaveBeenCalledTimes(1);
    expect(pivots.calls[0].args[0]).toBe('SOLUSDT');
    expect(pivots.calls[0].args[1]).toBe(15);

    pivots.calls[0].resolve({ symbol: 'SOLUSDT', pivots: [swing] });
    await flush();
    expect(latestChart().candles.markers).toHaveLength(1);
    expect(view.container.querySelectorAll('.level-chip').length).toBeGreaterThan(0);
  });

  it('never draws one symbol swings on another symbol chart', async () => {
    const view = await boot();
    view.container.querySelector('.pivot-toggle').click();
    await flush();
    pivots.calls[0].resolve({ symbol: 'SOLUSDT', pivots: [swing] });
    await flush();
    expect(latestChart().candles.markers).toHaveLength(1);

    rowButton(view.container, 'BTCUSDT').click();
    await flush();

    expect(api.fetchPivots).toHaveBeenCalledTimes(2);
    expect(pivots.calls[1].args[0]).toBe('BTCUSDT');
    // SOL swings are gone the moment the instruments change.
    expect(latestChart().candles.markers).toEqual([]);
    expect(latestChart().candles.priceLines).toHaveLength(0);
    expect(view.container.querySelectorAll('.level-chip')).toHaveLength(0);

    candles.calls[1].resolve({ symbol: 'BTCUSDT', timeframe: 'H4', points: h4(9, T0, 60000) });
    pivots.calls[1].resolve({ symbol: 'BTCUSDT', pivots: [] });
    await flush();
    expect(latestChart().candles.markers).toEqual([]);
  });

  it('refetches at the new strength when the lookback changes', async () => {
    const view = await boot();
    view.container.querySelector('.pivot-toggle').click();
    await flush();
    pivots.calls[0].resolve({ symbol: 'SOLUSDT', pivots: [swing] });
    await flush();

    view.container.querySelectorAll('.stepper button')[0].click();
    await flush();

    expect(api.fetchPivots).toHaveBeenCalledTimes(2);
    expect(pivots.calls[1].args[0]).toBe('SOLUSDT');
    expect(pivots.calls[1].args[1]).toBe(10);
    expect(view.container.querySelector('.stepper .steps').textContent).toContain('10');
  });

  it('hides markers and levels when the toggle goes off', async () => {
    const view = await boot();
    const toggle = view.container.querySelector('.pivot-toggle');
    toggle.click();
    await flush();
    pivots.calls[0].resolve({ symbol: 'SOLUSDT', pivots: [swing] });
    await flush();
    expect(latestChart().candles.markers).toHaveLength(1);

    toggle.click();
    await flush();
    expect(latestChart().candles.markers).toEqual([]);
    expect(latestChart().candles.priceLines).toHaveLength(0);
    expect(view.container.querySelectorAll('.level-chip')).toHaveLength(0);
  });
});
