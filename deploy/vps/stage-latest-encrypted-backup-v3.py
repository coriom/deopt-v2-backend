#!/usr/bin/env python3
"""Stage only the latest encrypted DeOpt dump for an isolated WSL restore."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

REMOTE = "deopt_r2:deopt-private-backups/postgres"
DEST = Path("/home/deoptops/deopt-release.stage/real-backup-v3.dump.gpg")


def run(*args: str, timeout: int = 45) -> bytes:
    return subprocess.run(args, capture_output=True, check=True, timeout=timeout).stdout


def main() -> int:
    if os.geteuid() != 0 or run("hostname", "-s").strip() != b"v2202611448349549977":
        return 2
    os.environ["RCLONE_CONFIG"] = "/root/.config/deopt/rclone.conf"
    listing = json.loads(run("rclone", "lsjson", REMOTE, "--max-depth", "1"))
    candidates = [item for item in listing if isinstance(item, dict)
                  and re.fullmatch(r"20[0-9]{6}T[0-9]{6}Z-[0-9a-f]{32}\.dump\.gpg",
                                   str(item.get("Path", "")))
                  and not item.get("IsDir") and int(item.get("Size", 0)) > 0]
    if not candidates or DEST.exists() or DEST.is_symlink():
        print("CIPHERTEXT_STAGE=BLOCKED")
        return 2
    latest = max(candidates, key=lambda item: item["ModTime"])
    owner = DEST.parent.stat()
    if (owner.st_uid, owner.st_mode & 0o777) != (1000, 0o700):
        print("STAGE_DIRECTORY_UNSAFE")
        return 2
    source = REMOTE + "/" + latest["Path"]
    temporary = DEST.with_suffix(".partial")
    if temporary.exists() or temporary.is_symlink():
        return 2
    try:
        run("rclone", "copyto", source, str(temporary), timeout=90)
        digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
        remote_digest = hashlib.sha256(run("rclone", "cat", source, timeout=90)).hexdigest()
        if digest != remote_digest or temporary.stat().st_size != int(latest["Size"]):
            print("CIPHERTEXT_INTEGRITY=FAIL")
            return 2
        os.chown(temporary, 1000, 1000)
        temporary.chmod(0o600)
        temporary.rename(DEST)
        print("CIPHERTEXT_STAGE=PASS")
        print("CIPHERTEXT_SHA256=" + digest)
        print("CIPHERTEXT_SIZE=" + str(DEST.stat().st_size))
        print("CIPHERTEXT_PATH=" + str(DEST))
        return 0
    finally:
        if temporary.exists():
            temporary.unlink()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        print("CIPHERTEXT_STAGE=INCONCLUSIVE")
        sys.exit(2)
