<script>
  import { changeClass, formatPct, formatPrice } from './format.js';

  let { rows = [], selected = '', timeframe = 'H4', onselect = () => {} } = $props();

  function snap(row) {
    return row?.timeframes?.[timeframe] ?? null;
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
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>
