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
  let loading = $state(true);
  let error = $state('');
  let filter = $state(ALL);
  let timeframe = $state('H4');
  let cardLimit = $state(CARD_PAGE);

  let symbols = $derived([...new Set(rows.map((row) => row.symbol))]);
  let visible = $derived(filter === ALL ? rows : rows.filter((row) => row.symbol === filter));
  // Newest crossing first across every asset, which is what the panel promises:
  // the API groups its rows per series.
  let shown = $derived(
    visible
      .filter((row) => row.timeframe === timeframe)
      .sort((a, b) => b.crossed_at - a.crossed_at),
  );
  let cards = $derived(shown.slice(0, cardLimit));

  function resetCards() {
    cardLimit = CARD_PAGE;
  }

  // Every figure is price against the crossing close, never direction-relative:
  // a down run reads negative, the way the price actually moved.
  function figures(row) {
    const down = row.direction === 'down';
    return {
      high: down ? -row.trough_pct : row.peak_pct,
      low: down ? -row.peak_pct : row.trough_pct,
      swing: down ? -row.trough_to_peak_pct : row.trough_to_peak_pct,
      result: down ? -row.exit_pct : row.exit_pct,
    };
  }

  // The swing is the trade a user could actually have taken, in the order the
  // market made it: buy the dip then sell the high, or sell the high then cover
  // the low. Never the other way round.
  function swingLabel(direction) {
    return direction === 'down' ? 'peak → trough' : 'trough → peak';
  }

  function median(values) {
    if (values.length === 0) return 0;
    const sorted = [...values].sort((a, b) => a - b);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
  }

  // Averages over exactly the rows on screen, in the same raw terms as the table.
  let summary = $derived.by(() => {
    if (shown.length === 0) return null;
    const list = shown.map(figures);
    const mean = (values) => values.reduce((total, value) => total + value, 0) / values.length;
    return {
      crossings: shown.length,
      ups: shown.filter((row) => row.direction === 'up').length,
      downs: shown.filter((row) => row.direction === 'down').length,
      open: shown.filter((row) => row.open).length,
      avgHigh: mean(list.map((fig) => fig.high)),
      avgLow: mean(list.map((fig) => fig.low)),
      medianDays: median(shown.map((row) => row.days_to_peak)),
      avgSwing: mean(list.map((fig) => Math.abs(fig.swing))),
    };
  });

  // The open run of each series is the trend in force right now, so the page can
  // show both timeframes side by side instead of hiding one behind the toggle.
  let nowSymbols = $derived([...new Set(visible.map((row) => row.symbol))]);

  function openRun(symbol, tf) {
    return visible.find((row) => row.open && row.symbol === symbol && row.timeframe === tf) ?? null;
  }

  // The run figures are direction-relative (positive = the signal paid). This block
  // answers "where is price against the crossing", so a down trend reads negative.
  function priceMove(run, value) {
    return run.direction === 'down' ? -value : value;
  }

  function labelFor(symbol) {
    return rows.find((row) => row.symbol === symbol)?.label ?? '';
  }

  onMount(async () => {
    try {
      const payload = await fetchCrossings();
      rows = payload.rows ?? [];
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
                    {@const nowPct = priceMove(run, run.exit_pct)}
                    {@const peakPct = priceMove(run, run.peak_pct)}
                    <span class="now-vals">
                      <span
                        class="now-pnl {changeClass(nowPct)}"
                        title="Price now against the crossing close"
                      >
                        <span class="k">now</span> {formatPct(nowPct)}
                      </span>
                      <span class="now-peak">
                        <span class="k">{run.direction === 'down' ? 'trough' : 'peak'}</span>
                        {formatPct(peakPct)}
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
          {#if summary}
            <p class="summary">
              <span class="stat">
                <b>{summary.crossings}</b>
                {summary.crossings === 1 ? 'crossing' : 'crossings'}
              </span>
              <span class="stat">
                <b class="up">{trendGlyph('up')} {summary.ups}</b> up ·
                <b class="down">{trendGlyph('down')} {summary.downs}</b> down
              </span>
              <span class="stat">
                avg high <b class={changeClass(summary.avgHigh)}>{formatPct(summary.avgHigh)}</b>
              </span>
              <span class="stat">
                avg low <b class={changeClass(summary.avgLow)}>{formatPct(summary.avgLow)}</b>
              </span>
              <span class="stat">median <b>{formatDays(summary.medianDays)}</b> to best</span>
              <span class="stat" title="Average travel from the worst price to the best">
                avg swing <b>{summary.avgSwing.toFixed(2)}%</b>
              </span>
              {#if summary.open > 0}
                <span class="stat"><b>{summary.open}</b> still open</span>
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
                  <th title="Highest price of the run, against the crossing close">High</th>
                  <th title="Lowest price of the run, against the crossing close">Low</th>
                  <th title="From the adverse extreme to the best price: trough → peak on up runs, peak → trough on down runs">
                    Swing
                  </th>
                  <th title="Days until the best price of the run">To best</th>
                  <th title="Price change from the crossing close to the end of the run">Result</th>
                </tr>
              </thead>
              <tbody>
                {#each shown as row (row.symbol + row.timeframe + row.crossed_at)}
                  {@const fig = figures(row)}
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
                    <td class="num {changeClass(fig.high)}">{formatPct(fig.high)}</td>
                    <td class="num {changeClass(fig.low)}">{formatPct(fig.low)}</td>
                    <td class="num swing {changeClass(fig.swing)}" title={swingLabel(row.direction)}>
                      {formatPct(fig.swing)}
                    </td>
                    <td
                      class="num days"
                      title={`${row.peak_bars} ${timeframe} bars to the best price · run lasted ${formatDays(
                        row.run_days,
                      )}`}
                    >
                      {formatDays(row.days_to_peak)}
                    </td>
                    <td class="num result">
                      <span class={changeClass(fig.result)}>{formatPct(fig.result)}</span>
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
              {@const fig = figures(row)}
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
                    <dt>High</dt>
                    <dd class={changeClass(fig.high)}>{formatPct(fig.high)}</dd>
                  </div>
                  <div>
                    <dt>Low</dt>
                    <dd class={changeClass(fig.low)}>{formatPct(fig.low)}</dd>
                  </div>
                  <div>
                    <dt>Swing</dt>
                    <dd class={changeClass(fig.swing)} title={swingLabel(row.direction)}>
                      {formatPct(fig.swing)}
                    </dd>
                  </div>
                  <div>
                    <dt>Days to best</dt>
                    <dd>{formatDays(row.days_to_peak)}</dd>
                  </div>
                </dl>
                <div class="card-result">
                  <span class="label">Result</span>
                  <span class="value {changeClass(fig.result)}">{formatPct(fig.result)}</span>
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
    <span>High, low and result are price against the crossing close</span>
    <span class="sep"></span>
    <span>Swing runs from the worst price to the best, in the order they happened</span>
    <span class="sep"></span>
    <span>Crypto: Binance · FX &amp; index: Yahoo Finance</span>
  </footer>
</div>
