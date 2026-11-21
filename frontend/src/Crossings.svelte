<script>
  import { onMount } from 'svelte';
  import { fetchCrossings } from './lib/api.js';
  import {
    changeClass,
    formatBarTime,
    formatDate,
    formatDays,
    formatPct,
    formatPrice,
    trendGlyph,
  } from './lib/format.js';

  // Both timeframes are always shown: the page is the full record, not a filter.
  const TIMEFRAMES = ['H4', 'D1'];
  const ALL = 'ALL';

  let rows = $state([]);
  let summaries = $state([]);
  let loading = $state(true);
  let error = $state('');
  let filter = $state(ALL);

  let symbols = $derived([...new Set(rows.map((row) => row.symbol))]);
  let visible = $derived(filter === ALL ? rows : rows.filter((row) => row.symbol === filter));

  function timeframeRows(timeframe) {
    return visible.filter((row) => row.timeframe === timeframe);
  }

  // The server sends one summary for every asset and one per asset, so the
  // figures under a table always describe the rows in that table.
  function summaryFor(timeframe) {
    return (
      summaries.find(
        (summary) =>
          summary.timeframe === timeframe &&
          (filter === ALL ? summary.symbol === null : summary.symbol === filter),
      ) ?? null
    );
  }

  function labelFor(symbol) {
    return rows.find((row) => row.symbol === symbol)?.label ?? '';
  }

  onMount(async () => {
    try {
      const payload = await fetchCrossings();
      rows = payload.rows ?? [];
      summaries = payload.summaries ?? [];
    } catch (err) {
      error = err?.message ?? 'Cannot load the crossings';
    } finally {
      loading = false;
    }
  });
</script>

<div class="page">
  <header class="topbar">
    <div class="brand">
      <img class="mark" src="/logo.png" alt="" width="191" height="256" />
      <h1>Market Watcher</h1>
      <span class="tag">Crossings</span>
    </div>

    <div class="controls">
      <div class="chips" role="group" aria-label="Asset">
        <button class="chip" aria-pressed={filter === ALL} onclick={() => (filter = ALL)}>
          All assets
        </button>
        {#each symbols as symbol}
          <button class="chip" aria-pressed={filter === symbol} onclick={() => (filter = symbol)}>
            {symbol}
          </button>
        {/each}
      </div>
      <a class="nav-link" href="/">← Dashboard</a>
    </div>
  </header>

  <main class="page-body">
    {#if error}
      <p class="error-bar">{error}</p>
    {/if}

    {#if loading}
      <p class="skeleton">Loading crossings…</p>
    {:else if rows.length === 0}
      <section class="panel">
        <div class="panel-head">
          <h2>No crossings yet</h2>
        </div>
        <p class="panel-empty">
          A series needs 200 bars before both averages exist, so the first crossing can only be
          counted after that. The history fills in as the sync runs.
        </p>
      </section>
    {:else}
      {#each TIMEFRAMES as timeframe}
        {@const list = timeframeRows(timeframe)}
        {@const summary = summaryFor(timeframe)}
        {#if list.length > 0}
          <section class="panel">
            <div class="panel-head">
              <h2>{timeframe} crossings</h2>
              <span class="hint">
                {filter === ALL ? 'all assets' : labelFor(filter)} · newest first
              </span>
            </div>

            {#if summary}
              <p class="summary">
                <span class="stat"><b>{summary.crossings}</b> crossings</span>
                <span class="stat">
                  <b class="up">{trendGlyph('up')} {summary.ups}</b> up ·
                  <b class="down">{trendGlyph('down')} {summary.downs}</b> down
                </span>
                <span class="stat">
                  avg peak <b class={changeClass(summary.avg_peak_pct)}>{formatPct(summary.avg_peak_pct)}</b>
                </span>
                <span class="stat">
                  median <b>{formatDays(summary.median_days_to_peak)}</b> to peak
                </span>
                <span class="stat">
                  trough → peak <b>{formatPct(summary.avg_trough_to_peak_pct)}</b>
                </span>
                {#if summary.avg_exit_pct !== null}
                  <span class="stat">
                    avg result <b class={changeClass(summary.avg_exit_pct)}>{formatPct(summary.avg_exit_pct)}</b>
                  </span>
                {/if}
                {#if summary.open_runs > 0}
                  <span class="stat"><b>{summary.open_runs}</b> still open</span>
                {/if}
              </p>
            {/if}

            <div class="cross-table-wrap">
              <table class="crossings">
                <thead>
                  <tr>
                    <th>Asset</th>
                    <th>Crossed</th>
                    <th>Dir</th>
                    <th>Price</th>
                    <th>Peak</th>
                    <th>To peak</th>
                    <th>Trough</th>
                    <th>Trough → peak</th>
                    <th>Result</th>
                  </tr>
                </thead>
                <tbody>
                  {#each list as row (row.symbol + row.timeframe + row.crossed_at)}
                    <tr>
                      <td class="asset">
                        <span class="ticker num">{row.symbol}</span>
                        <span class="name">{row.label}</span>
                      </td>
                      <td class="num when" title={formatBarTime(row.crossed_at * 1000)}>
                        {formatDate(row.crossed_at * 1000)}
                      </td>
                      <td>
                        <span class="badge {row.direction}">
                          {trendGlyph(row.direction)} {row.direction}
                        </span>
                      </td>
                      <td class="num">{formatPrice(row.price, row.decimals)}</td>
                      <td class="num {changeClass(row.peak_pct)}">{formatPct(row.peak_pct)}</td>
                      <td
                        class="num days"
                        title={`${row.peak_bars} ${timeframe} bars to the peak · run lasted ${formatDays(
                          row.run_days,
                        )}`}
                      >
                        {formatDays(row.days_to_peak)}
                      </td>
                      <td class="num {changeClass(row.trough_pct)}">{formatPct(row.trough_pct)}</td>
                      <td class="num {changeClass(row.trough_to_peak_pct)}">
                        {formatPct(row.trough_to_peak_pct)}
                      </td>
                      <td class="num result">
                        <span class={changeClass(row.exit_pct)}>{formatPct(row.exit_pct)}</span>
                        {#if row.open}
                          <span class="open-badge" title="This run is the current trend">open</span>
                        {/if}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </section>
        {/if}
      {/each}
    {/if}
  </main>

  <footer class="footer">
    <span>Measured from the crossing bar's close</span>
    <span class="sep"></span>
    <span>A run ends at the opposite crossing</span>
    <span class="sep"></span>
    <span>Favourable and adverse moves are read in the signal's direction</span>
    <span class="sep"></span>
    <span>Crypto: Binance · FX &amp; index: Yahoo Finance</span>
  </footer>
</div>
