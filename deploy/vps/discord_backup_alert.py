#!/usr/bin/env python3
"""Send a minimal DeOpt backup alert using a systemd credential, never a CLI URL."""

from __future__ import annotations

import argparse
import datetime as dt
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import ssl
import stat
import subprocess
import sys
import tempfile
import time
import uuid


BACKUP_SERVICE = "deopt-production-backup.service"
WATCHDOG_SERVICE = "deopt-production-backup-watchdog.service"
TEST_SERVICE = "deopt-backup-failure-probe.service"
ALLOWED_SERVICES = {BACKUP_SERVICE, WATCHDOG_SERVICE, TEST_SERVICE}
WEBHOOK_PATTERN = re.compile(
    r"https://discord\.com/api/webhooks/[0-9]{15,25}/[A-Za-z0-9._-]{20,256}"
)
STATE_DIR = Path("/var/lib/deopt-backup")
PENDING_DIR = STATE_DIR / "alerts-pending"
OWNER_UID = 0


def read_webhook_credential() -> str:
    directory = os.environ.get("CREDENTIALS_DIRECTORY", "")
    if not directory.startswith("/run/credentials/"):
        raise ValueError("SYSTEMD_CREDENTIAL_DIRECTORY_REQUIRED")
    return read_webhook_file(Path(directory) / "discord_webhook")


def read_webhook_file(path: Path, expected_uid: int = 0) -> str:
    file_stat = path.lstat()
    if (not stat.S_ISREG(file_stat.st_mode) or file_stat.st_uid != expected_uid
            or file_stat.st_mode & 0o077 or file_stat.st_size > 512):
        raise ValueError("INVALID_WEBHOOK_CREDENTIAL_FILE")
    url = path.read_text(encoding="utf-8").removesuffix("\n")
    if not WEBHOOK_PATTERN.fullmatch(url):
        raise ValueError("INVALID_WEBHOOK_CREDENTIAL_FORMAT")
    return url


def execution_id(service: str) -> str:
    if service != TEST_SERVICE:
        try:
            result = subprocess.run(
                ["systemctl", "show", "--value", "--property=InvocationID", service],
                capture_output=True, text=True, check=False, timeout=3,
            )
            candidate = result.stdout.strip().lower()
            if result.returncode == 0 and candidate != "0" * 32 and re.fullmatch(r"[0-9a-f]{32}", candidate):
                return candidate
        except (OSError, subprocess.TimeoutExpired):
            pass
    return uuid.uuid4().hex


def message(service: str, when: dt.datetime, run_id: str) -> bytes:
    if service not in ALLOWED_SERVICES or not re.fullmatch(r"[0-9a-f]{32}", run_id):
        raise ValueError("INVALID_ALERT_IDENTITY")
    state = "FAIL"
    utc = when.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return message_from_fields({"service": service, "utc": utc, "state": state, "execution_id": run_id})


def message_from_fields(fields: dict[str, str]) -> bytes:
    if (not isinstance(fields, dict)
            or set(fields) != {"service", "utc", "state", "execution_id"}
            or not all(isinstance(value, str) for value in fields.values())
            or fields["service"] not in ALLOWED_SERVICES
            or fields["state"] != "FAIL"
            or not re.fullmatch(r"[0-9a-f]{32}", fields["execution_id"])
            or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", fields["utc"])):
        raise ValueError("INVALID_ALERT_FIELDS")
    content = "\n".join(f"{name}={fields[name]}" for name in ("service", "utc", "state", "execution_id"))
    return json.dumps(
        {"content": content, "allowed_mentions": {"parse": []}},
        separators=(",", ":"),
    ).encode("utf-8")


def post_alert(url: str, body: bytes, *, connect=http.client.HTTPSConnection, sleep=time.sleep) -> bool:
    if not WEBHOOK_PATTERN.fullmatch(url):
        raise ValueError("INVALID_WEBHOOK_CREDENTIAL_FORMAT")
    path = url.removeprefix("https://discord.com") + "?wait=true"
    for attempt in range(3):
        connection = None
        try:
            connection = connect("discord.com", timeout=8, context=ssl.create_default_context())
            connection.request(
                "POST", path, body=body,
                headers={"Content-Type": "application/json", "User-Agent": "DeOptBackup/1"},
            )
            response = connection.getresponse()
            response_body = response.read(65537)
            if response.status == 200 and len(response_body) <= 65536:
                result = json.loads(response_body)
                return bool(re.fullmatch(r"[0-9]{15,25}", str(result.get("id", ""))))
            if response.status not in {429, 500, 502, 503, 504}:
                return False
            delay = min(5.0, max(0.5, float(response.getheader("Retry-After") or 2 ** attempt)))
        except (OSError, TimeoutError, ValueError, json.JSONDecodeError, http.client.HTTPException):
            delay = min(5.0, float(2 ** attempt))
        finally:
            if connection is not None:
                connection.close()
        if attempt < 2:
            sleep(delay)
    return False


def watchdog_incident_key() -> str:
    source = STATE_DIR / "last-success"
    raw = source.read_bytes() if source.exists() else b"missing"
    return hashlib.sha256(raw).hexdigest()


def write_marker(path: Path, value: str) -> None:
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=".alert-", delete=False) as file:
        name = file.name
        file.write(value + "\n")
        file.flush()
        os.fsync(file.fileno())
    os.chmod(name, 0o600)
    os.replace(name, path)


