#!/usr/bin/env bash
# Encrypt with the operator's public GPG recipient before off-server upload.
# Credentials reside in a host-only PostgreSQL service file, never arguments.
set -euo pipefail
: "${DEOPT_BACKUP_RECIPIENT:?set reviewed GPG recipient fingerprint}"
: "${DEOPT_BACKUP_REMOTE:?set reviewed rclone destination}"
: "${PGSERVICEFILE:?set host-only PostgreSQL service file}"
[[ "$DEOPT_BACKUP_RECIPIENT" =~ ^[0-9A-Fa-f]{40}([0-9A-Fa-f]{24})?$ ]] || {
  echo 'Full GPG recipient fingerprint required' >&2; exit 2;
}
[[ "$DEOPT_BACKUP_REMOTE" == *:* ]] || { echo 'Off-server rclone destination required' >&2; exit 2; }
[[ -r "$PGSERVICEFILE" && "$(stat -c %a "$PGSERVICEFILE")" == 600 ]] || {
  echo 'Host-only PostgreSQL service file must be readable and mode 0600' >&2; exit 2;
}
command -v pg_dump >/dev/null
command -v gpg >/dev/null
command -v rclone >/dev/null
tmpdir="$(mktemp -d /tmp/deopt-backup.XXXXXXXX)"
trap 'rm -rf "$tmpdir"' EXIT
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
pg_dump --format=custom --no-owner --no-acl --dbname='service=deopt_readonly' \
  | gpg --batch --yes --trust-model always --encrypt --recipient "$DEOPT_BACKUP_RECIPIENT" \
      --output "$tmpdir/$stamp.dump.gpg"
test -s "$tmpdir/$stamp.dump.gpg"
rclone copyto "$tmpdir/$stamp.dump.gpg" "$DEOPT_BACKUP_REMOTE/$stamp.dump.gpg"
echo 'ENCRYPTED_OFFSERVER_BACKUP=UPLOADED; RESTORE_NOT_YET_VERIFIED'
