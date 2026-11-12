#!/usr/bin/env bash
# Disposable schema-75 API -> PostgreSQL -> real Base Sepolia read-only smoke.
set -euo pipefail
set +x
umask 077
rpc_source=${OPTIONS_V29_RPC_SOURCE:-/home/corio/DEOPT/deopt-v2-sol/.env.base-sepolia}
image=${OPTIONS_SCHEMA_CANDIDATE_IMAGE:-deopt-v29-candidate:local}
[[ -f "$rpc_source" && ! -L "$rpc_source" ]] || exit 2
for tool in docker python3 curl; do command -v "$tool" >/dev/null || exit 2; done
tag="deopt-v29-rpc-$$"
network="$tag-net"
pg="$tag-pg"
api="$tag-api"
work=$(mktemp -d /tmp/deopt-v29-rpc.XXXXXX)
cleanup() {
  docker rm -f "$api" "$pg" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf -- "$work"
}
trap cleanup EXIT
python3 - "$rpc_source" "$work/api.env" <<'PY'
from pathlib import Path
import sys
from urllib.parse import urlsplit
source, target = map(Path, sys.argv[1:])
values = [line.split('=', 1)[1].strip().strip('"\'') for line in source.read_text().splitlines() if line.startswith('RPC_URL=')]
assert len(values) == 1 and values[0].startswith('https://') and '\n' not in values[0]
assert urlsplit(values[0]).hostname
target.write_text('RPC_URL=' + values[0] + '\n')
target.chmod(0o600)
PY
port=$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    print(s.getsockname()[1])
PY
)
docker network create "$network" >/dev/null
docker run --rm -d --name "$pg" --network "$network" --network-alias postgres \
  --memory 768m --cpus 1 -e POSTGRES_PASSWORD=deopt-local-test-only \
  -e POSTGRES_DB=deopt_readonly postgres:16 >/dev/null
for _ in {1..60}; do
  docker exec "$pg" pg_isready -U postgres -d deopt_readonly >/dev/null 2>&1 && break
  sleep 1
done
docker exec "$pg" pg_isready -U postgres -d deopt_readonly >/dev/null
db_url='postgres://postgres:deopt-local-test-only@postgres:5432/deopt_readonly'
docker run --rm --network "$network" --memory 512m --cpus 1 \
  -e DATABASE_URL="$db_url" -e CHAIN_ID=84532 -e DEOPT_READ_ONLY_RELEASE=true \
  "$image" --migrate-only >/dev/null
[[ "$(docker exec "$pg" psql -XAt -U postgres -d deopt_readonly -c 'SELECT COUNT(*) FROM _sqlx_migrations')" == 75 ]] || exit 2
docker run --rm -d --name "$api" --network "$network" -p "127.0.0.1:$port:8080" \
  --memory 768m --cpus 1 --env-file "$work/api.env" \
  -e CHAIN_ID=84532 -e PERSISTENCE_ENABLED=true -e DEOPT_READ_ONLY_RELEASE=true \
  -e DATABASE_URL="$db_url" -e OPTIONS_ENABLED=false -e EXECUTION_ENABLED=false \
  -e EXECUTOR_REAL_BROADCAST_ENABLED=false -e HV2_BROADCAST_ENABLED=false \
  -e OPTION_EXECUTION_ENABLED=false -e OPTION_EXECUTION_BROADCAST_ENABLED=false \
  -e INDEXER_ENABLED=false -e OPTION_EVENT_INDEXER_ENABLED=false \
  -e OPTION_PHYSICAL_REGISTRY_ADDRESS=0xcb7bc539ed4A5771F3420f498002e0fc4eE1bD27 \
  -e OPTION_PHYSICAL_ENGINE_ADDRESS=0x2C4589c88323009db372B4067702e6292ddD294C \
  -e OPTION_PHYSICAL_MATCHING_ENGINE_ADDRESS=0x63DE63912BB6743F72ea77847572503Cc93B7d80 \
  "$image" >/dev/null
for _ in {1..60}; do
  curl --fail --silent "http://127.0.0.1:$port/ready" >/dev/null && break
  sleep 1
done
curl --fail --silent "http://127.0.0.1:$port/ready" >/dev/null
curl --fail --silent "http://127.0.0.1:$port/options/physical-stack" > "$work/stack.json"
python3 - "$work/stack.json" <<'PY'
from pathlib import Path
import json, sys
body = json.loads(Path(sys.argv[1]).read_text())
data = body['data']
assert body['status'] == 'ok'
assert data['chain_id'] == 84532
assert data['series_count_onchain'] == '0' and data['series_count_indexed'] == 0
assert data['engine_paused']
assert data['max_open_contracts'] == '0' and not data['matching_engine_bound']
assert data['economic_state'] == 'inactive'
print('MATCHER_PAUSED_CURRENT=' + str(data['matcher_paused']).lower())
print('BASE_SEPOLIA_PHYSICAL_READ_SCHEMA75=PASS')
PY
[[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$port/options/orders")" == 405 ]] || exit 2
echo 'SCHEMA75_RPC_ECONOMIC_WRITE_GUARD=PASS'
