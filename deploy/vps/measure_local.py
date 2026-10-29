"""Bounded local HTTP/CPU/RSS sample. Never sends economic requests."""

import argparse
import json
import pathlib
import statistics
import time
import urllib.error
import urllib.request


def process_sample(pid):
    status = (pathlib.Path("/proc") / str(pid) / "status").read_text()
    rss_kib = int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:")))
    stat = (pathlib.Path("/proc") / str(pid) / "stat").read_text().rsplit(") ", 1)[1].split()
    ticks = int(stat[11]) + int(stat[12])
    return rss_kib, ticks


def request(url, method="GET"):
    started = time.monotonic()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, method=method), timeout=3) as response:
            status = response.status
            response.read(1024)
    except urllib.error.HTTPError as error:
        status = error.code
        error.read(1024)
    return status, (time.monotonic() - started) * 1000


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:18080")
    parser.add_argument("--pid", required=True, type=int)
    parser.add_argument("--seconds", type=int, default=10)
    args = parser.parse_args()
    if not (1 <= args.seconds <= 60) or not args.url.startswith("http://127.0.0.1:"):
        parser.error("local loopback URL and 1..60 seconds required")
    before_rss, before_ticks = process_sample(args.pid)
    started = time.monotonic()
    latencies = []
    peak_rss = before_rss
    counts = {}
    while time.monotonic() - started < args.seconds:
        status, latency = request(args.url + "/health")
        counts[str(status)] = counts.get(str(status), 0) + 1
        latencies.append(latency)
        peak_rss = max(peak_rss, process_sample(args.pid)[0])
        time.sleep(0.1)
    after_rss, after_ticks = process_sample(args.pid)
    probes = {}
    for path, method in [('/ready', 'GET'), ('/perps/markets', 'GET'),
                         ('/accounts/0x0000000000000000000000000000000000000001/perps/canonical-fees', 'GET'),
                         ('/perps/orders', 'POST')]:
        probes[method + ' ' + path] = request(args.url + path, method)[0]
    print(json.dumps({"duration_seconds": round(time.monotonic() - started, 2),
                      "health_status_counts": counts, "request_count": len(latencies),
                      "latency_ms_p50": round(statistics.median(latencies), 2),
                      "latency_ms_p95": round(sorted(latencies)[max(0, int(len(latencies) * .95) - 1)], 2),
                      "backend_rss_mib_start": round(before_rss / 1024, 2),
                      "backend_rss_mib_end": round(after_rss / 1024, 2),
                      "backend_rss_mib_peak": round(peak_rss / 1024, 2),
                      "backend_cpu_seconds": round((after_ticks - before_ticks) / 100, 3),
                      "probes": probes}, sort_keys=True))


if __name__ == "__main__":
    main()
