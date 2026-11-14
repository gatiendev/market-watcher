#!/usr/bin/env bash
# End-to-end check against a running instance.
# Usage: scripts/smoke.sh [http://127.0.0.1:8791]
set -euo pipefail

BASE="${1:-http://127.0.0.1:8791}"

python3 - "$BASE" <<'PY'
import json
import sys
import urllib.request

BASE = sys.argv[1].rstrip('/')
failures = []


def get(path, method="GET"):
    req = urllib.request.Request(BASE + path, method=method, headers={"accept": "application/json"})
    with urllib.request.urlopen(req, timeout=90) as response:
        return response.status, json.loads(response.read() or b"{}")


def check(condition, message):
    if not condition:
        failures.append(message)
    print(("  PASS  " if condition else "  FAIL  ") + message)


status, health = get("/api/health")
check(status == 200 and health.get("status") == "ok", "health endpoint is ok")

status, markets = get("/api/markets")
symbols = markets.get("symbols", [])
check([row.get("symbol") for row in symbols] == ["SOLUSDT", "BTCUSDT", "EURUSD", "ES1"],
      "watchlist exposes the four symbols in order")

print(f"\n{'symbol':10} {'tf':3} {'bars':>6} {'last close':>12} {'ema50':>12} {'ema200':>12}  trend")
for row in symbols:
    for tf in ("H4", "D1"):
        snap = row["timeframes"][tf]
        check(snap["bars"] > 0, f"{row['symbol']} {tf} has cached bars")
        check(snap["last_close"] is not None, f"{row['symbol']} {tf} has a price")
        check(snap["ema50"] is not None, f"{row['symbol']} {tf} has a 50 EMA")
        check(snap["ema200"] is not None, f"{row['symbol']} {tf} has a 200 EMA")
        check(snap["status"] == "ok", f"{row['symbol']} {tf} last sync ok ({snap['status']})")

        fmt = lambda value: "—" if value is None else f"{value:,.{row['decimals']}f}"
        print(f"{row['symbol']:10} {tf:3} {snap['bars']:>6} {fmt(snap['last_close']):>12} "
              f"{fmt(snap['ema50']):>12} {fmt(snap['ema200']):>12}  {snap['trend']}")

        status, candles = get(f"/api/candles?symbol={row['symbol']}&timeframe={tf}&limit=260")
        points = candles.get("points", [])
        check(status == 200 and len(points) >= 200, f"{row['symbol']} {tf} serves >= 200 points")
        check(points and points[-1]["ema200"] is not None, f"{row['symbol']} {tf} last point carries both EMAs")
        times = [point["time"] for point in points]
        check(times == sorted(times), f"{row['symbol']} {tf} points are chronological")

status, refreshed = get("/api/refresh", method="POST")
reports = refreshed.get("reports", [])
check(len(reports) == 8, "refresh touched 8 series")
check(all(report["status"] == "ok" for report in reports), "refresh reported no errors")

print()
if failures:
    print(f"{len(failures)} check(s) failed")
    sys.exit(1)
print("all checks passed")
PY
