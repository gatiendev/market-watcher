<script>
  import {
    changeClass,
    formatBarTime,
    formatPct,
    formatPrice,
    formatSpan,
    trendGlyph,
    trendLabel,
  } from './format.js';

  let { rows = [], selected = '', timeframe = 'H4', onselect = () => {} } = $props();

  const TIMEFRAMES = ['H4', 'D1'];

  function snap(row) {
    return row?.timeframes?.[timeframe] ?? null;
  }

  function tfSnap(row, tf) {
    return row?.timeframes?.[tf] ?? null;
  }

  /** How long the trend has been running, e.g. "3w". */
  function trendAge(row, tf) {
    const s = tfSnap(row, tf);
    if (!s?.trend_since) return '';
    return formatSpan(Date.now() - s.trend_since);
  }

  function trendTitle(row, tf) {
    const s = tfSnap(row, tf);
    if (!s?.trend) return `${tf}: waiting for 200 bars`;
    const relation = s.trend === 'up' ? 'above' : 'below';
    const age = trendAge(row, tf);
    const span = s.trend_bars === 1 ? '1 bar' : `${s.trend_bars} bars`;
    const since = s.trend_since ? ` since ${formatBarTime(s.trend_since)}, ${span}` : '';
    return `${tf} ${trendLabel(s.trend)}${age ? ` for ${age}` : ''}${since}: EMA 50 ${formatPrice(s.ema50, row.decimals)} ${relation} EMA 200 ${formatPrice(s.ema200, row.decimals)}`;
  }

  function pick(symbol) {
    onselect(symbol);
  }

  function onKey(event, symbol) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      pick(symbol);
    }
  }
</script>

<div class="table-wrap">
  {#if rows.length === 0}
    <p class="skeleton">Waiting for the first sync…</p>
  {:else}
    <table class="watch">
      <thead>
        <tr>
          <th>Symbol</th>
          <th>Last</th>
          <th>Chg</th>
          <th>EMA 50</th>
          <th>EMA 200</th>
          <th class="trend-head">Trend</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as row (row.symbol)}
          {@const s = snap(row)}
          <tr
            class:selected={row.symbol === selected}
            tabindex="0"
            role="button"
            aria-pressed={row.symbol === selected}
            aria-label={`Show ${row.symbol} chart`}
            onclick={() => pick(row.symbol)}
            onkeydown={(event) => onKey(event, row.symbol)}
          >
            <td>
              <div class="sym">
                <span class="ticker">{row.symbol}</span>
                <span class="name">{row.label}</span>
              </div>
            </td>
            <td class="price">{formatPrice(s?.last_close, row.decimals)}</td>
            <td>
              <span class="chg {changeClass(s?.change_pct)}">{formatPct(s?.change_pct)}</span>
            </td>
            <td class="ema-cell">{formatPrice(s?.ema50, row.decimals)}</td>
            <td class="ema-cell slow">{formatPrice(s?.ema200, row.decimals)}</td>
            <td class="trend-cell">
              {#each TIMEFRAMES as tf}
                <span
                  class="trend-line {tfSnap(row, tf)?.trend ?? 'flat'}"
                  title={trendTitle(row, tf)}
                >
                  <span class="tf">{tf}</span>
                  <span class="glyph">{trendGlyph(tfSnap(row, tf)?.trend)}</span>
                  <span class="age">{trendAge(row, tf)}</span>
                </span>
              {/each}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>
