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
- **EMAs**: computed over the full stored series (seed = SMA of the first *N* closes, then the
  standard recursion), persisted per bar and returned with every candle. A 200 EMA stays `null`
  until 200 bars exist.

A background loop refreshes all 8 series every `REFRESH_SECS` (default 300s); the UI also polls the
API every 60s and has a manual **Refresh** button that hits the providers immediately.

## API

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/api/health` | liveness + database check |
| GET | `/api/markets` | watchlist rows: last price, change %, EMA50/EMA200, trend, freshness, per timeframe |
| GET | `/api/candles?symbol=SOLUSDT&timeframe=H4&limit=400` | candles + EMAs for the chart |
| POST | `/api/refresh[?symbol=ES1]` | force a download now |

`timeframe` accepts `H4`/`4h`/`D1`/`1d`. Unknown symbols return 404, unknown timeframes 400.

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
  symbol/timeframe, indicator join), sync behaviour (incremental cursor, provider failure keeps
  old data, recovery) and the HTTP contract (status codes, payload shape, refresh flow) — 59 tests.
- Frontend tests cover number/time formatting, chart data mapping and the API client's error
  handling — 25 tests.

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

## Operations

- Database lives in the `market-data` volume (`/data/market.db`, WAL mode).
- `POST /api/refresh` is also what the UI button calls; concurrent calls queue instead of
  hammering the providers.
- If a provider fails the series keeps its stored bars, the failure is recorded in `sync_state`
  and surfaced in the UI footer — nothing is wiped.
- Symbols and the provider used for each one are defined in `backend/src/models.rs` (`SYMBOLS`).
