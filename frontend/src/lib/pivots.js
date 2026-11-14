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
  if (!Array.isArray(times) || times.length === 0 || !isFiniteNumber(target)) return null;
  if (target < times[0]) return null;
  let low = 0;
  let high = times.length - 1;
  let best = null;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (times[mid] <= target) {
      best = times[mid];
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  return best;
}

/**
 * Pivots mapped onto bar times the series really has, one marker per bar,
 * oldest first. Dropped pivots are the ones older than the first stored bar.
 */
export function pivotMarkers(pivots, barTimes) {
  if (!Array.isArray(pivots) || pivots.length === 0) return [];
  if (!Array.isArray(barTimes) || barTimes.length === 0) return [];

  const byTime = new Map();
  for (const pivot of pivots) {
    if (!pivot || !isFiniteNumber(pivot.price) || !isFiniteNumber(pivot.time)) continue;
    const snapped = snapDown(barTimes, pivot.time);
    if (snapped === null) continue;
    const exact = snapped === pivot.time;
    const previous = byTime.get(snapped);
    // Two pivots landing on one bar: keep the exact hit, otherwise the first one.
    if (previous && (previous.exact || !exact)) continue;
    byTime.set(snapped, {
      time: snapped,
      price: pivot.price,
      kind: pivot.kind === 'high' ? 'high' : 'low',
      exact,
    });
  }

  return [...byTime.values()]
    .sort((a, b) => a.time - b.time)
    .map(({ time, price, kind }) => ({ time, price, kind }));
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
