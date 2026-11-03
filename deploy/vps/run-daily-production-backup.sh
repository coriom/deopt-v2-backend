#!/usr/bin/env bash
# Root-owned systemd target only. Prepared offline; do not enable without alerting.
set -euo pipefail
set +x
umask 077
[[ ${EUID} == 0 && "$(hostname -s)" == v2202611448349549977 ]] || {
  echo 'REVIEWED_ROOT_HOST_REQUIRED' >&2; exit 2;
}
exec 9>/run/lock/deopt-production-backup.lock
flock -n 9 || { echo 'BACKUP_ALREADY_RUNNING' >&2; exit 75; }
backup_script=/root/deopt-backup-offsite-v1.sh
expected_script=d132f8596042a4a1011257e98fd6a8d340f26407d1b0d4b0dc520f103b13a418
[[ -f "$backup_script" && ! -L "$backup_script" &&
   "$(stat -c %U:%G:%a "$backup_script")" == root:root:700 &&
   "$(sha256sum "$backup_script" | cut -d ' ' -f 1)" == "$expected_script" ]] || {
  echo 'BACKUP_SCRIPT_INTEGRITY_FAIL' >&2; exit 2;
}
export GNUPGHOME=/root/.local/share/deopt-backup-gpg
export RCLONE_CONFIG=/root/.config/deopt/rclone.conf
[[ -d "$GNUPGHOME" && "$(stat -c %u:%a "$GNUPGHOME")" == 0:700 ]] || {
  echo 'PUBLIC_GPG_KEYRING_NOT_READY' >&2; exit 2;
}
[[ -f "$RCLONE_CONFIG" && ! -L "$RCLONE_CONFIG" &&
   "$(stat -c %u:%a "$RCLONE_CONFIG")" == 0:600 ]] || {
  echo 'ROOT_RCLONE_CONFIG_NOT_READY' >&2; exit 2;
}
rclone listremotes | grep -qx 'deopt_r2:' || { echo 'DEOPT_R2_REMOTE_NOT_READY' >&2; exit 2; }
mapfile -t fingerprints < <(gpg --batch --with-colons --list-keys |
  awk -F: '$1 == "pub" { want=1; next } want && $1 == "fpr" { print $10; want=0 }')
[[ ${#fingerprints[@]} == 1 && ${fingerprints[0]} =~ ^[0-9A-Fa-f]{40}([0-9A-Fa-f]{24})?$ ]] || {
  echo 'ONE_DEDICATED_PUBLIC_GPG_KEY_REQUIRED' >&2; exit 2;
}
export DEOPT_BACKUP_RECIPIENT=${fingerprints[0]}
export DEOPT_BACKUP_REMOTE=deopt_r2:deopt-private-backups/postgres
export DEOPT_COMPOSE_PROJECT=dxco90jwpdxyennmquv2elyj
export DEOPT_BACKUP_POSTGRES_IMAGE='postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a'
state_dir=/var/lib/deopt-backup
[[ ! -e "$state_dir" || ( -d "$state_dir" && ! -L "$state_dir" ) ]] || {
  echo 'BACKUP_STATE_DIRECTORY_CONFLICT' >&2; exit 2;
}
install -d -o root -g root -m 0700 "$state_dir"
[[ "$(stat -c %U:%G:%a "$state_dir")" == root:root:700 ]] || {
  echo 'BACKUP_STATE_DIRECTORY_UNSAFE' >&2; exit 2;
}
run_id=${INVOCATION_ID:-$(tr -d '-' </proc/sys/kernel/random/uuid)}
[[ "$run_id" =~ ^[0-9a-f]{32}$ ]] || { echo 'BACKUP_RUN_ID_INVALID' >&2; exit 2; }
bash "$backup_script"
state_file="$(mktemp "$state_dir/.last-success.XXXXXXXX")"
printf '%s %s\n' "$(date -u +%s)" "$run_id" > "$state_file"
chmod 0600 "$state_file"
mv -T -- "$state_file" "$state_dir/last-success"
rm -f -- "$state_dir/last-watchdog-alert-key" || true
printf 'BACKUP_SUCCESS_STATE=RECORDED\nBACKUP_RUN_ID=%s\n' "$run_id"
