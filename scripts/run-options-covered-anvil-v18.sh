#!/usr/bin/env bash
# Local covered-call receipt proof. Deliberately does not enable hosted execution.
set -euo pipefail
trap 'echo "V18_FAILED_AT_LINE=$LINENO" >&2' ERR
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sol=$(cd -- "$repo/../deopt-v2-sol" && pwd)
front=$(cd -- "$repo/../deopt-v2-frontend" && pwd)
browser=${OPTIONS_V19_PRODUCT:-0}
inactive_series=${OPTIONS_V28_INACTIVE_SERIES:-0}
if [[ "$inactive_series" == 1 ]]; then browser=1; fi
port=${OPTIONS_V18_ANVIL_PORT:-18546}
pg_port=${OPTIONS_V18_PG_PORT:-15433}
frontend_port=${OPTIONS_V19_FRONTEND_PORT:-13001}
api_port=${OPTIONS_V19_API_PORT:-18556}
if [[ "$browser" == 1 && -z "${OPTIONS_V18_ANVIL_PORT:-}${OPTIONS_V18_PG_PORT:-}${OPTIONS_V19_FRONTEND_PORT:-}${OPTIONS_V19_API_PORT:-}" ]]; then
  # Keep the one-command product bench independent from other local projects.
  readarray -t bench_ports < <(python3 - <<'PY'
import socket
reserved = []
try:
    for _ in range(4):
        sock = socket.socket()
        sock.bind(('127.0.0.1', 0))
        reserved.append(sock)
    for sock in reserved:
        print(sock.getsockname()[1])
finally:
    for sock in reserved:
        sock.close()
PY
  )
  port=${bench_ports[0]}
  pg_port=${bench_ports[1]}
  frontend_port=${bench_ports[2]}
  api_port=${bench_ports[3]}
fi
url="http://127.0.0.1:$port"
onchain_uint() { cast call "$@" --rpc-url "$url" | awk '{print $1}'; }
log=$(mktemp /tmp/deopt-v18-covered.XXXXXX)
anvil_pid=
frontend_pid=
container="deopt-options-v18-pg-$$"
pg_started=0
cleanup() {
  if [[ -n "$frontend_pid" ]]; then kill -- "-$frontend_pid" 2>/dev/null || true; wait "$frontend_pid" 2>/dev/null || true; fi
  if [[ -n "$anvil_pid" ]]; then kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true; fi
  if (( pg_started )); then docker stop "$container" > /dev/null 2>&1 || true; fi
  rm -f "$log"
}
trap cleanup EXIT
for tool in anvil cast forge python3 docker cargo; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
if [[ "$browser" == 1 ]]; then
  for tool in npm curl setsid; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
fi
docker info > /dev/null
python3 - "$port" "$pg_port" "$frontend_port" "$api_port" "$browser" <<'PY'
import socket,sys
ports=list(map(int,sys.argv[1:5]))
assert len(ports)==len(set(ports))
for port in ports if sys.argv[5]=='1' else ports[:2]:
    with socket.socket() as s: s.bind(('127.0.0.1',port))
PY
anvil --host 127.0.0.1 --port "$port" --chain-id 84532 --silent > /dev/null 2>&1 &
anvil_pid=$!
for _ in {1..30}; do
  if [[ "$(cast chain-id --rpc-url "$url" 2>/dev/null || true)" == 84532 ]]; then break; fi
  sleep 1
done
test "$(cast chain-id --rpc-url "$url")" = 84532
cast rpc anvil_nodeInfo --rpc-url "$url" > /dev/null
docker run --rm -d --name "$container" \
  -e POSTGRES_PASSWORD=deopt-local-test-only -e POSTGRES_DB=deopt_options_v18 \
  -p "127.0.0.1:$pg_port:5432" --memory 768m --cpus 1 postgres:16 \
  -c fsync=off -c synchronous_commit=off > /dev/null
