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

  // One table at a time, switched by the toggle in the header.
  const TIMEFRAMES = ['H4', 'D1'];
  const ALL = 'ALL';
  // Phones get cards; 100 of them in one scroll is too much, so they come in pages.
  const CARD_PAGE = 25;

  let rows = $state([]);
  let summaries = $state([]);
  let loading = $state(true);
  let error = $state('');
  let filter = $state(ALL);
  let timeframe = $state('H4');
  let cardLimit = $state(CARD_PAGE);

  let symbols = $derived([...new Set(rows.map((row) => row.symbol))]);
  let visible = $derived(filter === ALL ? rows : rows.filter((row) => row.symbol === filter));
  let shown = $derived(visible.filter((row) => row.timeframe === timeframe));
  let cards = $derived(shown.slice(0, cardLimit));

  function resetCards() {
    cardLimit = CARD_PAGE;
  }

  // The server sends one summary for every asset and one per asset, so the
  // figures under a table always describe the rows in that table.
  function summaryFor(tf) {
    return (
      summaries.find(
        (summary) =>
          summary.timeframe === tf &&
          (filter === ALL ? summary.symbol === null : summary.symbol === filter),
      ) ?? null
    );
  }

  let activeSummary = $derived(summaryFor(timeframe));

  // The open run of each series is the trend in force right now, so the page can
  // show both timeframes side by side instead of hiding one behind the toggle.
  let nowSymbols = $derived([...new Set(visible.map((row) => row.symbol))]);

  function openRun(symbol, tf) {
    return visible.find((row) => row.open && row.symbol === symbol && row.timeframe === tf) ?? null;
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
      <div class="segmented" role="group" aria-label="Timeframe">
        {#each TIMEFRAMES as tf}
          <button
            aria-pressed={tf === timeframe}
            onclick={() => {
              timeframe = tf;
              resetCards();
            }}>{tf}</button
          >
        {/each}
      </div>
      <div class="chips" role="group" aria-label="Asset">
        <button
          class="chip"
          aria-pressed={filter === ALL}
          onclick={() => {
            filter = ALL;
            resetCards();
          }}>All assets</button
        >
        {#each symbols as symbol}
          <button
            class="chip"
            aria-pressed={filter === symbol}
            onclick={() => {
              filter = symbol;
              resetCards();
            }}>{symbol}</button
          >
        {/each}
      </div>
    </div>
    <a class="nav-link" href="/">← Dashboard</a>
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
      <section class="panel now">
        <div class="panel-head">
          <h2>Right now</h2>
          <span class="hint">the trend in force on both timeframes</span>
        </div>
        <ul class="now-list">
          {#each nowSymbols as symbol}
            <li class="now-asset">
              <div class="now-head">
                <span class="ticker num">{symbol}</span>
                <span class="name">{labelFor(symbol)}</span>
              </div>
              {#each TIMEFRAMES as tf}
                {@const run = openRun(symbol, tf)}
                <div class="now-row">
                  <span class="now-tf">{tf}</span>
                  {#if run}
                    <span class="badge {run.direction}">
                      {trendGlyph(run.direction)} {run.direction}
                    </span>
                    <span class="now-age" title={`Crossed ${formatBarTime(run.crossed_at * 1000)}`}>
                      {formatDays(run.run_days)}<span class="since">
                        {' '}· {formatDate(run.crossed_at * 1000)}</span
                      >
                    </span>
                    <span class="now-vals">
                      <span
                        class="now-pnl {changeClass(run.exit_pct)}"
                        title="Result so far, from the crossing close to the latest close"
                      >
                        <span class="k">now</span> {formatPct(run.exit_pct)}
                      </span>
                      <span class="now-peak">
                        <span class="k">peak</span> {formatPct(run.peak_pct)}
                      </span>
                    </span>
                  {:else}
                    <span class="now-none">no crossing stored yet</span>
                  {/if}
                </div>
              {/each}
            </li>
          {/each}
        </ul>
      </section>

      <section class="panel">
        <div class="panel-head">
          <h2>{timeframe} crossings</h2>
          <span class="hint">
            {filter === ALL ? 'all assets' : labelFor(filter)} · newest first
          </span>
        </div>

        {#if shown.length === 0}
          <p class="panel-empty">
            No {timeframe} crossing stored yet for {filter === ALL ? 'these assets' : filter}.
          </p>
        {:else}
          {#if activeSummary}
            <p class="summary">
              <span class="stat">
                <b>{activeSummary.crossings}</b>
                {activeSummary.crossings === 1 ? 'crossing' : 'crossings'}
              </span>
              <span class="stat">
                <b class="up">{trendGlyph('up')} {activeSummary.ups}</b> up ·
                <b class="down">{trendGlyph('down')} {activeSummary.downs}</b> down
              </span>
              <span class="stat">
                avg peak <b class={changeClass(activeSummary.avg_peak_pct)}>{formatPct(activeSummary.avg_peak_pct)}</b>
              </span>
              <span class="stat">
                median <b>{formatDays(activeSummary.median_days_to_peak)}</b> to peak
              </span>
              <span class="stat">
                trough → peak <b>{formatPct(activeSummary.avg_trough_to_peak_pct)}</b>
              </span>
              {#if activeSummary.avg_exit_pct !== null}
                <span class="stat">
                  avg result <b class={changeClass(activeSummary.avg_exit_pct)}>{formatPct(activeSummary.avg_exit_pct)}</b>
                </span>
              {/if}
              {#if activeSummary.open_runs > 0}
                <span class="stat"><b>{activeSummary.open_runs}</b> still open</span>
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
                {#each shown as row (row.symbol + row.timeframe + row.crossed_at)}
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

          <!-- Phones read the same rows as cards: nine columns never fit a 390px screen. -->
          <ul class="cross-cards">
            {#each cards as row (row.symbol + row.timeframe + row.crossed_at)}
              <li class="cross-card">
                <div class="card-top">
                  <span class="ticker num">{row.symbol}</span>
                  <span class="badge {row.direction}">
                    {trendGlyph(row.direction)} {row.direction}
                  </span>
                </div>
                <div class="card-sub">
                  <span class="when" title={formatBarTime(row.crossed_at * 1000)}>
                    {formatDate(row.crossed_at * 1000)}
                  </span>
                  <span class="at num">{formatPrice(row.price, row.decimals)}</span>
                </div>
                <dl class="card-metrics">
                  <div>
                    <dt>Peak</dt>
                    <dd class={changeClass(row.peak_pct)}>{formatPct(row.peak_pct)}</dd>
                  </div>
                  <div>
                    <dt>Days to peak</dt>
                    <dd>{formatDays(row.days_to_peak)}</dd>
                  </div>
                  <div>
                    <dt>Trough</dt>
                    <dd class={changeClass(row.trough_pct)}>{formatPct(row.trough_pct)}</dd>
                  </div>
                  <div>
                    <dt>Trough → peak</dt>
                    <dd class={changeClass(row.trough_to_peak_pct)}>
                      {formatPct(row.trough_to_peak_pct)}
                    </dd>
                  </div>
                </dl>
                <div class="card-result">
                  <span class="label">Result</span>
                  <span class="value {changeClass(row.exit_pct)}">{formatPct(row.exit_pct)}</span>
                  {#if row.open}
                    <span class="open-badge" title="This run is the current trend">open</span>
                  {/if}
                </div>
              </li>
            {/each}
            {#if shown.length > cards.length}
              <li class="more-row">
                <button class="more" onclick={() => (cardLimit += CARD_PAGE)}>
                  Show {Math.min(CARD_PAGE, shown.length - cards.length)} more ·
                  {shown.length - cards.length} left
                </button>
              </li>
            {/if}
          </ul>
        {/if}
      </section>
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
