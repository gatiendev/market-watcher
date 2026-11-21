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
      crossing({
        symbol: 'BTCUSDT',
        label: 'BTC / USDT',
        crossed_at: T0 + 86_400,
        direction: 'down',
        open: true,
      }),
      crossing(),
      crossing({
        timeframe: 'D1',
        crossed_at: T0 + 86_400,
        peak_pct: 21.5,
        days_to_peak: 14,
        trough_pct: -4.2,
      }),
    ],
    summaries: [
      summary(),
      summary({
        symbol: 'SOLUSDT',
        crossings: 1,
        ups: 1,
        downs: 0,
        open_runs: 0,
        avg_peak_pct: 8,
        avg_exit_pct: 3,
      }),
      summary({
        symbol: 'BTCUSDT',
        crossings: 1,
        ups: 0,
        downs: 1,
        open_runs: 1,
        avg_peak_pct: 5,
        avg_exit_pct: null,
      }),
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

function timeframeButton(container, label) {
  return [...container.querySelectorAll('.segmented button')].find(
    (button) => button.textContent.trim() === label,
  );
}

function tableRows(container) {
  return container.querySelectorAll('table.crossings tbody tr');
}

/** Panel headings, minus the "Right now" block that sits above them. */
function crossingHeadings(container) {
  return [...container.querySelectorAll('.panel-head h2')]
    .map((node) => node.textContent.trim())
    .filter((text) => text !== 'Right now');
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('crossings page', () => {
  it('opens on H4 with the performance that followed each cross', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    expect(crossingHeadings(view.container)).toEqual(['H4 crossings']);

    const rows = tableRows(view.container);
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('BTCUSDT');
    expect(rows[0].textContent).toContain('open');
    expect(rows[1].textContent).toContain('+8.00%');
    expect(rows[1].textContent).toContain('2.0d');
    expect(view.container.textContent).toContain('trough → peak');
  });

  it('switches the table to D1 when the toggle is used', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    timeframeButton(view.container, 'D1').click();
    await flush();

    expect(crossingHeadings(view.container)).toEqual(['D1 crossings']);
    const rows = tableRows(view.container);
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('+21.50%');
    expect(rows[0].textContent).toContain('14d');

    expect(timeframeButton(view.container, 'D1').getAttribute('aria-pressed')).toBe('true');
    expect(timeframeButton(view.container, 'H4').getAttribute('aria-pressed')).toBe('false');
  });

  it('summarises the crossings above the table', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    const summaryLine = view.container.querySelector('.summary');
    expect(summaryLine.textContent).toContain('2 crossings');
    expect(summaryLine.textContent).toContain('1 up');
    expect(summaryLine.textContent).toContain('1 down');
    expect(summaryLine.textContent).toContain('avg peak +6.50%');
    expect(summaryLine.textContent).toContain('median 1.5d to peak');
    expect(summaryLine.textContent).toContain('1 still open');
  });

  it('filters to one asset and uses that asset’s numbers', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    chip(view.container, 'BTCUSDT').click();
    await flush();

    const rows = tableRows(view.container);
    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('BTCUSDT');
    expect([...rows].some((row) => row.textContent.includes('SOLUSDT'))).toBe(false);
    expect(view.container.querySelector('.summary').textContent).toContain('avg peak +5.00%');
  });

  it('shows the trend in force on both timeframes, whatever the toggle says', async () => {
    const rows = [
      crossing({
        symbol: 'SOLUSDT',
        timeframe: 'H4',
        direction: 'down',
        open: true,
        run_days: 38,
        exit_pct: 33.5,
        peak_pct: 37.6,
      }),
      crossing({
        symbol: 'SOLUSDT',
        timeframe: 'D1',
        direction: 'up',
        open: true,
        run_days: 5,
        exit_pct: 3.1,
        peak_pct: 4.2,
      }),
      crossing({
        symbol: 'BTCUSDT',
        label: 'BTC / USDT',
        timeframe: 'H4',
        direction: 'up',
        open: true,
        run_days: 12,
        exit_pct: -2.5,
        peak_pct: 6,
      }),
    ];
    api.fetchCrossings.mockResolvedValue({
      generated_at: T0 * 1000,
      rows,
      summaries: [summary({ crossings: 3, ups: 2, downs: 1, open_runs: 3 })],
    });
    const view = render(Crossings);
    await flush();

    // The toggle is on H4, yet the block still answers for D1.
    expect(crossingHeadings(view.container)).toEqual(['H4 crossings']);

    const now = view.container.querySelector('.now');
    const assets = now.querySelectorAll('.now-asset');
    expect(assets).toHaveLength(2);

    const [solH4, solD1] = assets[0].querySelectorAll('.now-row');
    expect(solH4.textContent).toContain('H4');
    expect(solH4.textContent).toContain('down');
    expect(solH4.textContent).toContain('38d');
    expect(solH4.textContent).toContain('+33.50%');
    expect(solH4.textContent).toContain('peak +37.60%');
    expect(solD1.textContent).toContain('D1');
    expect(solD1.textContent).toContain('up');
    expect(solD1.textContent).toContain('5.0d');

    // BTC has no stored D1 crossing, and the page says so instead of going blank.
    expect(assets[1].textContent).toContain('BTCUSDT');
    expect(assets[1].textContent).toContain('no crossing stored yet');
  });

  it('renders the same crossings as cards for a phone', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    const cards = view.container.querySelectorAll('.cross-card');
    expect(cards).toHaveLength(2);
    expect(cards[0].textContent).toContain('BTCUSDT');
    expect(cards[0].textContent).toContain('open');
    expect(cards[1].textContent).toContain('+8.00%');
    expect(cards[1].textContent).toContain('2.0d');
    expect(cards[1].textContent).toContain('Result');
    expect(cards[1].textContent).toContain('Trough → peak');
  });

  it('pages the phone cards instead of dumping every crossing at once', async () => {
    const rows = Array.from({ length: 60 }, (_, index) =>
      crossing({ crossed_at: T0 - index * 14_400 }),
    );
    api.fetchCrossings.mockResolvedValue({
      generated_at: T0 * 1000,
      rows,
      summaries: [summary({ crossings: 60, ups: 60, downs: 0 })],
    });
    const view = render(Crossings);
    await flush();

    expect(view.container.querySelectorAll('.cross-card')).toHaveLength(25);
    expect(view.container.querySelector('.more').textContent).toContain('35 left');

    view.container.querySelector('.more').click();
    await flush();
    expect(view.container.querySelectorAll('.cross-card')).toHaveLength(50);
    expect(view.container.querySelector('.more').textContent).toContain('10 left');

    // The desktop table is never paged.
    expect(view.container.querySelectorAll('table.crossings tbody tr')).toHaveLength(60);

    // Leaving the timeframe and coming back starts at the first page again.
    timeframeButton(view.container, 'D1').click();
    await flush();
    timeframeButton(view.container, 'H4').click();
    await flush();
    expect(view.container.querySelectorAll('.cross-card')).toHaveLength(25);
  });

  it('says so when the chosen asset has no cross on that timeframe', async () => {
    api.fetchCrossings.mockResolvedValue(payload());
    const view = render(Crossings);
    await flush();

    chip(view.container, 'BTCUSDT').click();
    timeframeButton(view.container, 'D1').click();
    await flush();

    expect(tableRows(view.container)).toHaveLength(0);
    expect(view.container.textContent).toContain('No D1 crossing stored yet for BTCUSDT');
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
