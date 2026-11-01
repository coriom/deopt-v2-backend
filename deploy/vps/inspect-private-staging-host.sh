#!/usr/bin/env bash
# Operator-only, read-only VPS evidence. Prints no secret or raw Compose content.
set -euo pipefail
[[ $# == 1 && "$1" == --inspect ]] || { echo 'usage: inspect-private-staging-host.sh --inspect' >&2; exit 2; }
[[ "$EUID" == 0 ]] || { echo 'ROOT_REQUIRED' >&2; exit 2; }

for path in /srv/deopt /srv/deopt/staging /srv/deopt/staging/secrets; do
  [[ -d "$path" && ! -L "$path" && "$(stat -c '%u:%g:%a' "$path")" == 0:0:700 ]] ||
    { echo 'POSTGRES_SECRET_DIRECTORY_PERMISSIONS=FAIL' >&2; exit 2; }
done
secret=/srv/deopt/staging/secrets/postgres_password
[[ -f "$secret" && ! -L "$secret" && -s "$secret" &&
   "$(stat -c '%u:%g:%a' "$secret")" == 0:1001:640 ]] ||
  { echo 'POSTGRES_SECRET_FILE_PERMISSIONS=FAIL' >&2; exit 2; }
echo 'POSTGRES_SECRET_HOST_PERMISSIONS=PASS_CONTENT_UNREAD'

proxy=/data/coolify/proxy/docker-compose.yml
[[ -f "$proxy" && ! -L "$proxy" ]] || { echo 'PROXY_GENERATED_COMPOSE_MISSING' >&2; exit 2; }
printf 'PROXY_GENERATED_COMPOSE_SHA256=%s\n' "$(sha256sum "$proxy" | cut -d ' ' -f 1)"
docker compose -f "$proxy" config --format json |
  python3 -c 'import json,sys; d=json.load(sys.stdin); ps=[]
for svc in d.get("services",{}).values():
 for p in svc.get("ports",[]):
  if isinstance(p,dict): ps.append(str(p.get("published", "")))
  elif isinstance(p,str): ps.append(p)
print("PROXY_GENERATED_PUBLISHED_PORTS="+",".join(sorted(ps)))'
ss -ltnH '( sport = :8080 )' |
  python3 -c 'import sys; s=sys.stdin.read(); print("PROXY_8080_LISTEN_IPV4="+str("0.0.0.0:8080" in s)); print("PROXY_8080_LISTEN_IPV6="+str("[::]:8080" in s))'
containers="$(docker ps --format '{{.Names}}')" || { echo 'DOCKER_LIST_FAILED' >&2; exit 2; }
if grep -Eq '(^|[-_])deopt([-_]|$)' <<<"$containers"; then
  echo 'DEOPT_CONTAINER_PRESENT=YES'
else
  echo 'DEOPT_CONTAINER_PRESENT=NO'
fi
echo 'COOLIFY_UI_SAVED_PROXY_CONFIGURATION=NOT_VERIFIED_BY_THIS_SCRIPT'
