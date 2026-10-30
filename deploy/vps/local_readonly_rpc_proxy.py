"""Loopback-only read RPC for disposable Anvil forks; never forwards writes.

The upstream URL is read from an existing local env file. It is never printed
or passed to Anvil as a command-line argument. This is test tooling, not a
hosted proxy or an application RPC gateway.
"""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

READ_METHODS = frozenset({
    "eth_chainId", "eth_blockNumber", "eth_getBlockByNumber", "eth_getBlockByHash",
    "eth_getTransactionByHash", "eth_getTransactionReceipt", "eth_getBlockReceipts",
    "eth_getLogs", "eth_getCode", "eth_getBalance", "eth_getStorageAt",
    "eth_getProof", "eth_call", "eth_estimateGas", "eth_getTransactionCount",
    "eth_feeHistory", "eth_gasPrice", "eth_maxPriorityFeePerGas", "eth_syncing",
    "net_version", "web3_clientVersion",
})
MAX_REQUEST = 1024 * 1024
MAX_RESPONSE = 16 * 1024 * 1024


def read_upstream(path: Path) -> str:
    value = None
    with path.open(encoding="utf-8") as source:
        for line in source:
            if line.startswith("RPC_URL="):
                value = line.partition("=")[2].strip().strip("\"'")
                break
    if not value:
        raise ValueError("RPC_URL missing")
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("HTTPS RPC_URL required")
    return value


def handler_for(upstream: str):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not (0 < size <= MAX_REQUEST):
                    self.send_error(413)
                    return
                payload = self.rfile.read(size)
                request = json.loads(payload)
                if not isinstance(request, dict) or request.get("method") not in READ_METHODS:
                    self.send_error(403)
                    return
                with urlopen(Request(upstream, data=payload, headers={"Content-Type": "application/json"}), timeout=30) as response:
                    body = response.read(MAX_RESPONSE + 1)
                if len(body) > MAX_RESPONSE:
                    self.send_error(502)
                    return
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except (ValueError, KeyError, json.JSONDecodeError):
                self.send_error(400)
            except (HTTPError, URLError, TimeoutError, OSError):
                # Never forward the provider's response body or URL.
                self.send_error(502)

    return Handler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--env-file", required=True, type=Path)
    parser.add_argument("--port", type=int, default=18547)
    args = parser.parse_args()
    if not (1024 <= args.port <= 65535):
        parser.error("unprivileged port required")
    upstream = read_upstream(args.env_file)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler_for(upstream))
    server.daemon_threads = True
    print("LOCAL_READ_ONLY_RPC_PROXY=READY", flush=True)
    server.serve_forever(poll_interval=0.2)


if __name__ == "__main__":
    main()
