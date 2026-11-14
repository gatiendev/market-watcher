import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  candlesPath,
  fetchCandles,
  fetchMarkets,
  marketsPath,
  refreshPath,
  triggerRefresh,
} from './api.js';

function mockFetch({ ok = true, status = 200, body = '{}' } = {}) {
  const fn = vi.fn().mockResolvedValue({
    ok,
    status,
    text: async () => (typeof body === 'string' ? body : JSON.stringify(body)),
  });
  vi.stubGlobal('fetch', fn);
  return fn;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('paths', () => {
  it('builds the candle query', () => {
    const path = candlesPath('solusdt', 'h4', 200);
    expect(path).toBe('/api/candles?symbol=SOLUSDT&timeframe=H4&limit=200');
  });

  it('builds refresh paths with and without a symbol', () => {
    expect(refreshPath()).toBe('/api/refresh');
    expect(refreshPath('es1')).toBe('/api/refresh?symbol=ES1');
    expect(marketsPath()).toBe('/api/markets');
  });

  it('falls back to sane defaults', () => {
    expect(candlesPath('', '', undefined)).toContain('timeframe=H4');
    expect(candlesPath(null, null, 10)).toContain('limit=10');
  });
});

describe('fetchMarkets', () => {
  it('returns parsed json', async () => {
    const fn = mockFetch({ body: { symbols: [{ symbol: 'SOLUSDT' }] } });
    const data = await fetchMarkets();
    expect(data.symbols[0].symbol).toBe('SOLUSDT');
    expect(fn).toHaveBeenCalledWith('/api/markets', expect.objectContaining({ method: 'GET' }));
  });

  it('surfaces the backend error message', async () => {
    mockFetch({ ok: false, status: 404, body: { error: 'unknown symbol' } });
    await expect(fetchCandles('DOGE', 'H4')).rejects.toThrow('unknown symbol');
  });

  it('flags unreachable servers', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('fetch failed')));
    await expect(fetchMarkets()).rejects.toThrow('Cannot reach the server');
  });

  it('rejects empty bodies', async () => {
    mockFetch({ body: '' });
    await expect(fetchMarkets()).rejects.toBeInstanceOf(ApiError);
  });

  it('post for refresh', async () => {
    const fn = mockFetch({ body: { reports: [] } });
    await triggerRefresh('BTCUSDT');
    expect(fn).toHaveBeenCalledWith('/api/refresh?symbol=BTCUSDT', expect.objectContaining({ method: 'POST' }));
  });
});
