#!/usr/bin/env bash
# Host-operated backup: dump through private Compose PostgreSQL, encrypt before
# off-server upload. No PostgreSQL host port or container Docker socket needed.
set -euo pipefail
: "${DEOPT_BACKUP_RECIPIENT:?set reviewed GPG recipient fingerprint}"
: "${DEOPT_BACKUP_REMOTE:?set reviewed rclone destination}"
: "${DEOPT_COMPOSE_FILE:?set absolute path to reviewed Compose profile}"
: "${DEOPT_RELEASE_ENV_FILE:?set absolute path to host-only release.env}"
[[ "$DEOPT_BACKUP_RECIPIENT" =~ ^[0-9A-Fa-f]{40}([0-9A-Fa-f]{24})?$ ]] || {
  echo 'Full GPG recipient fingerprint required' >&2; exit 2;
}
[[ "$DEOPT_BACKUP_REMOTE" == *:* ]] || { echo 'Off-server rclone destination required' >&2; exit 2; }
[[ "$DEOPT_COMPOSE_FILE" == /* && -r "$DEOPT_COMPOSE_FILE" ]] || {
  echo 'Reviewed absolute Compose file required' >&2; exit 2;
}
[[ "$DEOPT_RELEASE_ENV_FILE" == /* && -r "$DEOPT_RELEASE_ENV_FILE" && "$(stat -c %a "$DEOPT_RELEASE_ENV_FILE")" == 600 ]] || {
  echo 'Host-only release.env must be readable and mode 0600' >&2; exit 2;
}
command -v docker >/dev/null
command -v gpg >/dev/null
command -v rclone >/dev/null
tmpdir="$(mktemp -d /tmp/deopt-backup.XXXXXXXX)"
on_exit() {
  result=$?
  rm -rf -- "$tmpdir"
  if [[ "$result" != 0 ]]; then echo 'ENCRYPTED_OFFSERVER_BACKUP=FAILED' >&2; fi
}
trap on_exit EXIT
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
docker compose --env-file "$DEOPT_RELEASE_ENV_FILE" -f "$DEOPT_COMPOSE_FILE" exec -T postgres \
  pg_dump --format=custom --no-owner --no-acl -U deopt_readonly -d deopt_readonly \
  | gpg --batch --yes --trust-model always --encrypt --recipient "$DEOPT_BACKUP_RECIPIENT" \
      --output "$tmpdir/$stamp.dump.gpg"
test -s "$tmpdir/$stamp.dump.gpg"
rclone copyto "$tmpdir/$stamp.dump.gpg" "$DEOPT_BACKUP_REMOTE/$stamp.dump.gpg"
echo 'ENCRYPTED_OFFSERVER_BACKUP=UPLOADED; RESTORE_NOT_YET_VERIFIED'
