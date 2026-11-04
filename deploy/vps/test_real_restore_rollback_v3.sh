#!/usr/bin/env bash
# Local Docker-only drill; never point this at Netcup or the active production DB.
set -euo pipefail
set +x
[[ "$(hostname -s)" != v2202611448349549977 ]] || exit 2
repo=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
old=deopt-production-restore-drill
pg=deopt-real-rollback-pg-v3
api=deopt-real-rollback-api-v3
secrets=/tmp/deopt-leastpriv-test
image='postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a'
[[ "$(docker inspect "$old" --format '{{.Config.Image}}')" == "$image" &&
   "$(docker inspect "$old" --format '{{.HostConfig.NetworkMode}}')" == none ]] || exit 2
volume=$(docker inspect "$old" --format \
  '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}')
[[ "$volume" =~ ^[0-9a-f]{64}$ && "$volume" != dxco90jwpdxyennmquv2elyj_postgres_data ]] || exit 2
socket_volume=deopt_real_rollback_socket_v3
if [[ "$(docker inspect "$old" --format '{{.State.Running}}')" == true ]]; then
  [[ "$(docker exec "$old" psql -X -A -t -U postgres -d deopt_prod_restore_test \
    -c 'SELECT count(*) FROM _sqlx_migrations')" == 72 ]] || exit 2
  [[ "$(docker exec "$old" psql -X -A -t -U postgres -d deopt_prod_restore_test \
    -c 'SHOW log_statement')" == none ]] || exit 2
  for role in postgres runtime migrator backup; do
    file="$secrets/${role/postgres/admin}"
    [[ ! -L "$file" && -f "$file" && "$(cat "$file")" =~ ^[0-9a-f]{64}$ ]] || exit 2
    password=$(cat "$file")
    db_role=postgres
    [[ "$role" == postgres ]] || db_role="deopt_$role"
    printf "SET password_encryption='scram-sha-256'; ALTER ROLE %s PASSWORD '%s';\n" \
      "$db_role" "$password" |
      docker exec -i "$old" psql -X -q -v ON_ERROR_STOP=1 -U postgres \
        -d deopt_prod_restore_test >/dev/null 2>&1 || {
          echo 'DISPOSABLE_ROLE_SETUP_FAILED' >&2; exit 2;
        }
    unset password
  done
  docker volume create "$socket_volume" >/dev/null
  docker stop "$old" >/dev/null
  docker run -d --name "$pg" --network none --pull never \
    -v "$volume:/var/lib/postgresql/data" \
    -v "$socket_volume:/var/run/postgresql" \
    -v "$repo/deploy/vps/postgres-socket-pg_hba.conf:/etc/postgresql/deopt_pg_hba.conf:ro" \
    -v "$secrets/admin:/run/secrets/admin_password:ro" \
    "$image" postgres -c listen_addresses= \
      -c unix_socket_directories=/var/run/postgresql \
      -c unix_socket_group=postgres -c unix_socket_permissions=0770 \
      -c hba_file=/etc/postgresql/deopt_pg_hba.conf >/dev/null
else
  [[ "$(docker inspect "$pg" --format '{{.State.Running}}')" == true &&
     "$(docker inspect "$pg" --format '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}')" == "$volume" ]] || exit 2
fi
for attempt in $(seq 1 30); do
  if docker exec "$pg" pg_isready -h /var/run/postgresql -U postgres \
      -d deopt_prod_restore_test >/dev/null 2>&1; then break; fi
  sleep 1
done
[[ "$(docker inspect "$pg" --format '{{.HostConfig.NetworkMode}}')" == none ]] || exit 2
if ! docker exec "$pg" sh -ec '
  PGPASSWORD=$(cat /run/secrets/admin_password); export PGPASSWORD
  psql -X -q -U postgres -d deopt_readonly -c "SELECT 1" >/dev/null' \
  2>/dev/null; then
  docker exec "$pg" sh -ec '
    PGPASSWORD=$(cat /run/secrets/admin_password); export PGPASSWORD
    exec createdb -U postgres -T deopt_prod_restore_test deopt_readonly'
fi
printf 'REVOKE CONNECT, TEMPORARY ON DATABASE deopt_readonly FROM PUBLIC; GRANT CONNECT ON DATABASE deopt_readonly TO deopt_runtime, deopt_migrator, deopt_backup;\n' |
  docker exec -i "$pg" sh -ec '
    PGPASSWORD=$(cat /run/secrets/admin_password); export PGPASSWORD
    exec psql -X -q -v ON_ERROR_STOP=1 -U postgres -d deopt_readonly' >/dev/null
if docker inspect "$api" >/dev/null 2>&1; then
  [[ "$(docker inspect "$api" --format '{{.State.Running}}')" == false &&
     "$(docker inspect "$api" --format '{{.HostConfig.NetworkMode}}')" == none ]] || exit 2
  docker rm "$api" >/dev/null
fi
run_api() {
  local local_image=$1 expected=$2
  docker run -d --name "$api" --network none --pull never --user 1001:999 \
    --read-only --pids-limit 128 --memory 512m --cap-drop ALL \
    --security-opt no-new-privileges \
    -v "$socket_volume:/var/run/postgresql:ro" \
    -v "$secrets/runtime:/run/secrets/runtime_password:ro" \
    -e CHAIN_ID=84532 -e NETWORK_NAME=base-sepolia \
    -e DEOPT_READ_ONLY_RELEASE=true -e PERSISTENCE_ENABLED=true \
    -e DEOPT_POSTGRES_SOCKET_ONLY=true -e DEOPT_DB_ROLE=runtime \
    -e PERPS_READ_ENABLED=false -e EXECUTION_ENABLED=false \
    -e EXECUTOR_REAL_BROADCAST_ENABLED=false -e HV2_BROADCAST_ENABLED=false \
    -e INDEXER_ENABLED=false -e WALLET_READ_AUTH_ENABLED=true \
    -e WALLET_READ_AUTH_ORIGIN=https://deoptv2-frontend.vercel.app \
    -e HOST=127.0.0.1 -e PORT=8080 \
    --entrypoint /usr/local/bin/deopt-readonly-entrypoint "$local_image" >/dev/null
  local ready=000
  for attempt in $(seq 1 20); do
    ready=$(docker exec "$api" sh -c \
      'curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8080/ready' 2>/dev/null) || true
    [[ "$ready" == 200 ]] && break
    sleep 1
  done
  [[ "$ready" == 200 ]] || exit 2
  local status
  status=$(docker exec "$api" sh -c \
    'curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8080/accounts/0x1111111111111111111111111111111111111111/perps/canonical-fees')
  [[ "$status" == "$expected" ]] || exit 2
  docker stop "$api" >/dev/null
  docker rm "$api" >/dev/null
}
run_api deopt-v3-leastpriv-local:cutover 401
run_api deopt-v3-fallback-local:locked 403
run_api deopt-v3-leastpriv-local:cutover 401
count=$(docker exec "$pg" sh -ec '
  PGPASSWORD=$(cat /run/secrets/admin_password); export PGPASSWORD
  psql -X -A -t -U postgres -d deopt_readonly \
    -c "SELECT count(*) FROM _sqlx_migrations"')
[[ "$count" == 72 ]] || exit 2
printf 'REAL_DATA_ROLLBACK_IMAGE_TEST=PASS\nRESTORED_SCHEMA=72\nPGDATA_VOLUME_PRESERVED=YES\nNETWORK_PORTS=NONE\n'
docker stop "$pg" >/dev/null
