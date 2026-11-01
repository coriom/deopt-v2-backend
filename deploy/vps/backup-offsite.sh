#!/usr/bin/env bash
# Host-operated backup: locate the reviewed private Compose PostgreSQL service,
# encrypt its dump before off-server upload, then verify downloaded ciphertext.
set -euo pipefail
: "${DEOPT_BACKUP_RECIPIENT:?set reviewed GPG recipient fingerprint}"
: "${DEOPT_BACKUP_REMOTE:?set reviewed rclone destination}"
[[ "$DEOPT_BACKUP_RECIPIENT" =~ ^[0-9A-Fa-f]{40}([0-9A-Fa-f]{24})?$ ]] || {
  echo 'Full GPG recipient fingerprint required' >&2; exit 2;
}
[[ "$DEOPT_BACKUP_REMOTE" == *:* ]] || { echo 'Off-server rclone destination required' >&2; exit 2; }
if [[ -n "${DEOPT_COMPOSE_PROJECT:-}" ]]; then
  [[ "$DEOPT_COMPOSE_PROJECT" =~ ^[a-z0-9][a-z0-9_-]{2,63}$ ]] || {
    echo 'Reviewed Coolify Compose project required' >&2; exit 2;
  }
  : "${DEOPT_BACKUP_POSTGRES_IMAGE:?set reviewed PostgreSQL image digest}"
  [[ "$DEOPT_BACKUP_POSTGRES_IMAGE" == \
     'postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a' ]] || {
    echo 'Reviewed PostgreSQL 16 image digest required' >&2; exit 2;
  }
  container="$(docker ps --filter "label=com.docker.compose.project=$DEOPT_COMPOSE_PROJECT" \
    --filter 'label=com.docker.compose.service=postgres' --format '{{.ID}}')" || exit 2
  [[ "$container" =~ ^[0-9a-f]{12,64}$ ]] || {
    echo 'Expected exactly one running PostgreSQL service container' >&2; exit 2;
  }
  [[ "$(docker inspect "$container" --format '{{.Config.Image}}')" == "$DEOPT_BACKUP_POSTGRES_IMAGE" ]] || {
    echo 'PostgreSQL container image differs from reviewed release' >&2; exit 2;
  }
  dump_command=(docker exec "$container" pg_dump --format=custom --no-owner --no-acl
    -U deopt_readonly -d deopt_readonly)
else
  : "${DEOPT_COMPOSE_FILE:?set absolute path to reviewed Compose profile}"
  : "${DEOPT_RELEASE_ENV_FILE:?set absolute path to host-only release.env}"
  [[ "$DEOPT_COMPOSE_FILE" == /* && -r "$DEOPT_COMPOSE_FILE" ]] || {
    echo 'Reviewed absolute Compose file required' >&2; exit 2;
  }
  [[ "$DEOPT_RELEASE_ENV_FILE" == /* && -r "$DEOPT_RELEASE_ENV_FILE" && "$(stat -c %a "$DEOPT_RELEASE_ENV_FILE")" == 600 ]] || {
    echo 'Host-only release.env must be readable and mode 0600' >&2; exit 2;
  }
  dump_command=(docker compose --env-file "$DEOPT_RELEASE_ENV_FILE" -f "$DEOPT_COMPOSE_FILE"
    exec -T postgres pg_dump --format=custom --no-owner --no-acl
    -U deopt_readonly -d deopt_readonly)
fi
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
"${dump_command[@]}" | gpg --batch --yes --trust-model always --encrypt \
  --recipient "$DEOPT_BACKUP_RECIPIENT" --output "$tmpdir/$stamp.dump.gpg"
test -s "$tmpdir/$stamp.dump.gpg"
rclone copyto "$tmpdir/$stamp.dump.gpg" "$DEOPT_BACKUP_REMOTE/$stamp.dump.gpg"
local_hash="$(sha256sum "$tmpdir/$stamp.dump.gpg" | cut -d ' ' -f 1)"
remote_hash="$(rclone cat "$DEOPT_BACKUP_REMOTE/$stamp.dump.gpg" | sha256sum | cut -d ' ' -f 1)"
[[ "$local_hash" == "$remote_hash" ]] || { echo 'OFFSERVER_CIPHERTEXT_HASH_MISMATCH' >&2; exit 1; }
echo 'ENCRYPTED_OFFSERVER_BACKUP=UPLOADED_AND_RETRIEVED; RESTORE_NOT_YET_VERIFIED'
