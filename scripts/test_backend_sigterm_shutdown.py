#!/usr/bin/env python3
"""Local-only SIGTERM regression for the backend's container lifecycle."""

import http.client
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import time


def main() -> int:
    binary = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/deopt-v2-backend")
    binary = binary.resolve(strict=True)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOST": "127.0.0.1",
        "PORT": str(port),
        "CHAIN_ID": "84532",
        "PERSISTENCE_ENABLED": "false",
        "EXECUTION_ENABLED": "false",
        "EXECUTOR_REAL_BROADCAST_ENABLED": "false",
        "HV2_BROADCAST_ENABLED": "false",
        "PERPS_PUBLIC_TRADING_ENABLED": "false",
        "PERPS_CLOSED_TEST_ENABLED": "false",
        "PERPS_READ_ENABLED": "false",
        "OPTIONS_ENABLED": "false",
        "RUST_LOG": "warn",
    }
    with tempfile.TemporaryDirectory(prefix="deopt-sigterm-") as workdir:
        with open(os.path.join(workdir, "backend.log"), "w", encoding="utf-8") as log:
            process = subprocess.Popen(
                [str(binary)], cwd=workdir, env=env, stdout=log, stderr=subprocess.STDOUT
            )
            try:
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError("backend exited before HTTP readiness")
                    try:
                        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=1)
                        conn.request("GET", "/health")
                        response = conn.getresponse()
                        response.read()
                        conn.close()
                        if response.status == 200:
                            break
                    except (OSError, http.client.HTTPException):
                        time.sleep(0.1)
                else:
                    raise AssertionError("backend did not reach HTTP readiness")

                started = time.monotonic()
                process.send_signal(signal.SIGTERM)
                try:
                    code = process.wait(timeout=12)
                except subprocess.TimeoutExpired as exc:
                    raise AssertionError("backend did not exit after SIGTERM") from exc
                elapsed = time.monotonic() - started
                if code != 0:
                    raise AssertionError(f"backend SIGTERM exit code {code}")
                print(f"PASS: SIGTERM exit=0, elapsed={elapsed:.2f}s, local-only")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
    return 0


if __name__ == "__main__":
    sys.exit(main())
