<script>
  import { onMount } from 'svelte';
  import { ColorType, CrosshairMode, LineStyle, createChart } from 'lightweight-charts';
  import { toCandles, toLine, toVolume } from './format.js';
  import { keyLevels, pivotMarkers } from './pivots.js';

  let {
    points = [],
    decimals = 2,
    timeframe = 'H4',
    resetKey = '',
    pivots = [],
    showPivots = false,
  } = $props();

  const COLORS = {
    up: '#16c784',
    down: '#ea3943',
    ema50: '#4c9aff',
    emaSlow: '#f7931a',
    grid: 'rgba(29, 38, 50, 0.5)',
    border: '#1d2632',
    text: '#8792a4',
  };

  let mount = $state(null);
  let chart = null;
  let series = null;
  let renderedKey = '';
  let levelLines = [];

  function priceFormat() {
    return { type: 'price', precision: decimals, minMove: Number(`0.${'0'.repeat(Math.max(0, decimals - 1))}1`) };
  }

  onMount(() => {
    const instance = createChart(mount, {
      autoSize: true,
      layout: {
        background: { type: ColorType.Solid, color: 'transparent' },
        textColor: COLORS.text,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        fontSize: 11,
      },
      grid: {
        vertLines: { color: COLORS.grid },
        horzLines: { color: COLORS.grid },
      },
      rightPriceScale: {
        borderColor: COLORS.border,
        scaleMargins: { top: 0.08, bottom: 0.26 },
      },
      timeScale: {
        borderColor: COLORS.border,
        timeVisible: timeframe === 'H4',
        secondsVisible: false,
        rightOffset: 4,
        barSpacing: 7,
        minBarSpacing: 0.6,
      },
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: '#39465a', width: 1, style: 3, labelBackgroundColor: '#1d2632' },
        horzLine: { color: '#39465a', width: 1, style: 3, labelBackgroundColor: '#1d2632' },
      },
      handleScroll: { mouseWheel: true, pressedMouseMove: true },
      handleScale: { mouseWheel: true, pinch: true, axisPressedMouseMove: true },
    });

    const candles = instance.addCandlestickSeries({
      upColor: COLORS.up,
      downColor: COLORS.down,
      wickUpColor: COLORS.up,
      wickDownColor: COLORS.down,
      borderVisible: false,
      priceLineColor: 'rgba(232, 236, 243, 0.35)',
      priceLineStyle: 2,
      priceFormat: priceFormat(),
    });

    const ema50 = instance.addLineSeries({
      color: COLORS.ema50,
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
      priceFormat: priceFormat(),
    });

    const ema200 = instance.addLineSeries({
      color: COLORS.emaSlow,
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
      priceFormat: priceFormat(),
    });

    const volume = instance.addHistogramSeries({
      priceScaleId: 'volume',
      priceLineVisible: false,
      lastValueVisible: false,
      priceFormat: { type: 'volume' },
    });
    instance.priceScale('volume').applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });

    chart = instance;
    series = { candles, ema50, ema200, volume };

    return () => {
      instance.remove();
      chart = null;
      series = null;
      renderedKey = '';
      levelLines = [];
    };
  });

  function clearLevels() {
    for (const line of levelLines) {
      try {
        series.candles.removePriceLine(line);
      } catch {
        // The series is gone; the chart it belonged to has been removed.
      }
    }
    levelLines = [];
  }

  /**
   * Draw the daily swings: a marker on each pivot bar plus dashed price lines at
   * the handful of levels nearest the last price, labelled R (resistance) and S
   * (support) on the price axis.
   */
  function applyPivots(bars, swings, visible) {
    clearLevels();
    if (!visible || swings.length === 0) {
      series.candles.setMarkers([]);
      return;
    }

    const markers = pivotMarkers(swings, bars.map((bar) => bar.time));
    series.candles.setMarkers(
      markers.map((pivot) => ({
        time: pivot.time,
        position: pivot.kind === 'high' ? 'aboveBar' : 'belowBar',
        color: pivot.kind === 'high' ? COLORS.down : COLORS.up,
        shape: pivot.kind === 'high' ? 'arrowDown' : 'arrowUp',
        size: 0.8,
      })),
    );

    const lastClose = bars.length ? bars[bars.length - 1].close : null;
    for (const level of keyLevels(swings, lastClose).all) {
      // Above the price is resistance whatever swing made it: an old pivot low
      // becomes resistance once price falls under it.
      const resistance = !Number.isFinite(lastClose) || level.price > lastClose;
      const tag = resistance ? 'R' : 'S';
      levelLines.push(
        series.candles.createPriceLine({
          price: level.price,
          color: resistance ? 'rgba(234, 57, 67, 0.55)' : 'rgba(22, 199, 132, 0.55)',
          lineWidth: 1,
          lineStyle: LineStyle.Dashed,
          axisLabelVisible: true,
          title: level.touches > 1 ? `${tag} ×${level.touches}` : tag,
        }),
      );
    }
  }

  // Push new data whenever the parent hands us a fresh series.
  $effect(() => {
    const data = points;
    const key = resetKey;
    const precision = decimals;
    const swings = pivots;
    const visible = showPivots;

    if (!chart || !series) return;

    const format = {
      type: 'price',
      precision,
      minMove: Number(`0.${'0'.repeat(Math.max(0, precision - 1))}1`),
    };
    series.candles.applyOptions({ priceFormat: format });
    series.ema50.applyOptions({ priceFormat: format });
    series.ema200.applyOptions({ priceFormat: format });

    series.candles.setData(toCandles(data));
    series.ema50.setData(toLine(data, 'ema50'));
    series.ema200.setData(toLine(data, 'ema200'));
    series.volume.setData(toVolume(data));

    applyPivots(data, swings, visible);

    // Re-fit only when the instrument or timeframe changes, so a background
    // refresh never yanks the user's zoom away.
    if (data.length > 0 && key !== renderedKey) {
      renderedKey = key;
      chart.timeScale().fitContent();
    }
  });
</script>

<div class="chart-wrap">
  <div class="chart-mount" bind:this={mount}></div>
  {#if points.length === 0}
    <div class="chart-empty">
      <span>No candles stored yet for this timeframe.</span>
    </div>
  {/if}
</div>
