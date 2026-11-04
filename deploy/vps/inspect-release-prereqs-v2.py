#!/usr/bin/env python3
"""Read-only, redacted production database and R2 preflight for the reviewed VPS."""

import json
import os
import subprocess
import sys

PROJECT = "dxco90jwpdxyennmquv2elyj"
HOST = "v2202611448349549977"


def run(args: list[str], timeout: int = 20) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def main() -> int:
    if os.geteuid() != 0 or run(["hostname", "-s"]).stdout.strip() != HOST:
        print("REVIEWED_ROOT_HOST_REQUIRED")
        return 2
    os.environ["RCLONE_CONFIG"] = "/root/.config/deopt/rclone.conf"
    ids = run(["docker", "ps", "--filter", f"label=com.docker.compose.project={PROJECT}",
               "--filter", "label=com.docker.compose.service=postgres", "--format", "{{.ID}}"])
    containers = ids.stdout.splitlines() if ids.returncode == 0 else []
    if len(containers) != 1:
        print("POSTGRES_CONTAINER=INCONCLUSIVE")
        return 2
    query = ("export PGPASSWORD=\"$(cat /run/secrets/postgres_password)\"; "
             "psql -h /var/run/postgresql -U deopt_readonly -d deopt_readonly -Atqc "
             "\"select 'ROLE',rolsuper::text from pg_roles where rolname=current_user; "
             "select 'LOCAL_HBA',auth_method from pg_hba_file_rules where type='local';\"")
    db = run(["docker", "exec", containers[0], "sh", "-ec", query])
    if db.returncode == 0:
        for line in db.stdout.splitlines():
            kind, sep, value = line.partition("|")
            if sep and kind == "ROLE" and value in ("true", "false", "t", "f"):
                print("DB_APP_ROLE_SUPERUSER=" + ("YES" if value in ("true", "t") else "NO"))
            elif sep and kind == "LOCAL_HBA" and value.isascii() and value.replace("-", "").isalnum():
                print("POSTGRES_LOCAL_HBA=" + value)
    else:
        print("POSTGRES_AUTH_INSPECTION=INCONCLUSIVE")
    service = run(["systemctl", "show", "deopt-production-backup.service", "-p", "Result",
                   "-p", "ExecMainStatus", "-p", "ExecMainStartTimestamp"])
    if service.returncode == 0:
        for line in service.stdout.splitlines():
            if line.startswith(("Result=", "ExecMainStatus=", "ExecMainStartTimestamp=")):
                print("BACKUP_" + line)
    objects = run(["rclone", "lsjson", "deopt_r2:deopt-private-backups/postgres",
                   "--max-depth", "1"], timeout=35)
    if objects.returncode != 0:
        print("R2_OBJECT_INSPECTION=INCONCLUSIVE")
        return 0
    try:
        ciphertexts = [item for item in json.loads(objects.stdout)
                       if isinstance(item, dict) and not item.get("IsDir")
                       and str(item.get("Path", "")).endswith(".dump.gpg")]
        latest = max(ciphertexts, key=lambda item: item.get("ModTime", ""))
        print("R2_LATEST_CIPHERTEXT_SIZE=" + str(int(latest["Size"])))
        print("R2_LATEST_CIPHERTEXT_MODIFIED=" + latest["ModTime"])
        print("R2_CIPHERTEXT_COUNT=" + str(len(ciphertexts)))
    except (ValueError, KeyError) as exc:
        print("R2_OBJECT_INSPECTION=INCONCLUSIVE_" + type(exc).__name__)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, subprocess.TimeoutExpired) as exc:
        print("RELEASE_PREFLIGHT=INCONCLUSIVE_" + type(exc).__name__)
        sys.exit(2)
