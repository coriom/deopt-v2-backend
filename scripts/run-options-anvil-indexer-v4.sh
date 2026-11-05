#!/usr/bin/env bash
# Local transaction + Rust indexer + disposable PostgreSQL proof. No public RPC.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sol=$(cd -- "$repo/../deopt-v2-sol" && pwd)
anvil_port=${OPTIONS_V4_ANVIL_PORT:-18545}
pg_port=${OPTIONS_V4_PG_PORT:-15432}
container="deopt-options-v4-pg-$$"
anvil_pid=
pg_started=0
cleanup() {
  if [[ -n "$anvil_pid" ]]; then kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true; fi
  if (( pg_started )); then docker stop "$container" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
for tool in anvil cast forge docker cargo python3 psql; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
docker info >/dev/null
python3 - "$anvil_port" "$pg_port" <<'PY'
import socket,sys
for port in map(int,sys.argv[1:]):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',port))
PY
if [[ -n "$(docker ps -aq --filter "name=^/${container}$")" ]]; then echo 'test container name in use' >&2; exit 1; fi
anvil --host 127.0.0.1 --port "$anvil_port" --chain-id 31337 --silent >/dev/null 2>&1 &
anvil_pid=$!
for _ in {1..30}; do
  if [[ "$(cast chain-id --rpc-url "http://127.0.0.1:$anvil_port" 2>/dev/null || true)" == 31337 ]]; then break; fi
  sleep 1
done
test "$(cast chain-id --rpc-url "http://127.0.0.1:$anvil_port")" = 31337
export OPTIONS_V4_SNAPSHOT
OPTIONS_V4_SNAPSHOT=$(cast rpc evm_snapshot --rpc-url "http://127.0.0.1:$anvil_port" | python3 -c 'import json,sys; print(json.load(sys.stdin))')
[[ "$OPTIONS_V4_SNAPSHOT" == 0x* ]] || { echo 'invalid Anvil snapshot ID' >&2; exit 1; }
docker run --rm -d --name "$container" \
  -e POSTGRES_PASSWORD=deopt-local-test-only -e POSTGRES_DB=deopt_options_v4 \
  -p "127.0.0.1:$pg_port:5432" --memory 768m --cpus 1 postgres:16 \
  -c fsync=off -c synchronous_commit=off >/dev/null
pg_started=1
for _ in {1..90}; do
  if docker exec "$container" pg_isready -U postgres -d deopt_options_v4 >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$container" pg_isready -U postgres -d deopt_options_v4 >/dev/null
for _ in {1..120}; do
  if PGPASSWORD=deopt-local-test-only psql -h 127.0.0.1 -p "$pg_port" \
      -U postgres -d deopt_options_v4 -Atqc 'SELECT 1' 2>/dev/null | grep -qx 1; then break; fi
  sleep 1
done
PGPASSWORD=deopt-local-test-only psql -h 127.0.0.1 -p "$pg_port" \
  -U postgres -d deopt_options_v4 -Atqc 'SELECT 1' | grep -qx 1
(cd "$sol" && forge script script/OptionsLocalAnvilE2E.s.sol:OptionsLocalAnvilE2E \
  --rpc-url "http://127.0.0.1:$anvil_port" --broadcast --slow)
export OPTIONS_V4_RPC_URL="http://127.0.0.1:$anvil_port"
export OPTIONS_V4_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$pg_port/deopt_options_v4"
export OPTIONS_V4_BROADCAST="$sol/broadcast/OptionsLocalAnvilE2E.s.sol/31337/run-latest.json"
(cd "$repo" && CARGO_BUILD_JOBS=2 cargo test --test options_v4_live_anvil_pg -- --ignored --nocapture)