pg_started=1
for _ in {1..90}; do
  if docker exec "$container" pg_isready -h 127.0.0.1 -p 5432 -U postgres -d deopt_options_v18 > /dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$container" pg_isready -h 127.0.0.1 -p 5432 -U postgres -d deopt_options_v18 > /dev/null || {
  docker logs --tail 25 "$container" >&2 || true
  exit 1
}
if [[ "$browser" == 1 ]]; then
  # Run the existing cash fixture first to retain a live legacy Registry,
  # position and Vault throughout the physical product scenario.
  (cd "$sol" && OPTIONS_V4_DEPLOY_ONLY=false OPTIONS_V16_DUAL_REGISTRY=0 forge script \
    script/OptionsLocalAnvilE2E.s.sol:OptionsLocalAnvilE2E --rpc-url "$url" --broadcast --slow > "$log" 2>&1) \
    || { tail -n 25 "$log" >&2; exit 1; }
  export OPTIONS_V19_LEGACY_BROADCAST="$sol/broadcast/OptionsLocalAnvilE2E.s.sol/84532/run-latest.json"
fi
if [[ "$inactive_series" == 1 ]]; then
  (cd "$sol" && OPTIONS_V28_DEPLOY_ONLY=true forge script \
    script/OptionsInactiveLocalAnvilV28.s.sol:OptionsInactiveLocalAnvilV28 \
    --rpc-url "$url" --broadcast --slow > "$log" 2>&1) \
    || { tail -n 35 "$log" >&2; exit 1; }
  artifact="$sol/broadcast/OptionsInactiveLocalAnvilV28.s.sol/84532/run-latest.json"
  readarray -t deployed_v28 < <(python3 - "$artifact" <<'PY'
import json,sys
tx=json.load(open(sys.argv[1]))['transactions']
def made(name): return [x['contractAddress'] for x in tx if x.get('contractName')==name and x.get('transactionType')=='CREATE']
legacy,physical=made('OptionProductRegistry')
engine,=made('CoveredCallMarginEngine')
matcher,=made('OptionMatchingEngine')
owner,=made('LocalInactiveSeriesOwnerV28')
weth,usdc=made('CoveredLocalTokenV18')
print(legacy);print(physical);print(engine);print(matcher);print(owner);print(weth);print(usdc)
PY
  )
  export OPTIONS_V28_LEGACY_REGISTRY=${deployed_v28[0]}
  export OPTIONS_V28_PHYSICAL_REGISTRY=${deployed_v28[1]}
  export OPTIONS_V28_ENGINE=${deployed_v28[2]}
  export OPTIONS_V28_MATCHER=${deployed_v28[3]}
  export OPTIONS_V28_OWNER=${deployed_v28[4]}
  export OPTIONS_V28_WETH=${deployed_v28[5]}
  export OPTIONS_V28_USDC=${deployed_v28[6]}
  export OPTIONS_V28_EXPIRY=$(sed -n 's/^.*V28_EXPIRY[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
  export OPTIONS_V28_OPTION_ID=$(sed -n 's/^.*V28_LEGACY_ID[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
  export OPTIONS_V18_RPC_URL="$url" OPTIONS_V18_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$pg_port/deopt_options_v18"
  export OPTIONS_V19_FRONTEND_PORT="$frontend_port" OPTIONS_V19_API_PORT="$api_port"
  (cd "$front" && exec setsid env DEOPT_BACKEND_HTTPS_ORIGIN="http://127.0.0.1:$api_port" \
    NEXT_PUBLIC_TRADING_API_BASE_URL="http://127.0.0.1:$api_port" \
    NEXT_PUBLIC_CHAIN_ENV=sepolia NEXT_PUBLIC_OPTIONS_PHYSICAL_READS_ENABLED=1 \
    NEXT_PUBLIC_OPTIONS_LEGACY_REGISTRY="$OPTIONS_V28_LEGACY_REGISTRY" \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_REGISTRY="$OPTIONS_V28_PHYSICAL_REGISTRY" \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_ENGINE="$OPTIONS_V28_ENGINE" \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_UNDERLYING="$OPTIONS_V28_WETH" \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_QUOTE="$OPTIONS_V28_USDC" \
    npm run dev -- --hostname 127.0.0.1 --port "$frontend_port") \
    >"/tmp/deopt-options-v28-next-$$.log" 2>&1 &
  frontend_pid=$!
  for _ in {1..90}; do
    if curl -fsS "http://127.0.0.1:$frontend_port/options" >/dev/null 2>&1; then break; fi
    kill -0 "$frontend_pid" 2>/dev/null || { echo 'Next.js exited' >&2; exit 1; }
    sleep 1
  done
  (cd "$repo" && CARGO_BUILD_JOBS=1 cargo test --test options_v28_inactive_series -- --ignored --nocapture)
  echo 'V28_INACTIVE_SERIES_E2E=PASS'
  exit 0
fi
(cd "$sol" && OPTIONS_V19_DEPLOY_ONLY="$( [[ "$browser" == 1 ]] && echo true || echo false )" forge script script/OptionsCoveredLocalAnvilV18.s.sol:OptionsCoveredLocalAnvilV18 --rpc-url "$url" --broadcast --slow > "$log" 2>&1) || { tail -n 25 "$log" >&2; exit 1; }
artifact="$sol/broadcast/OptionsCoveredLocalAnvilV18.s.sol/84532/run-latest.json"
readarray -t deployed < <(python3 - "$artifact" <<'PY'
import json,sys
j=json.load(open(sys.argv[1])); tx=j['transactions']; receipts=j['receipts']
def creations(name): return [t['contractAddress'] for t in tx if t.get('contractName')==name and t.get('transactionType')=='CREATE']
engine,=creations('CoveredCallMarginEngine'); matcher,=creations('OptionMatchingEngine')
registry,=creations('OptionProductRegistry')
weth,usdc=creations('CoveredLocalTokenV18')
trades=[t for t in tx if t.get('contractName')=='CoveredLocalTradeBatchV18' and t.get('transactionType')=='CALL' and t.get('function','').startswith('executeTwo(')]
trade=trades[0] if trades else None
receipt=next((r for r in receipts if trade and r['transactionHash'].lower()==trade['hash'].lower()),None)
assert trade is None or receipt['status']=='0x1' and receipt['blockHash'].startswith('0x')
print(engine);print(matcher);print(weth);print(usdc);print(trade['hash'] if trade else '');print(receipt['blockHash'] if receipt else '');print(registry)
PY
)
engine=${deployed[0]}; weth=${deployed[2]}; usdc=${deployed[3]}; trade_hash=${deployed[4]}; registry=${deployed[6]}
expiry=$(sed -n 's/^.*V18_EXPIRY[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
[[ "$expiry" =~ ^[0-9]+$ ]] || { echo 'missing expiry' >&2; exit 1; }
# Resolve actual test-wallet addresses from the script log, never use a guessed wallet.
buyer=$(sed -n 's/^.*V18_BUYER[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
seller=$(sed -n 's/^.*V18_SELLER[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
[[ "$buyer" =~ ^0x[[:xdigit:]]{40}$ && "$seller" =~ ^0x[[:xdigit:]]{40}$ ]]
export OPTIONS_V18_RPC_URL="$url"
export OPTIONS_V18_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$pg_port/deopt_options_v18"
export OPTIONS_V18_BROADCAST="$artifact"
if [[ "$browser" == 1 ]]; then
  export OPTIONS_V19_FRONTEND_PORT="$frontend_port" OPTIONS_V19_API_PORT="$api_port"
  (cd "$front" && exec setsid env DEOPT_BACKEND_HTTPS_ORIGIN="http://127.0.0.1:$api_port" \
    NEXT_PUBLIC_TRADING_API_BASE_URL="http://127.0.0.1:$api_port" \
    NEXT_PUBLIC_DEOPT_WALLET_READ_AUTH=true NEXT_PUBLIC_CHAIN_ENV=sepolia \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_MODE=1 NEXT_PUBLIC_OPTIONS_PHYSICAL_REGISTRY="$registry" \
    NEXT_PUBLIC_OPTIONS_PHYSICAL_ENGINE="$engine" \
    npm run dev -- --hostname 127.0.0.1 --port "$frontend_port") \
    >"/tmp/deopt-options-v19-next-$$.log" 2>&1 &
  frontend_pid=$!
  for _ in {1..90}; do
    if curl -fsS "http://127.0.0.1:$frontend_port/options" >/dev/null 2>&1; then break; fi
    kill -0 "$frontend_pid" 2>/dev/null || { echo 'Next.js exited' >&2; exit 1; }
    sleep 1
  done
  curl -fsS "http://127.0.0.1:$frontend_port/portfolio" >/dev/null
  (cd "$repo" && CARGO_BUILD_JOBS=1 cargo test --test options_v19_physical_product -- --ignored --nocapture)
  echo 'V19_PHYSICAL_PRODUCT_E2E=PASS'
  exit 0
fi
initial_locked=$(onchain_uint "$engine" 'totalLockedUnderlying()(uint256)')
echo "V18_INITIAL_LOCKED=$initial_locked"
test "$initial_locked" = 2000000000000000000
snapshot=$(cast rpc evm_snapshot --rpc-url "$url" | python3 -c 'import json,sys; print(json.load(sys.stdin))')
[[ "$snapshot" == 0x* ]]
cast rpc evm_setNextBlockTimestamp "$expiry" --rpc-url "$url" > /dev/null
buyer_key=0x$(printf '%064x' $((0xB0B)))
seller_key=0x$(printf '%064x' $((0xCA11)))
exercise_hash=$(cast send "$engine" 'exercise(address,uint256)' "$seller" 1 --private-key "$buyer_key" --rpc-url "$url" --json | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["status"]=="0x1"; print(r["transactionHash"])')
test "$(onchain_uint "$weth" 'balanceOf(address)(uint256)' "$buyer")" = 1000000000000000000
seller_expected=2450000000
buyer_expected=2550000000
if [[ "${OPTIONS_V20_NONZERO_FEES:-0}" == 1 ]]; then
  seller_expected=2449950000
  buyer_expected=2549990000
fi
test "$(onchain_uint "$usdc" 'balanceOf(address)(uint256)' "$seller")" = "$seller_expected"
deadline=$((expiry + 48 * 3600))
cast rpc evm_setNextBlockTimestamp "$deadline" --rpc-url "$url" > /dev/null
release_hash=$(cast send "$engine" 'releaseExpired(address,address,uint256)' "$buyer" "$seller" 1 --private-key "$seller_key" --rpc-url "$url" --json | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["status"]=="0x1"; print(r["transactionHash"])')
test "$(onchain_uint "$engine" 'totalLockedUnderlying()(uint256)')" = 0
test "$(onchain_uint "$weth" 'balanceOf(address)(uint256)' "$seller")" = 1000000000000000000
test "$(onchain_uint "$weth" 'balanceOf(address)(uint256)' "$engine")" = 0
test "$(onchain_uint "$usdc" 'balanceOf(address)(uint256)' "$buyer")" = "$buyer_expected"
export OPTIONS_V18_TRADE_HASH="$trade_hash"
export OPTIONS_V18_EXERCISE_HASH="$exercise_hash"
export OPTIONS_V18_RELEASE_HASH="$release_hash"
export OPTIONS_V18_REORG_SNAPSHOT="$snapshot"
(cd "$repo" && CARGO_BUILD_JOBS=1 cargo test --test options_v18_physical_anvil_pg -- --ignored --nocapture)
echo 'V18_CHAIN_ID=84532'
echo "V18_COVERED_TRADE_TX=$trade_hash"
echo "V18_EXERCISE_TX=$exercise_hash"
echo "V18_NONEXERCISE_RELEASE_TX=$release_hash"
echo 'V18_WETH_LOCK_DELIVERY_RELEASE=PASS'
echo 'V18_USDC_STRIKE_AND_PREMIUM=PASS'