def ensure_private_directory(path: Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != OWNER_UID or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError("ALERT_STATE_DIRECTORY_UNSAFE")


def read_pending(path: Path) -> dict[str, str]:
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != OWNER_UID or stat.S_IMODE(info.st_mode) != 0o600 or info.st_size > 512:
        raise ValueError("PENDING_ALERT_FILE_UNSAFE")
    fields = json.loads(path.read_text(encoding="utf-8"))
    message_from_fields(fields)
    return fields


def mark_delivered(service: str, key: str) -> None:
    if service == WATCHDOG_SERVICE:
        write_marker(STATE_DIR / "last-watchdog-alert-key", key)
    if service == TEST_SERVICE:
        write_marker(Path("/run/deopt-backup-alert-probe-success"), key)


def retry_pending(url: str) -> tuple[int, int]:
    paths = sorted(PENDING_DIR.glob("*.json"))[:5]
    delivered = 0
    failed = 0
    for path in paths:
        try:
            fields = read_pending(path)
            if fields["service"] == BACKUP_SERVICE and path.name != f"backup-{fields['execution_id']}.json":
                raise ValueError("PENDING_ALERT_IDENTITY_CONFLICT")
            if fields["service"] == WATCHDOG_SERVICE and not re.fullmatch(r"watchdog-[0-9a-f]{64}\.json", path.name):
                raise ValueError("PENDING_ALERT_IDENTITY_CONFLICT")
            if fields["service"] == TEST_SERVICE:
                raise ValueError("SYNTHETIC_ALERT_MUST_NOT_BE_QUEUED")
            if not post_alert(url, message_from_fields(fields)):
                failed += 1
                continue
            if path.name.startswith("watchdog-"):
                key = path.stem.removeprefix("watchdog-")
                if not re.fullmatch(r"[0-9a-f]{64}", key) or fields["service"] != WATCHDOG_SERVICE:
                    raise ValueError("PENDING_ALERT_IDENTITY_CONFLICT")
                mark_delivered(WATCHDOG_SERVICE, key)
            path.unlink()
            delivered += 1
        except (OSError, ValueError, UnicodeError, json.JSONDecodeError):
            failed += 1
    return delivered, failed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("service", nargs="?", choices=sorted(ALLOWED_SERVICES))
    parser.add_argument("--retry-pending", action="store_true")
    args = parser.parse_args()
    if args.retry_pending == (args.service is not None):
        parser.error("specify either a service or --retry-pending")
    if os.geteuid() != 0:
        print("DISCORD_ALERT=REFUSED_NONROOT", file=sys.stderr)
        return 2
    try:
        ensure_private_directory(STATE_DIR)
        ensure_private_directory(PENDING_DIR)
        with (STATE_DIR / "alerts.lock").open("a+") as lock:
            os.fchmod(lock.fileno(), 0o600)
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            if args.retry_pending:
                url = read_webhook_credential()
                delivered, failed = retry_pending(url)
                print(f"PENDING_ALERTS_DELIVERED={delivered} PENDING_ALERTS_FAILED={failed}")
                return 0 if failed == 0 else 1
            run_id = execution_id(args.service)
            key = watchdog_incident_key() if args.service == WATCHDOG_SERVICE else run_id
            if args.service == WATCHDOG_SERVICE:
                marker = STATE_DIR / "last-watchdog-alert-key"
                if marker.exists() and marker.read_text(encoding="ascii").strip() == key:
                    print("DISCORD_ALERT=ALREADY_SENT_FOR_INCIDENT")
                    return 0
            now = dt.datetime.now(dt.timezone.utc)
            fields = {"service": args.service, "utc": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
                      "state": "FAIL", "execution_id": run_id}
            pending = None
            if args.service != TEST_SERVICE:
                prefix = "watchdog" if args.service == WATCHDOG_SERVICE else "backup"
                pending = PENDING_DIR / f"{prefix}-{key}.json"
                if pending.exists():
                    fields = read_pending(pending)
                else:
                    write_marker(pending, json.dumps(fields, separators=(",", ":")))
            url = read_webhook_credential()
            if not post_alert(url, message_from_fields(fields)):
                raise ValueError("DELIVERY_NOT_CONFIRMED")
            mark_delivered(args.service, key)
            if pending is not None:
                pending.unlink()
    except (OSError, ValueError, UnicodeError, json.JSONDecodeError):
        print("DISCORD_ALERT=FAILED", file=sys.stderr)
        return 1
    print(f"DISCORD_ALERT=DELIVERED SERVICE={args.service} EXECUTION_ID={run_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
