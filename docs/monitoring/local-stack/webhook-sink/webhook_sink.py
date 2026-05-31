#!/usr/bin/env python3
"""V2G-L0 local webhook sink.

Receives Alertmanager webhook dispatches on 0.0.0.0:9095 and appends
each one to /var/log/sink/received.log as a JSON-lines log. Six paths
correspond to the V2G-J receivers: /default, /critical, /high,
/tickets, /ops, /backend.

NOT FOR PRODUCTION. Plain HTTP, no auth, no rate limiting, no TLS.
The container ships behind a Docker network and is reachable only
from other compose services.
"""
import datetime
import http.server
import json
import os
import sys


LOG = os.environ.get("SINK_LOG", "/var/log/sink/received.log")


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8") if length else ""
        try:
            data = json.loads(body) if body else {}
        except json.JSONDecodeError:
            data = {"raw": body}

        receiver_path = self.path.lstrip("/")
        entry = {
            "ts": datetime.datetime.utcnow().isoformat() + "Z",
            "receiver_path": receiver_path,
            "data_receiver": data.get("receiver"),
            "status": data.get("status"),
            "alerts": [
                {
                    "labels": a.get("labels"),
                    "status": a.get("status"),
                }
                for a in (data.get("alerts") or [])
            ],
        }

        os.makedirs(os.path.dirname(LOG), exist_ok=True)
        with open(LOG, "a") as fh:
            fh.write(json.dumps(entry) + "\n")
            fh.flush()

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, *_args, **_kwargs) -> None:
        # Quiet — we already have JSON-lines in LOG.
        return


def main() -> int:
    os.makedirs(os.path.dirname(LOG), exist_ok=True)
    # Truncate on startup so the log only contains current-session
    # dispatches (the volume keeps the file around across restarts).
    open(LOG, "w").close()

    bind_host = os.environ.get("SINK_HOST", "0.0.0.0")
    bind_port = int(os.environ.get("SINK_PORT", "9095"))
    print(f"sink listening on {bind_host}:{bind_port}, log={LOG}", flush=True)

    server = http.server.HTTPServer((bind_host, bind_port), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("shutting down", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
