"""Bounded local fee-history read sample over genuine Anvil-ingested wallets.

Run only against the loopback development backend. The VPS release profile
continues to block this unauthenticated route until caller auth is reviewed.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from glob import glob
import json
import os
from pathlib import Path
import statistics
import time
from urllib.request import urlopen


def process_sample(pid):
    status = (Path("/proc") / str(pid) / "status").read_text()
    fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
    rss_kib = int(fields["VmRSS"].split()[0])
    hwm_kib = int(fields["VmHWM"].split()[0])
    stat = (Path("/proc") / str(pid) / "stat").read_text().rsplit(") ", 1)[1].split()
    ticks = int(stat[11]) + int(stat[12])
    return rss_kib, hwm_kib, ticks


def fixture_wallets(pattern):
    paths = sorted(glob(pattern))
    if not paths or len(paths) > 20:
        raise ValueError("1..20 local fixture files required")
    wallets = []
    for path in paths:
        if not path.startswith("/tmp/deopt_vps_capacity_fixture_") or not path.endswith(".json"):
            raise ValueError("only disposable capacity fixtures under /tmp accepted")
        with open(path, encoding="utf-8") as source:
            data = json.load(source)
        if data.get("receipt", {}).get("status") != "0x1" or not data.get("trace"):
            raise ValueError("receipt and trace required")
        wallets.extend(data["fixture"]["buyers"] + data["fixture"]["sellers"])
    if any(len(wallet) != 42 or not wallet.startswith("0x") for wallet in wallets):
        raise ValueError("invalid fixture wallet")
    return paths, sorted(set(wallets))


def one_read(base, wallet):
    started = time.monotonic()
    with urlopen(f"{base}/accounts/{wallet}/perps/canonical-fees?limit=100", timeout=5) as response:
        if response.status != 200:
            raise ValueError("history HTTP status changed")
        body = json.load(response)
    if body.get("chain_id") != 84532 or body.get("identity_scope") != "wallet" or body.get("trading_enabled") is not False:
        raise ValueError("history identity or trading gate changed")
    rows = body.get("fees")
    if not isinstance(rows, list) or not rows:
        raise ValueError("missing persisted fixture history")
    if any(row.get("wallet", "").lower() != wallet.lower() or row.get("subaccount_id") is not None for row in rows):
        raise ValueError("cross-wallet or unproven subaccount attribution")
    return (time.monotonic() - started) * 1000, len(rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:4010")
    parser.add_argument("--fixture-glob", default="/tmp/deopt_vps_capacity_fixture_*.json")
    parser.add_argument("--requests", type=int, default=128)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--backend-pid", type=int, required=True)
    args = parser.parse_args()
    if not args.url.startswith("http://127.0.0.1:") or not (1 <= args.requests <= 500) or not (1 <= args.workers <= 16):
        parser.error("loopback URL, 1..500 requests and 1..16 workers required")
    paths, wallets = fixture_wallets(args.fixture_glob)
    before_rss, _, before_ticks = process_sample(args.backend_pid)
    started = time.monotonic()
    latencies = []
    row_counts = []
    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = [executor.submit(one_read, args.url, wallets[index % len(wallets)]) for index in range(args.requests)]
        for future in as_completed(futures):
            latency, count = future.result()
            latencies.append(latency)
            row_counts.append(count)
    duration = time.monotonic() - started
    after_rss, hwm, after_ticks = process_sample(args.backend_pid)
    ticks_per_second = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
    print(json.dumps({
        "fixture_transactions": len(paths),
        "distinct_wallets": len(wallets),
        "requests": len(latencies),
        "workers": args.workers,
        "duration_seconds": round(duration, 3),
        "requests_per_second": round(len(latencies) / duration, 2),
        "latency_p50_ms": round(statistics.median(latencies), 2),
        "latency_p95_ms": round(sorted(latencies)[max(0, int(len(latencies) * 0.95) - 1)], 2),
        "min_rows_per_wallet_response": min(row_counts),
        "max_rows_per_wallet_response": max(row_counts),
        "cross_wallet_leaks": 0,
        "backend_rss_start_mib": round(before_rss / 1024, 2),
        "backend_rss_end_mib": round(after_rss / 1024, 2),
        "backend_rss_high_water_mib": round(hwm / 1024, 2),
        "backend_cpu_seconds": round((after_ticks - before_ticks) / ticks_per_second, 3),
    }, sort_keys=True))


if __name__ == "__main__":
    main()
