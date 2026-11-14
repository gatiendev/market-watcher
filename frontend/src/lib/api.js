// Thin wrapper over the backend API. All paths are relative so the same build
// works behind nginx, the Vite dev proxy and a local `vite preview`.

const BASE = (import.meta.env?.VITE_API_BASE ?? '').replace(/\/$/, '');

export class ApiError extends Error {
  constructor(message, status = 0) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

export function marketsPath() {
  return '/api/markets';
}

export function candlesPath(symbol, timeframe, limit = 400) {
  const params = new URLSearchParams({
    symbol: String(symbol || '').toUpperCase(),
    timeframe: String(timeframe || 'H4').toUpperCase(),
    limit: String(limit),
  });
  return `/api/candles?${params.toString()}`;
}

export function refreshPath(symbol) {
  return symbol ? `/api/refresh?symbol=${encodeURIComponent(String(symbol).toUpperCase())}` : '/api/refresh';
}

async function request(path, { method = 'GET', signal } = {}) {
  let response;
  try {
    response = await fetch(`${BASE}${path}`, {
      method,
      signal,
      headers: { accept: 'application/json' },
    });
  } catch (error) {
    if (error?.name === 'AbortError') throw error;
    throw new ApiError('Cannot reach the server');
  }

  const text = await response.text();
  let payload = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null;
    }
  }

  if (!response.ok) {
    const message = payload?.error || `Request failed (${response.status})`;
    throw new ApiError(message, response.status);
  }
  if (payload === null) throw new ApiError('The server returned an empty response');
  return payload;
}

export function fetchMarkets(signal) {
  return request(marketsPath(), { signal });
}

export function fetchCandles(symbol, timeframe, limit = 400, signal) {
  return request(candlesPath(symbol, timeframe, limit), { signal });
}

export function triggerRefresh(symbol, signal) {
  return request(refreshPath(symbol), { method: 'POST', signal });
}

export function fetchHealth(signal) {
  return request('/api/health', { signal });
}
