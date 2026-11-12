#!/usr/bin/env bash
# Read-only source volume -> disposable PostgreSQL clone. Never points at Netcup.
set -euo pipefail
[[ $# == 1 && "$1" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo 'usage: local-restored-volume-name' >&2; exit 2; }
source_volume=$1
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
docker volume inspect "$source_volume" >/dev/null
[[ -z "$(docker ps -q --filter "volume=$source_volume")" ]] || { echo 'SOURCE_VOLUME_IN_USE' >&2; exit 2; }
tag="deopt-v29-restored-$$"
docker volume create "$tag" >/dev/null
cleanup() {
  docker rm -f "$tag" >/dev/null 2>&1 || true
  docker volume rm "$tag" >/dev/null 2>&1 || true
}
trap cleanup EXIT
docker run --rm -v "$source_volume:/src:ro" -v "$tag:/dst" busybox:musl \
  sh -c 'test "$(cat /src/PG_VERSION)" = 16 && cp -a /src/. /dst/ && test "$(cat /dst/PG_VERSION)" = 16' >/dev/null
port=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    print(sock.getsockname()[1])
PY
)
docker run -d --name "$tag" -v "$tag:/var/lib/postgresql/data" \
  -p "127.0.0.1:$port:5432" --memory 768m --cpus 1 postgres:16 >/dev/null
for _ in {1..30}; do
  docker exec -u postgres "$tag" pg_isready -q >/dev/null 2>&1 && break
  sleep 1
done
docker exec -u postgres "$tag" pg_isready -q >/dev/null
sql() { docker exec -u postgres "$tag" psql -XAt -v ON_ERROR_STOP=1 -d deopt_readonly -c "$1"; }
[[ "$(sql 'SELECT COUNT(*) FROM _sqlx_migrations')" == 72 ]] || exit 2
nonempty=$(sql "SELECT COUNT(*) FROM pg_stat_user_tables WHERE schemaname='public' AND n_live_tup>0 AND relname NOT LIKE '_sqlx%' ")
sql "ALTER ROLE postgres PASSWORD 'deopt-local-test-only'" >/dev/null
export OPTIONS_V20_REAL_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$port/deopt_readonly"
(cd "$repo" && CARGO_BUILD_JOBS=1 cargo test --test options_v20_real_migration -- --ignored --nocapture)
[[ "$(sql 'SELECT COUNT(*) FROM _sqlx_migrations')" == 75 ]] || exit 2
echo 'LOCAL_RESTORED_VOLUME_SCHEMA75=PASS'
echo "SOURCE_NONEMPTY_TABLES=$nonempty"
