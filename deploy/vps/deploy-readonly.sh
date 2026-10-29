#!/usr/bin/env bash
# Prepared for a future separately approved VPS release; never run --execute
# during local preparation. Host credentials and secrets stay outside Git.
set -euo pipefail
mode="${1:---check}"
[[ "$mode" == --check || "$mode" == --execute ]] || { echo 'usage: deploy-readonly.sh --check|--execute' >&2; exit 2; }
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
[[ -f "$root/compose.vps-readonly.yml" && -f "$root/deploy/vps/Caddyfile" ]] || exit 2
if [[ "$mode" == --check ]]; then
  bash -n "$0" "$root/deploy/vps/backend-entrypoint.sh"
  echo 'LOCAL_PREPARATION_CHECK=PASS; REMOTE_DEPLOYMENT=NOT_ATTEMPTED'
  exit 0
fi
[[ "${DEOPT_VPS_DEPLOY_AUTHORIZED:-}" == YES ]] || { echo 'Separate VPS deployment authorization required' >&2; exit 2; }
: "${DEOPT_VPS_HOST:?set explicit approved host}"
: "${DEOPT_VPS_USER:?set non-root SSH user}"
: "${DEOPT_VPS_DIR:?set absolute release directory}"
: "${DEOPT_VPS_KNOWN_HOSTS:?set reviewed known_hosts file}"
: "${DEOPT_VPS_SSH_KEY:?set approved SSH key path}"
: "${DEOPT_BACKEND_IMAGE:?set reviewed image digest}"
: "${DEOPT_POSTGRES_IMAGE:?set PostgreSQL 16 image digest}"
: "${DEOPT_CADDY_IMAGE:?set Caddy image digest}"
[[ "$DEOPT_VPS_HOST" =~ ^[a-zA-Z0-9.-]+$ && "$DEOPT_VPS_USER" =~ ^[a-z_][a-z0-9_-]*$ ]] || exit 2
[[ "$DEOPT_VPS_DIR" =~ ^/[a-zA-Z0-9_./-]+$ ]] || exit 2
[[ "$DEOPT_VPS_DIR" != *..* ]] || { echo 'Release directory cannot contain ..' >&2; exit 2; }
[[ "$DEOPT_BACKEND_IMAGE" =~ @sha256:[0-9a-f]{64}$ ]] || { echo 'Backend image must be digest-pinned' >&2; exit 2; }
[[ "$DEOPT_POSTGRES_IMAGE" =~ ^postgres:16(-alpine)?@sha256:[0-9a-f]{64}$ ]] || { echo 'PostgreSQL 16 image digest required' >&2; exit 2; }
[[ "$DEOPT_CADDY_IMAGE" =~ ^caddy:2[.][0-9]+@sha256:[0-9a-f]{64}$ ]] || { echo 'Caddy 2 image digest required' >&2; exit 2; }
[[ -r "$DEOPT_VPS_KNOWN_HOSTS" && -r "$DEOPT_VPS_SSH_KEY" ]] || exit 2
ssh-keygen -F "$DEOPT_VPS_HOST" -f "$DEOPT_VPS_KNOWN_HOSTS" >/dev/null || { echo 'SSH host key not pinned' >&2; exit 2; }
ssh_opts=(-o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$DEOPT_VPS_KNOWN_HOSTS" -i "$DEOPT_VPS_SSH_KEY")
dest="$DEOPT_VPS_USER@$DEOPT_VPS_HOST"
stage="$DEOPT_VPS_DIR/.stage-$$"
ssh "${ssh_opts[@]}" "$dest" "install -d -m 700 '$stage'"
scp "${ssh_opts[@]}" "$root/compose.vps-readonly.yml" "$root/deploy/vps/Caddyfile" "$dest:$stage/"
ssh "${ssh_opts[@]}" "$dest" bash -s -- "$DEOPT_VPS_DIR" "$DEOPT_BACKEND_IMAGE" "$stage" "$DEOPT_POSTGRES_IMAGE" "$DEOPT_CADDY_IMAGE" <<'REMOTE'
set -euo pipefail
cd "$1"
exec 9>.deploy.lock
flock -n 9 || { echo 'Another deployment is active' >&2; exit 2; }
install -d -m 750 deploy/vps
install -m 640 "$3/compose.vps-readonly.yml" compose.vps-readonly.yml
install -m 640 "$3/Caddyfile" deploy/vps/Caddyfile
rm -f -- "$3/compose.vps-readonly.yml" "$3/Caddyfile"
rmdir -- "$3"
test -f release.env || { echo 'Host release.env missing' >&2; exit 2; }
[[ "$(stat -c %a release.env)" == 600 ]] || { echo 'Host release.env must be mode 0600' >&2; exit 2; }
password_path="$(sed -n 's/^DEOPT_POSTGRES_PASSWORD_FILE=//p' release.env)"
[[ "$password_path" == /* && -f "$password_path" && "$(stat -c %a "$password_path")" == 600 && "$(stat -c %u "$password_path")" == 1001 ]] || {
  echo 'Host DB secret path must be absolute, mode 0600, and owned by UID 1001' >&2; exit 2;
}
previous="$(cat .current-image 2>/dev/null || true)"
export DEOPT_BACKEND_IMAGE="$2"
export DEOPT_POSTGRES_IMAGE="$4"
export DEOPT_CADDY_IMAGE="$5"
docker compose --env-file release.env -f compose.vps-readonly.yml config -q
docker compose --env-file release.env -f compose.vps-readonly.yml pull backend
docker compose --env-file release.env -f compose.vps-readonly.yml up -d postgres backend caddy
for _ in $(seq 1 20); do
  if docker compose --env-file release.env -f compose.vps-readonly.yml exec -T backend curl -fsS --max-time 3 http://127.0.0.1:8080/health >/dev/null \
    && docker compose --env-file release.env -f compose.vps-readonly.yml exec -T backend curl -fsS --max-time 3 http://127.0.0.1:8080/ready >/dev/null; then
    if [[ -n "$previous" ]]; then printf '%s\n' "$previous" > .previous-image; fi
    printf '%s\n' "$2" > .current-image
    echo 'SERVICE_HEALTH=PASS; SERVICE_READINESS=PASS; ECONOMIC_READINESS=NOT_AUTHORIZED'
    exit 0
  fi
  sleep 2
done
echo 'HEALTH=FAIL; preserve DB and review .previous-image before manual rollback' >&2
exit 1
REMOTE
