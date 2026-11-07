#!/usr/bin/env bash
# Local covered-call receipt proof. Deliberately does not enable hosted execution.
set -euo pipefail
trap 'echo "V18_FAILED_AT_LINE=$LINENO" >&2' ERR
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
sol=$(cd -- "$repo/../deopt-v2-sol" && pwd)
port=${OPTIONS_V18_ANVIL_PORT:-18546}
pg_port=${OPTIONS_V18_PG_PORT:-15433}
url="http://127.0.0.1:$port"
onchain_uint() { cast call "$@" --rpc-url "$url" | awk '{print $1}'; }
log=$(mktemp /tmp/deopt-v18-covered.XXXXXX)
anvil_pid=
container="deopt-options-v18-pg-$$"
pg_started=0
cleanup() {
  if [[ -n "$anvil_pid" ]]; then kill "$anvil_pid" 2>/dev/null || true; wait "$anvil_pid" 2>/dev/null || true; fi
  if (( pg_started )); then docker stop "$container" > /dev/null 2>&1 || true; fi
  rm -f "$log"
}
trap cleanup EXIT
for tool in anvil cast forge python3 docker cargo; do command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }; done
docker info > /dev/null
python3 - "$port" "$pg_port" <<'PY'
import socket,sys
for port in map(int,sys.argv[1:]):
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
  if docker exec "$container" pg_isready -U postgres -d deopt_options_v18 > /dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$container" pg_isready -U postgres -d deopt_options_v18 > /dev/null
(cd "$sol" && forge script script/OptionsCoveredLocalAnvilV18.s.sol:OptionsCoveredLocalAnvilV18 --rpc-url "$url" --broadcast --slow > "$log" 2>&1) || { tail -n 25 "$log" >&2; exit 1; }
artifact="$sol/broadcast/OptionsCoveredLocalAnvilV18.s.sol/84532/run-latest.json"
readarray -t deployed < <(python3 - "$artifact" <<'PY'
import json,sys
j=json.load(open(sys.argv[1])); tx=j['transactions']; receipts=j['receipts']
def creations(name): return [t['contractAddress'] for t in tx if t.get('contractName')==name and t.get('transactionType')=='CREATE']
engine,=creations('CoveredCallMarginEngine'); matcher,=creations('OptionMatchingEngine')
weth,usdc=creations('CoveredLocalTokenV18')
trade=next(t for t in tx if t.get('contractName')=='OptionMatchingEngine' and t.get('transactionType')=='CALL' and t.get('function','').startswith('executeTrade('))
receipt=next(r for r in receipts if r['transactionHash'].lower()==trade['hash'].lower())
assert receipt['status']=='0x1' and receipt['blockHash'].startswith('0x')
print(engine);print(matcher);print(weth);print(usdc);print(trade['hash']);print(receipt['blockHash'])
PY
)
engine=${deployed[0]}; weth=${deployed[2]}; usdc=${deployed[3]}; trade_hash=${deployed[4]}
expiry=$(sed -n 's/^.*V18_EXPIRY[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
[[ "$expiry" =~ ^[0-9]+$ ]] || { echo 'missing expiry' >&2; exit 1; }
# Resolve actual test-wallet addresses from the script log, never use a guessed wallet.
buyer=$(sed -n 's/^.*V18_BUYER[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
seller=$(sed -n 's/^.*V18_SELLER[[:space:]]*//p' "$log" | tail -n 1 | tr -d '[:space:]')
[[ "$buyer" =~ ^0x[[:xdigit:]]{40}$ && "$seller" =~ ^0x[[:xdigit:]]{40}$ ]]
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
test "$(onchain_uint "$usdc" 'balanceOf(address)(uint256)' "$seller")" = 2450000000
deadline=$((expiry + 48 * 3600))
cast rpc evm_setNextBlockTimestamp "$deadline" --rpc-url "$url" > /dev/null
release_hash=$(cast send "$engine" 'releaseExpired(address,address,uint256)' "$buyer" "$seller" 1 --private-key "$seller_key" --rpc-url "$url" --json | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["status"]=="0x1"; print(r["transactionHash"])')
test "$(onchain_uint "$engine" 'totalLockedUnderlying()(uint256)')" = 0
test "$(onchain_uint "$weth" 'balanceOf(address)(uint256)' "$seller")" = 1000000000000000000
test "$(onchain_uint "$weth" 'balanceOf(address)(uint256)' "$engine")" = 0
test "$(onchain_uint "$usdc" 'balanceOf(address)(uint256)' "$buyer")" = 2550000000
export OPTIONS_V18_RPC_URL="$url"
export OPTIONS_V18_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$pg_port/deopt_options_v18"
export OPTIONS_V18_BROADCAST="$artifact"
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
