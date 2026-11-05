#!/usr/bin/env bash
# Local transaction + Rust indexer + disposable PostgreSQL proof. No public RPC.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sol=$(cd -- "$repo/../deopt-v2-sol" && pwd)
front=$(cd -- "$repo/../deopt-v2-frontend" && pwd)
anvil_port=${OPTIONS_V4_ANVIL_PORT:-18545}
pg_port=${OPTIONS_V4_PG_PORT:-15432}
chain_id=${OPTIONS_V4_CHAIN_ID:-31337}
deploy_only=false
if [[ "${OPTIONS_V4_BROWSER:-0}" == 1 ]]; then deploy_only=true; fi
[[ "$chain_id" == 31337 || "$chain_id" == 84532 ]] || { echo 'unsupported local chain ID' >&2; exit 1; }
container="deopt-options-v4-pg-$$"
anvil_pid=
frontend_pid=
pg_started=0
cleanup() {
  if [[ -n "$frontend_pid" ]]; then kill -- "-$frontend_pid" 2>/dev/null || true; wait "$frontend_pid" 2>/dev/null || true; fi
  if [[ -n "$anvil_pid" ]]; then kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true; fi
  if (( pg_started )); then docker stop "$container" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
for tool in anvil cast forge docker cargo python3 psql; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
if [[ "${OPTIONS_V4_BROWSER:-0}" == 1 ]]; then
  for tool in node npm curl setsid; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
  [[ "$chain_id" == 84532 ]] || { echo 'browser auth bench requires local Anvil chain 84532' >&2; exit 1; }
  export OPTIONS_V4_FRONTEND_PORT=${OPTIONS_V4_FRONTEND_PORT:-13000}
  export OPTIONS_V4_API_PORT=${OPTIONS_V4_API_PORT:-18555}
fi
docker info >/dev/null
python3 - "$anvil_port" "$pg_port" "${OPTIONS_V4_FRONTEND_PORT:-13000}" "${OPTIONS_V4_API_PORT:-18555}" <<'PY'
import socket,sys
ports = list(map(int,sys.argv[1:]))
if len(ports) != len(set(ports)): raise SystemExit('test ports must differ')
for port in ports[:4 if __import__('os').environ.get('OPTIONS_V4_BROWSER') == '1' else 2]:
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(('127.0.0.1',port))
PY
if [[ -n "$(docker ps -aq --filter "name=^/${container}$")" ]]; then echo 'test container name in use' >&2; exit 1; fi
anvil --host 127.0.0.1 --port "$anvil_port" --chain-id "$chain_id" --silent >/dev/null 2>&1 &
anvil_pid=$!
for _ in {1..30}; do
  if [[ "$(cast chain-id --rpc-url "http://127.0.0.1:$anvil_port" 2>/dev/null || true)" == "$chain_id" ]]; then break; fi
  sleep 1
done
test "$(cast chain-id --rpc-url "http://127.0.0.1:$anvil_port")" = "$chain_id"
cast rpc anvil_nodeInfo --rpc-url "http://127.0.0.1:$anvil_port" >/dev/null
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
(cd "$sol" && OPTIONS_V4_DEPLOY_ONLY="$deploy_only" forge script script/OptionsLocalAnvilE2E.s.sol:OptionsLocalAnvilE2E \
  --rpc-url "http://127.0.0.1:$anvil_port" --broadcast --slow)
if [[ "${OPTIONS_V4_BROWSER:-0}" == 1 ]]; then
  # Keep deployed contracts, then orphan only the browser-originated trade.
  OPTIONS_V4_SNAPSHOT=$(cast rpc evm_snapshot --rpc-url "http://127.0.0.1:$anvil_port" | python3 -c 'import json,sys; print(json.load(sys.stdin))')
  [[ "$OPTIONS_V4_SNAPSHOT" == 0x* ]] || { echo 'invalid post-deployment Anvil snapshot' >&2; exit 1; }
fi
export OPTIONS_V4_RPC_URL="http://127.0.0.1:$anvil_port"
export OPTIONS_V4_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$pg_port/deopt_options_v4"
export OPTIONS_V4_BROADCAST="$sol/broadcast/OptionsLocalAnvilE2E.s.sol/$chain_id/run-latest.json"
if [[ "${OPTIONS_V4_BROWSER:-0}" == 1 ]]; then
  (cd "$front" && exec setsid env DEOPT_BACKEND_HTTPS_ORIGIN="http://127.0.0.1:$OPTIONS_V4_API_PORT" \
    NEXT_PUBLIC_TRADING_API_BASE_URL="http://127.0.0.1:$OPTIONS_V4_API_PORT" \
    NEXT_PUBLIC_DEOPT_WALLET_READ_AUTH=true NEXT_PUBLIC_CHAIN_ENV=sepolia \
    npm run dev -- --hostname 127.0.0.1 --port "$OPTIONS_V4_FRONTEND_PORT") \
    >/tmp/deopt-options-v4-next-$$.log 2>&1 &
  frontend_pid=$!
  for _ in {1..90}; do
    if curl -fsS "http://127.0.0.1:$OPTIONS_V4_FRONTEND_PORT/portfolio" >/dev/null 2>&1; then break; fi
    kill -0 "$frontend_pid" 2>/dev/null || { echo 'Next.js exited before browser test' >&2; exit 1; }
    sleep 1
  done
  curl -fsS "http://127.0.0.1:$OPTIONS_V4_FRONTEND_PORT/portfolio" >/dev/null
  if [[ "${OPTIONS_V5_PRODUCT:-0}" == 1 ]]; then
    curl -fsS "http://127.0.0.1:$OPTIONS_V4_FRONTEND_PORT/options" >/dev/null
  fi
fi
(cd "$repo" && CARGO_BUILD_JOBS=2 cargo test --test options_v4_live_anvil_pg -- --ignored --nocapture)
