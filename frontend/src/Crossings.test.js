// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';

const api = vi.hoisted(() => ({
  fetchCrossings: vi.fn(),
  ApiError: class ApiError extends Error {},
}));
vi.mock('./lib/api.js', () => api);

const { default: Crossings } = await import('./Crossings.svelte');

const T0 = 1_700_000_000;

/** Let every queued promise callback and Svelte update settle. */
async function flush() {
  await tick();
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await tick();
}

function crossing(overrides = {}) {
  return {
    symbol: 'SOLUSDT',
    label: 'SOL / USDT',
    decimals: 2,
    timeframe: 'H4',
    direction: 'up',
    crossed_at: T0,
    price: 100,
    ema50: 101,
    ema200: 100,
    peak_pct: 8,
    peak_bars: 12,
    days_to_peak: 2,
    trough_pct: -1,
    trough_to_peak_pct: 9.09,
    run_bars: 30,
    run_days: 5,
    exit_pct: 3,
    open: false,
    ...overrides,
  };
}

function summary(overrides = {}) {
  return {
    timeframe: 'H4',
    symbol: null,
    crossings: 2,
    ups: 1,
    downs: 1,
    open_runs: 1,
    avg_peak_pct: 6.5,
    median_days_to_peak: 1.5,
    avg_trough_to_peak_pct: 7.4,
    avg_exit_pct: 3,
    ...overrides,
  };
}

function payload() {
  return {
    generated_at: T0 * 1000,
    rows: [
      crossing({ symbol: 'BTCUSDT', label: 'BTC / USDT', crossed_at: T0 + 86_400, direction: 'down', open: true }),
      crossing(),
      crossing({ timeframe: 'D1', crossed_at: T0 + 86_400, peak_pct: 21.5, days_to_peak: 14, trough_pct: -4.2 }),
    ],
    summaries: [
      summary(),
      summary({ symbol: 'SOLUSDT', crossings: 1, ups: 1, downs: 0, open_runs: 0, avg_peak_pct: 8, avg_exit_pct: 3 }),
      summary({ symbol: 'BTCUSDT', crossings: 1, ups: 0, downs: 1, open_runs: 1, avg_peak_pct: 5, avg_exit_pct: null }),
      summary({
        timeframe: 'D1',
        crossings: 1,
        ups: 1,
        downs: 0,
        open_runs: 0,
        avg_peak_pct: 21.5,
        median_days_to_peak: 14,
        avg_trough_to_peak_pct: 7.4,
        avg_exit_pct: 3,
      }),
    ],
  };
}

function chip(container, label) {
  return [...container.querySelectorAll('.chip')].find(
    (button) => button.textContent.trim() === label,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('crossings page', () => {
  it('shows both timeframes with the performance that followed each cross', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    const headings = [...view.container.querySelectorAll('.panel-head h2')].map((node) =>
      node.textContent.trim(),
    );
    expect(headings).toEqual(['H4 crossings', 'D1 crossings']);

    const [h4Table] = view.container.querySelectorAll('table.crossings');
    const h4Rows = h4Table.querySelectorAll('tbody tr');
    expect(h4Rows).toHaveLength(2);
    expect(h4Rows[0].textContent).toContain('BTCUSDT');
    expect(h4Rows[0].textContent).toContain('open');
    expect(h4Rows[1].textContent).toContain('+8.00%');
    expect(h4Rows[1].textContent).toContain('2.0d');

    const text = view.container.textContent;
    expect(text).toContain('trough → peak');
    expect(text).toContain('+21.50%');
    expect(text).toContain('14d');
  });

  it('summarises the crossings above each table', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    const [h4] = view.container.querySelectorAll('.summary');
    expect(h4.textContent).toContain('2 crossings');
    expect(h4.textContent).toContain('1 up');
    expect(h4.textContent).toContain('1 down');
    expect(h4.textContent).toContain('avg peak +6.50%');
    expect(h4.textContent).toContain('median 1.5d to peak');
    expect(h4.textContent).toContain('1 still open');
  });

  it('filters to one asset and uses that asset’s numbers', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    chip(view.container, 'BTCUSDT').click();
    await flush();

    const rows = view.container.querySelectorAll('table.crossings tbody tr');
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('BTCUSDT');
    expect([...rows].some((row) => row.textContent.includes('SOLUSDT'))).toBe(false);
    expect(view.container.querySelector('.summary').textContent).toContain('avg peak +5.00%');
  });

  it('says so when there is no crossing to show yet', async () => {
    api.fetchCrossings.mockResolvedValue({ generated_at: T0 * 1000, rows: [], summaries: [] });
    const view = render(Crossings);
    await flush();

    expect(view.container.textContent).toContain('No crossings yet');
    expect(view.container.textContent).toContain('200 bars');
    expect(view.container.querySelectorAll('table.crossings')).toHaveLength(0);
    expect(view.container.textContent).not.toContain('Loading crossings');
  });

  it('shows a plain sentence when the data cannot be loaded', async () => {
    api.fetchCrossings.mockRejectedValue(new Error('Cannot reach the server'));
    const view = render(Crossings);
    await flush();

    expect(view.container.querySelector('.error-bar').textContent).toContain(
      'Cannot reach the server',
    );
  });

  it('says it is loading before the answer arrives', async () => {
    api.fetchCrossings.mockReturnValue(new Promise(() => {}));
    const view = render(Crossings);
    await flush();

    expect(view.container.textContent).toContain('Loading crossings');
    expect(view.container.querySelector('.error-bar')).toBeNull();
  });
});
