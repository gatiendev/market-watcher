// Daily pivot levels, prepared for the chart.
//
// The backend measures pivots on daily bars; the chart may be showing H4 bars, so
// pivot times have to be snapped onto bar times that actually exist in the
// series (the charting library silently drops markers on unknown times). Support
// and resistance levels also need thinning: a 4-year daily series holds ~90
// swings, and drawing 90 lines hides the price.

const DEFAULT_MAX_LEVELS = 6;
// Two pivots closer than this are the same level to a trader's eye.
const DEFAULT_TOLERANCE = 0.004;
// A level sitting on top of the last price is not support or resistance, it is
// the price; drawing it just fights the price line.
const DEFAULT_MIN_DISTANCE = 0.003;

function isFiniteNumber(value) {
  return typeof value === 'number' && Number.isFinite(value);
}

/** Greatest value in the ascending array `times` that is <= target, or null. */
export function snapDown(times, target) {
  const index = snapIndex(times, target);
  return index === -1 ? null : times[index];
}

/** Index of the greatest value in `times` that is <= target, or -1. */
function snapIndex(times, target) {
  if (!Array.isArray(times) || times.length === 0 || !isFiniteNumber(target)) return -1;
  if (target < times[0]) return -1;
  let low = 0;
  let high = times.length - 1;
  let best = -1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (times[mid] <= target) {
      best = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  return best;
}

// How far past a daily pivot's timestamp to look for the bar that holds the
// extreme: a day of H4 bars plus slack for exchange sessions that do not open at
// midnight UTC.
const EXTREME_WINDOW_MS = 32 * 3600;

/**
 * The bar carrying the pivot's extreme, so the arrow sits on the actual spike
 * candle (its high or low), not on whatever bar happens to start the day.
 */
function extremeBar(bars, times, from, pivot, kind, fallbackTime) {
  const start = Math.max(0, snapIndex(times, from));
  const limit = pivot.time + EXTREME_WINDOW_MS;
  let best = null;

  for (let index = start; index < bars.length; index += 1) {
    const bar = bars[index];
    if (!bar || !isFiniteNumber(bar.time)) continue;
    if (bar.time > limit) break;
    const extreme = kind === 'high' ? bar.high : bar.low;
    if (!isFiniteNumber(extreme)) continue;
    const gap = Math.abs(extreme - pivot.price);
    const distance = Math.abs(bar.time - pivot.time);
    if (
      best === null ||
      gap < best.gap - 1e-9 ||
      (Math.abs(gap - best.gap) <= 1e-9 && distance < best.distance)
    ) {
      best = { time: bar.time, gap, distance };
    }
  }

  return best ? best.time : fallbackTime;
}

/**
 * Pivots mapped onto the bars that actually carry them, one marker per bar,
 * oldest first. Dropped pivots are the ones older than the first stored bar.
 */
export function pivotMarkers(pivots, bars) {
  if (!Array.isArray(pivots) || pivots.length === 0) return [];
  if (!Array.isArray(bars) || bars.length === 0) return [];

  const times = bars.map((bar) => bar.time);
  const byTime = new Map();

  for (const pivot of pivots) {
    if (!pivot || !isFiniteNumber(pivot.price) || !isFiniteNumber(pivot.time)) continue;
    const snapped = snapDown(times, pivot.time);
    if (snapped === null) continue;

    const kind = pivot.kind === 'high' ? 'high' : 'low';
    const time = extremeBar(bars, times, snapped, pivot, kind, snapped);
    // Two pivots landing on one bar: the first one wins, so markers never stack.
    if (byTime.has(time)) continue;
    byTime.set(time, { time, price: pivot.price, kind });
  }

  return [...byTime.values()].sort((a, b) => a.time - b.time);
}

/**
 * Group pivots that sit at practically the same price, then keep the levels
 * closest to `lastPrice` - the ones a trader is actually watching. Returns
 * `above` (resistance, nearest first) and `below` (support, nearest first).
 */
export function keyLevels(pivots, lastPrice, options = {}) {
  const max = Number.isFinite(options.max) ? Math.max(1, options.max) : DEFAULT_MAX_LEVELS;
  const tolerance = Number.isFinite(options.tolerance) ? options.tolerance : DEFAULT_TOLERANCE;
  const minDistance = Number.isFinite(options.minDistance) ? options.minDistance : DEFAULT_MIN_DISTANCE;

  const clean = (Array.isArray(pivots) ? pivots : []).filter(
    (pivot) => pivot && isFiniteNumber(pivot.price) && pivot.price > 0,
  );
  if (clean.length === 0) return { above: [], below: [], markers: [], all: [] };

  const sorted = [...clean].sort((a, b) => a.price - b.price);
  const clusters = [];
  for (const pivot of sorted) {
    const current = clusters[clusters.length - 1];
    if (current && Math.abs(pivot.price - current.price) <= current.price * tolerance) {
      current.touches += 1;
      current.price = (current.price * (current.touches - 1) + pivot.price) / current.touches;
      // The cluster is labelled by its most recent swing.
      if ((pivot.time ?? 0) >= (current.time ?? 0)) {
        current.time = pivot.time ?? current.time;
        current.kind = pivot.kind === 'high' ? 'high' : 'low';
      }
    } else {
      clusters.push({
        price: pivot.price,
        touches: 1,
        time: pivot.time ?? 0,
        kind: pivot.kind === 'high' ? 'high' : 'low',
      });
    }
  }

  let above = [];
  let below = [];
  if (isFiniteNumber(lastPrice) && lastPrice > 0) {
    // Anything within `minDistance` of the last price is treated as "at the price"
    // and left out, so the lines that remain are real gaps above and below.
    const far = (level) => Math.abs(level.price / lastPrice - 1) > minDistance;
    above = clusters.filter((level) => level.price > lastPrice && far(level)).sort((a, b) => a.price - b.price);
    below = clusters
      .filter((level) => level.price <= lastPrice && far(level))
      .sort((a, b) => b.price - a.price);

    const wantAbove = Math.ceil(max / 2);
    const wantBelow = max - wantAbove;
    above = above.slice(0, wantAbove);
    below = below.slice(0, wantBelow + Math.max(0, wantAbove - above.length));
    above = above.slice(0, wantAbove + Math.max(0, wantBelow - below.length));
  } else {
    // No price to measure from: fall back to the most recent swings.
    const recent = [...clusters].sort((a, b) => (b.time ?? 0) - (a.time ?? 0)).slice(0, max);
    above = recent.filter((level) => level.kind === 'high');
    below = recent.filter((level) => level.kind === 'low');
  }

  const all = [...above, ...below].sort((a, b) => b.price - a.price);
  return { above, below, all, markers: all };
}

/** Percent from `lastPrice` to `price` ("+1.80"), or null when unusable. */
export function levelDistance(price, lastPrice) {
  if (!isFiniteNumber(price) || !isFiniteNumber(lastPrice) || lastPrice === 0) return null;
  return (price / lastPrice - 1) * 100;
}
