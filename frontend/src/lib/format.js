// Pure display helpers. Kept free of DOM/Svelte so they can be unit tested.

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** True for real, finite numbers (rejects null/undefined/NaN). */
export function isNumber(value) {
  return typeof value === 'number' && Number.isFinite(value);
}

/** 1234.5 -> "1,234.50"; missing values -> "—". */
export function formatPrice(value, decimals = 2) {
  if (!isNumber(value)) return '—';
  return value.toLocaleString('en-US', {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
}

/** 1.2345 -> "+1.23%"; missing values -> "—". */
export function formatPct(value, digits = 2) {
  if (!isNumber(value)) return '—';
  const sign = value > 0 ? '+' : value < 0 ? '-' : '';
  return `${sign}${Math.abs(value).toFixed(digits)}%`;
}

/** Compact number for volume: 1_234_567 -> "1.23M". */
export function formatVolume(value) {
  if (!isNumber(value) || value === 0) return '—';
  const abs = Math.abs(value);
  if (abs >= 1e9) return `${(value / 1e9).toFixed(2)}B`;
  if (abs >= 1e6) return `${(value / 1e6).toFixed(2)}M`;
  if (abs >= 1e3) return `${(value / 1e3).toFixed(2)}K`;
  return value.toFixed(2);
}

/** "up"/"down" -> readable label. */
export function trendLabel(trend) {
  if (trend === 'up') return 'Bullish';
  if (trend === 'down') return 'Bearish';
  return 'No signal';
}

/** Compact age: "45m", "7h", "12d", "6w", "8mo", "3y". */
export function formatSpan(ms) {
  if (!isNumber(ms) || ms < 0) return '';
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.round(hours / 24);
  if (days < 21) return `${days}d`;
  if (days < 63) return `${Math.round(days / 7)}w`;
  if (days < 730) return `${Math.round(days / 30)}mo`;
  return `${Math.round(days / 365)}y`;
}

/** A duration in days: "0.3d", "3.5d", "12d". */
export function formatDays(value) {
  if (!isNumber(value)) return '—';
  return `${value < 10 ? value.toFixed(1) : Math.round(value)}d`;
}

/** Chart arrow for the EMA 50 vs EMA 200 relationship. */
export function trendGlyph(trend) {
  if (trend === 'up') return '▲';
  if (trend === 'down') return '▼';
  return '·';
}

/** CSS class for a signed value. */
export function changeClass(value) {
  if (!isNumber(value) || value === 0) return 'flat';
  return value > 0 ? 'up' : 'down';
}

/** Calendar day in UTC, e.g. "14 Nov 2026". */
export function formatDate(ms) {
  if (!isNumber(ms) || ms <= 0) return '—';
  const date = new Date(ms);
  return `${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]} ${date.getUTCFullYear()}`;
}

/** Candle time in UTC, e.g. "14 Nov 2026 08:00 UTC". */
export function formatBarTime(ms) {
  if (!isNumber(ms) || ms <= 0) return '—';
  const date = new Date(ms);
  const pad = (n) => String(n).padStart(2, '0');
  return `${formatDate(ms)} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())} UTC`;
}

/** "just now", "7 min ago", "3 h ago", "2 d ago". */
export function formatRelative(ms, now = Date.now()) {
  if (!isNumber(ms) || ms <= 0) return '—';
  const diff = Math.max(0, now - ms);
  if (diff < 45_000) return 'just now';
  if (diff < 90_000) return '1 min ago';
  if (diff < 3_600_000) return `${Math.round(diff / 60_000)} min ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} h ago`;
  return `${Math.floor(diff / 86_400_000)} d ago`;
}

/** API points -> candlestick series data. */
export function toCandles(points = []) {
  return points.map((point) => ({
    time: point.time,
    open: point.open,
    high: point.high,
    low: point.low,
    close: point.close,
  }));
}

/** API points -> one EMA line, skipping the warm-up gap at the start. */
export function toLine(points = [], key) {
  const line = [];
  for (const point of points) {
    if (isNumber(point[key])) line.push({ time: point.time, value: point[key] });
  }
  return line;
}

/** Volume histogram in the candle colours. */
export function toVolume(points = []) {
  return points.map((point) => ({
    time: point.time,
    value: isNumber(point.volume) ? point.volume : 0,
    color: point.close >= point.open ? 'rgba(22, 199, 132, 0.45)' : 'rgba(234, 57, 67, 0.45)',
  }));
}

/** Spread between the two EMAs, in percent of price. */
export function emaSpread(ema50, ema200) {
  if (!isNumber(ema50) || !isNumber(ema200) || ema200 === 0) return null;
  return ((ema50 - ema200) / ema200) * 100;
}

/** Order watchlist rows the way the desk wants them. */
export function sortSymbols(symbols = [], order = ['SOLUSDT', 'BTCUSDT', 'EURUSD', 'ES1']) {
  const rank = new Map(order.map((symbol, index) => [symbol, index]));
  return [...symbols].sort(
    (a, b) => (rank.get(a.symbol) ?? 99) - (rank.get(b.symbol) ?? 99),
  );
}
