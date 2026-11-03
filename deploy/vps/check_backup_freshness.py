#!/usr/bin/env python3
"""Fail when the last verified offsite backup is more than 36 hours old."""

from __future__ import annotations

import os
from pathlib import Path
import re
import stat
import sys
import time


LAST_SUCCESS = Path("/var/lib/deopt-backup/last-success")
MAX_AGE_SECONDS = 36 * 3600


def is_fresh(path: Path, now: float, expected_uid: int = 0) -> bool:
    file_stat = path.lstat()
    if not stat.S_ISREG(file_stat.st_mode) or file_stat.st_uid != expected_uid or stat.S_IMODE(file_stat.st_mode) != 0o600:
        return False
    if file_stat.st_size > 128:
        return False
    data = path.read_text(encoding="ascii").strip()
    if not re.fullmatch(r"[0-9]{10} [0-9a-f]{32}", data):
        return False
    timestamp = int(data.split(" ", 1)[0])
    return 1_577_836_800 <= timestamp <= now + 300 and now - timestamp <= MAX_AGE_SECONDS


def main() -> int:
    if os.geteuid() != 0:
        print("BACKUP_FRESHNESS=REFUSED_NONROOT", file=sys.stderr)
        return 2
    try:
        fresh = is_fresh(LAST_SUCCESS, time.time())
    except (OSError, ValueError, UnicodeError):
        fresh = False
    print("BACKUP_FRESHNESS=PASS" if fresh else "BACKUP_FRESHNESS=FAIL")
    return 0 if fresh else 1


if __name__ == "__main__":
    raise SystemExit(main())
