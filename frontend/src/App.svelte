<script>
  import { onMount } from 'svelte';
  import PriceChart from './lib/PriceChart.svelte';
  import SymbolTable from './lib/SymbolTable.svelte';
  import { fetchCandles, fetchMarkets, fetchPivots, triggerRefresh } from './lib/api.js';
  import { keyLevels, levelDistance } from './lib/pivots.js';
  import {
    changeClass,
    emaSpread,
    formatBarTime,
    formatPct,
    formatPrice,
    formatRelative,
    sortSymbols,
    trendLabel,
  } from './lib/format.js';

  const POLL_MS = 60_000;
  // Enough history to zoom out over months, not just the last few weeks.
  const CHART_BARS = 2000;
  const TIMEFRAMES = ['H4', 'D1'];

  // Pivot strength is set in daily bars each side of the swing; 15 finds the
  // big levels, 5 finds every minor one.
  const PIVOT_STEP = 5;
  const PIVOT_MIN = 5;
  const PIVOT_MAX = 45;
  const PIVOT_DEFAULT = 15;
  const PIVOT_PREFS = 'market-watcher:pivots';

  let markets = $state([]);
  let selected = $state('SOLUSDT');
  let timeframe = $state('H4');
  let points = $state([]);
  let error = $state('');
  let booting = $state(true);
  let chartBusy = $state(false);
  let refreshing = $state(false);
  let lastLoaded = $state(0);
  let now = $state(Date.now());

  let candlesAbort = null;
  let pivotsAbort = null;

  let pivotsOn = $state(false);
  let lookback = $state(PIVOT_DEFAULT);
  let pivots = $state([]);

  let rows = $derived(sortSymbols(markets));
  let activeRow = $derived(rows.find((row) => row.symbol === selected) ?? null);
  let snap = $derived(activeRow?.timeframes?.[timeframe] ?? null);
  let decimals = $derived(activeRow?.decimals ?? 2);
  let spread = $derived(emaSpread(snap?.ema50, snap?.ema200));
  let stale = $derived(!lastLoaded || now - lastLoaded > 5 * 60_000);
  let levels = $derived(keyLevels(pivots, snap?.last_close ?? null));
  let resistance = $derived(levels.above[0] ?? null);
  let support = $derived(levels.below[0] ?? null);

  function readPrefs() {
    try {
      const raw = localStorage.getItem(PIVOT_PREFS);
      const stored = raw ? JSON.parse(raw) : {};
      if (typeof stored.on === 'boolean') pivotsOn = stored.on;
      if (Number.isFinite(stored.lookback)) {
        lookback = Math.min(PIVOT_MAX, Math.max(PIVOT_MIN, stored.lookback));
      }
    } catch {
      // Private mode or a corrupted value: the defaults stand.
    }
  }

  function savePrefs() {
    try {
      localStorage.setItem(PIVOT_PREFS, JSON.stringify({ on: pivotsOn, lookback }));
    } catch {
      // Persisting the toggle is a nice-to-have, never a reason to fail.
    }
  }

  async function loadMarkets() {
    try {
      const payload = await fetchMarkets();
      markets = payload.symbols ?? [];
    } catch (err) {
      if (err?.name !== 'AbortError') error = err.message ?? 'Cannot load the watchlist';
    }
  }

  async function loadCandles(symbol = selected, tf = timeframe, silent = false) {
    candlesAbort?.abort();
    const controller = new AbortController();
    candlesAbort = controller;
    if (!silent) chartBusy = true;

    try {
      const payload = await fetchCandles(symbol, tf, CHART_BARS, controller.signal);
      if (controller.signal.aborted) return;
      points = payload.points ?? [];
      lastLoaded = Date.now();
      error = '';
    } catch (err) {
      if (err?.name !== 'AbortError') error = err.message ?? 'Cannot load candles';
    } finally {
      if (!silent) chartBusy = false;
    }
  }

  async function loadPivots(symbol = selected, strength = lookback) {
    pivotsAbort?.abort();
    const controller = new AbortController();
    pivotsAbort = controller;

    try {
      const payload = await fetchPivots(symbol, strength, controller.signal);
      if (controller.signal.aborted) return;
      pivots = payload.pivots ?? [];
    } catch (err) {
      if (err?.name !== 'AbortError') pivots = [];
    }
  }

  async function togglePivots() {
    pivotsOn = !pivotsOn;
    savePrefs();
    if (pivotsOn && pivots.length === 0) await loadPivots();
  }

  async function setLookback(value) {
    const stepped = Math.round(value / PIVOT_STEP) * PIVOT_STEP;
    const next = Math.min(PIVOT_MAX, Math.max(PIVOT_MIN, stepped));
    if (next === lookback) return;
    lookback = next;
    savePrefs();
    if (pivotsOn) await loadPivots(selected, next);
  }

  async function manualRefresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      await triggerRefresh();
      await loadMarkets();
      await loadCandles(selected, timeframe, true);
    } catch (err) {
      error = err?.message ?? 'Refresh failed';
    } finally {
      refreshing = false;
    }
  }

  function selectSymbol(symbol) {
    if (symbol === selected) return;
    selected = symbol;
    loadCandles(symbol, timeframe);
    if (pivotsOn) loadPivots(symbol, lookback);
  }

  function pickTimeframe(tf) {
    if (tf === timeframe) return;
    timeframe = tf;
    loadCandles(selected, tf);
  }

  onMount(() => {
    readPrefs();

    (async () => {
      await loadMarkets();
      await loadCandles();
      if (pivotsOn) await loadPivots();
      booting = false;
    })();

    const clock = setInterval(() => {
      now = Date.now();
    }, 15_000);

    const poll = setInterval(async () => {
      await loadMarkets();
      await loadCandles(selected, timeframe, true);
      if (pivotsOn) await loadPivots();
    }, POLL_MS);

    return () => {
      clearInterval(clock);
      clearInterval(poll);
      candlesAbort?.abort();
      pivotsAbort?.abort();
    };
  });
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">
      <img class="mark" src="/logo.png" alt="" width="191" height="256" />
      <span class="live-dot" class:stale></span>
      <h1>Market Watcher</h1>
      <span class="tag">EMA 50 / 200</span>
    </div>

    <div class="controls">
      <div class="segmented" role="group" aria-label="Timeframe">
        {#each TIMEFRAMES as tf}
          <button aria-pressed={tf === timeframe} onclick={() => pickTimeframe(tf)}>{tf}</button>
        {/each}
      </div>
      <button class="refresh" class:busy={refreshing} disabled={refreshing} onclick={manualRefresh}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
          <path d="M21 12a9 9 0 1 1-3.2-6.9" />
          <path d="M21 3v6h-6" />
        </svg>
        {refreshing ? 'Syncing' : 'Refresh'}
      </button>
    </div>
  </header>

  <section class="watchlist">
    <div class="panel-head">
      <h2>Watchlist</h2>
      <span class="hint">{timeframe} · {rows.length} symbols</span>
    </div>
    <SymbolTable {rows} {selected} {timeframe} onselect={selectSymbol} />
  </section>

  <section class="chart-panel">
    <div class="chart-head">
      <div class="instrument">
        <span class="ticker num">{activeRow?.symbol ?? '—'}</span>
        <span class="sub">{timeframe} · {activeRow?.label ?? ''}{snap?.bars ? ` · ${snap.bars} bars cached` : ''}</span>
      </div>

      <div class="pivot-bar">
        <button
          class="pivot-toggle"
          aria-pressed={pivotsOn}
          onclick={togglePivots}
          title="Daily pivot highs and lows, drawn as markers and key levels"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 18l4-7 4 5 4-8 6 10" />
          </svg>
          Pivots
        </button>
        {#if pivotsOn}
          <div class="stepper" role="group" aria-label="Daily bars required each side of a pivot">
            <button
              type="button"
              onclick={() => setLookback(lookback - PIVOT_STEP)}
              disabled={lookback <= PIVOT_MIN}
              aria-label="Weaker pivots">−</button>
            <span class="steps num" title="Daily bars each side of the pivot">{lookback}<span class="unit">/side</span></span>
            <button
              type="button"
              onclick={() => setLookback(lookback + PIVOT_STEP)}
              disabled={lookback >= PIVOT_MAX}
              aria-label="Stronger pivots">+</button>
          </div>
        {/if}
      </div>

      <div class="quote">
        <span class="last num">{formatPrice(snap?.last_close, decimals)}</span>
        <span class="delta num {changeClass(snap?.change_pct)}">{formatPct(snap?.change_pct)}</span>
      </div>
    </div>

    <div class="legend">
      <span class="legend-item">
        <span class="swatch" style="background:#4c9aff"></span>
        <span class="label">EMA 50</span>
        <span class="value">{formatPrice(snap?.ema50, decimals)}</span>
      </span>
      <span class="legend-item">
        <span class="swatch" style="background:#f7931a"></span>
        <span class="label">EMA 200</span>
        <span class="value">{formatPrice(snap?.ema200, decimals)}</span>
      </span>
      <span class="badge {snap?.trend ?? ''}">{trendLabel(snap?.trend)}</span>
      {#if pivotsOn}
        {#if resistance}
          <span class="level-chip r" title="Nearest pivot high above the price">
            <span class="tag">R</span>
            <span class="num">{formatPrice(resistance.price, decimals)}</span>
            <span class="dist num">{formatPct(levelDistance(resistance.price, snap?.last_close))}</span>
          </span>
        {/if}
        {#if support}
          <span class="level-chip s" title="Nearest pivot low below the price">
            <span class="tag">S</span>
            <span class="num">{formatPrice(support.price, decimals)}</span>
            <span class="dist num">{formatPct(levelDistance(support.price, snap?.last_close))}</span>
          </span>
        {/if}
        {#if pivots.length === 0}
          <span class="level-empty">No pivots at {lookback}/side for this history</span>
        {/if}
      {/if}
      {#if spread !== null}
        <span class="spread">50/200 spread {formatPct(spread)}</span>
      {/if}
    </div>

    {#if error}
      <p class="error-bar">{error}</p>
    {/if}

    <PriceChart
      {points}
      {decimals}
      {timeframe}
      {pivots}
      showPivots={pivotsOn}
      resetKey={`${selected}-${timeframe}`}
    />

    {#if chartBusy && !booting}
      <div class="chart-overlay"><span class="spinner"></span></div>
    {/if}
  </section>

  <footer class="footer">
    <span>Crypto: Binance · FX &amp; index: Yahoo Finance</span>
    <span class="sep"></span>
    <span>Candles cached in SQLite</span>
    <span class="sep"></span>
    <span>Last bar {formatBarTime(snap?.last_bar_time)}</span>
    <span class="sep"></span>
    <span>Updated {formatRelative(lastLoaded, now)}</span>
    {#if snap?.status === 'error'}
      <span class="sep"></span>
      <span class="down">Sync error: {snap?.message ?? 'unknown'}</span>
    {/if}
  </footer>
</div>
