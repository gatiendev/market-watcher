# Market Watcher

A small trading dashboard for four instruments — **SOLUSDT**, **BTCUSDT**, **EURUSD** and **ES1**
(S&P 500 E-mini future) — showing H4 and D1 candles with the **50 EMA** and **200 EMA** on top.

Rust backend (fetches, stores, computes) + Svelte frontend (watchlist + chart), dark trading theme.

## How it works

```
Binance  ─┐                      ┌─ SQLite (candles + EMA + sync state)
Yahoo ────┴─► Rust (axum) ──────►│
                                  └─► JSON API ─► Svelte UI (lightweight-charts)
```

- **Crypto** (SOLUSDT, BTCUSDT): native `4h` and `1d` klines from Binance.
- **FX / index** (EURUSD, ES1): Yahoo Finance serves `1h` and `1d` bars; 4h candles are
  resampled from the hourly stream, aligned to the UTC 4-hour grid.
- **Caching**: every bar is stored once per `(symbol, timeframe, open_time)`. A refresh asks the
  provider only for bars at/after the newest stored one, so restarts and 5-minute polling never
  re-download history. The newest bar is re-fetched each time because it is still forming.
- **History depth**: the H4 series is backfilled until it reaches the oldest daily bar, so both
  charts show the same period. Backfill walks backwards one provider page at a time, skips
  everything already stored, and stops when the provider runs out (Yahoo caps hourly data at
  ~730 days, so EURUSD/ES1 H4 cannot go further back than that). Binance symbols reach the full
  daily span, ~2.7 years of 4h bars.
- **EMAs**: computed over the full stored series (seed = SMA of the first *N* closes, then the
  standard recursion), persisted per bar and returned with every candle. A 200 EMA stays `null`
  until 200 bars exist. The watchlist shows both the H4 and D1 trend (50 EMA vs 200 EMA) at once,
  each with how long that trend has been running (`trend_since` = first bar after the last flip).

A background loop refreshes all 8 series every `REFRESH_SECS` (default 300s); the UI also polls the
API every 60s and has a manual **Refresh** button that hits the providers immediately.

## API

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/api/health` | liveness + database check |
| GET | `/api/markets` | watchlist rows: last price, change %, EMA50/EMA200, trend + where it started (`trend_since`, `trend_bars`), freshness, per timeframe |
| GET | `/api/candles?symbol=SOLUSDT&timeframe=H4&limit=400` | candles + EMAs for the chart |
| GET | `/api/pivots?symbol=SOLUSDT&lookback=15` | daily swing highs/lows, `left`/`right` optional |
| GET | `/api/crossings[?symbol=SOLUSDT&timeframe=D1]` | every past EMA 50/200 cross with the performance of the run it opened |
| POST | `/api/refresh[?symbol=ES1]` | force a download now |

`timeframe` accepts `H4`/`4h`/`D1`/`1d`. Unknown symbols return 404, unknown timeframes 400.

## Crossings page

`/crossings` opens with **Right now**: one block per asset showing the trend in force on H4 *and* D1
at once (direction, age in days, the price at the crossing and the latest price, the price now against
the crossing close, and the best price the run reached), whatever the toggle is set to. The latest
price is rebuilt from the crossing close and the run's result, so it always agrees with the figure
next to it. On a down run the best price is the low, so the figure is
named `trough`; on an up run it is the `peak`. Below it, the page lists every past EMA 50 / EMA 200
flip on H4 or D1 (the header toggle picks the table), newest crossing first across every asset, with
what price did inside the run that cross opened: the highest and lowest price, the swing between
them, how long the best price took, and the run's result. A run lasts until the opposite crossing;
the newest one is marked **open**.

Every figure is price against the crossing close, never direction-relative, so a down run reads
negative the way the price actually moved. The swing is the trade a user could actually have taken,
in the order the market made it: **trough → peak** on an up run (buy the dip, sell the high) and
**peak → trough** on a down run (sell the high, cover the low). It is never the other way round.

The summary line above the table is computed from the rows on screen: how many crossings, the up and
down split, average high and low, median days to the best price, average swing, and how many runs are
still open. On a phone each crossing is a card instead of a table row (the same figures, stacked),
shown 25 at a time behind a "show more" button, and the asset chips scroll sideways on one line. The
header never widens the page: the brand and the way back share a row, the toggle and the chips wrap
under them, and a table too wide for the screen scrolls inside its own box.

## Run it

Production-style stack (frontend on `127.0.0.1:8790`):

```bash
docker compose up -d --build
docker compose logs -f backend
scripts/smoke.sh http://127.0.0.1:8791   # end-to-end check of all 8 series
```

Development (hot reload, Vite on `127.0.0.1:5173`, API on `127.0.0.1:8791`):

```bash
docker compose -p market-watcher_dev -f docker-compose.dev.yml up -d
```

## Tests

```bash
scripts/test.sh          # Rust suite in Docker + Vitest
```

- Rust unit tests cover the EMA maths, resampling (including weekend gaps and partial bars),
  Binance/Yahoo payload parsing, candle validation and URL construction.
- Rust integration tests cover storage (dedupe, upsert of a forming bar, isolation per
  symbol/timeframe, indicator join), sync behaviour (incremental cursor, H4 backfill down to the
  daily history, no backfill once complete, provider failure keeps old data, recovery) and the
  HTTP contract (status codes, payload shape, refresh flow, trend age) — 67 tests.
- Frontend tests cover number/time formatting, trend arrows, chart data mapping and the API
  client's error handling and trend age labels — 30 tests.

## Layout

```
backend/src/models.rs      symbols, timeframes, candles
backend/src/indicators.rs  EMA series (50 / 200)
backend/src/aggregate.rs   1h -> 4h resampling
backend/src/providers/     Binance + Yahoo, retries, parsing
backend/src/db.rs          SQLite schema and queries
backend/src/ingest.rs      fetch -> store -> recompute EMAs -> record status
backend/src/api.rs         axum routes
frontend/src/App.svelte    shell, polling, selection
frontend/src/lib/          watchlist table, chart, API client, formatters
```

## Deployment

Live at **https://trading.gatiendevictor.com** (loopback origin `127.0.0.1:8790`, fronted by
nginx-proxy-manager which reaches `market-watcher-frontend` over the `gatien_app-network` network).

```bash
docker compose up -d --build          # build + start both containers
docker compose ps                     # backend should report (healthy)
docker compose logs -f backend        # sync log, one line per series
```

Redeploying a frontend-only change: `docker compose up -d --build --no-deps frontend`.

## Operations

- Database lives in the `market-data` volume (`/data/market.db`, WAL mode).
- `POST /api/refresh` is also what the UI button calls; concurrent calls queue instead of
  hammering the providers.
- If a provider fails the series keeps its stored bars, the failure is recorded in `sync_state`
  and surfaced in the UI footer — nothing is wiped.
- Symbols and the provider used for each one are defined in `backend/src/models.rs` (`SYMBOLS`).
