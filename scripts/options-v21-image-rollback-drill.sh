#!/usr/bin/env bash
# Disposable, local-only restore and candidate -> rollback -> candidate image drill.
set -euo pipefail
set +x
umask 077
[[ $# == 2 ]] || { echo 'usage: ciphertext expected-sha256' >&2; exit 2; }
ciphertext=$1 expected=$2
[[ "$ciphertext" == /* && -f "$ciphertext" && ! -L "$ciphertext" &&
   "$(stat -c %a "$ciphertext")" == 600 && "$expected" =~ ^[0-9a-f]{64}$ &&
   "$(sha256sum "$ciphertext" | cut -d' ' -f1)" == "$expected" ]] || exit 2
[[ "${GNUPGHOME:-}" == /home/corio/.local/share/deopt-backup-gpg ]] || exit 2
[[ "$(hostname -s)" != v2202611448349549977 ]] || exit 2
for tool in docker gpg python3 curl cast openssl; do command -v "$tool" >/dev/null || exit 2; done
gpg --batch --quiet --decrypt --output /dev/null "$ciphertext" 2>/dev/null || {
  echo 'LOCAL_GPG_UNLOCK_REQUIRED' >&2; exit 2;
}
tag="deopt-v21-drill-$$"
network="$tag-net"
pg="$tag-pg"
api="$tag-api"
work=$(mktemp -d /tmp/deopt-v21-drill.XXXXXX)
cleanup() {
  docker rm -f "$api" "$pg" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf -- "$work"
}
trap cleanup EXIT
port=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    print(sock.getsockname()[1])
PY
)
docker network create "$network" >/dev/null
docker run --rm -d --name "$pg" --network "$network" --network-alias postgres \
  --memory 768m --cpus 1 -e POSTGRES_PASSWORD=deopt-local-test-only \
  -e POSTGRES_DB=deopt_readonly postgres:16 -c fsync=off -c synchronous_commit=off >/dev/null
for _ in {1..60}; do
  docker exec "$pg" pg_isready -U postgres -d deopt_readonly >/dev/null 2>&1 && break
  sleep 1
done
docker exec "$pg" pg_isready -U postgres -d deopt_readonly >/dev/null
gpg --batch --quiet --decrypt --output - "$ciphertext" |
  docker exec -i "$pg" pg_restore --single-transaction --exit-on-error --no-owner --no-acl \
    -U postgres -d deopt_readonly >/dev/null
sql() { docker exec "$pg" psql -X -q -A -t -v ON_ERROR_STOP=1 -U postgres -d deopt_readonly -c "$1"; }
baseline=$(sql 'SELECT COUNT(*) FROM _sqlx_migrations')
[[ "$baseline" == 72 ]] || { echo "BASELINE_MIGRATIONS=$baseline" >&2; exit 2; }
fingerprint() {
  python3 - "$pg" <<'PY'
import hashlib, subprocess, sys
container=sys.argv[1]
base=['docker','exec',container,'psql','-X','-q','-A','-t','-v','ON_ERROR_STOP=1','-U','postgres','-d','deopt_readonly','-c']
def query(s): return subprocess.run(base+[s],check=True,capture_output=True,text=True).stdout.strip()
tables=query("SELECT table_name FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE' AND table_name NOT IN ('_sqlx_migrations','wallet_read_challenges','wallet_read_sessions','wallet_read_rate') ORDER BY table_name").splitlines()
parts=[]
for table in tables:
    name='"'+table.replace('"','""')+'"'
    parts.append(table+'='+query(f"SELECT COUNT(*), md5(COALESCE(string_agg(row_to_json(t)::text, E'\\n' ORDER BY row_to_json(t)::text),'')) FROM public.{name} t"))
print(len(tables),sum(int(p.split('=')[1].split('|')[0])>0 for p in parts),hashlib.sha256('\n'.join(parts).encode()).hexdigest())
PY
}
before=$(fingerprint)
cutoff_ms=$(date +%s%3N)
cutoff_minute=$((cutoff_ms / 60000))
auth_snapshot() {
  sql "SELECT COUNT(*), md5(COALESCE(string_agg(row_to_json(t)::text, E'\\n' ORDER BY row_to_json(t)::text),'')) FROM (SELECT * FROM wallet_read_challenges WHERE issued_at_ms < $cutoff_ms) t"
  sql "SELECT COUNT(*), md5(COALESCE(string_agg(row_to_json(t)::text, E'\\n' ORDER BY row_to_json(t)::text),'')) FROM (SELECT * FROM wallet_read_sessions WHERE issued_at_ms < $cutoff_ms) t"
  sql "SELECT COUNT(*), md5(COALESCE(string_agg(row_to_json(t)::text, E'\\n' ORDER BY row_to_json(t)::text),'')) FROM (SELECT * FROM wallet_read_rate WHERE minute_bucket < $cutoff_minute) t"
}
[[ "$(sql 'SELECT COALESCE(MAX(issued_at_ms),0) FROM wallet_read_challenges')" -lt "$cutoff_ms" ]] || exit 2
[[ "$(sql 'SELECT COALESCE(MAX(issued_at_ms),0) FROM wallet_read_sessions')" -lt "$cutoff_ms" ]] || exit 2
[[ "$(sql 'SELECT COALESCE(MAX(minute_bucket),0) FROM wallet_read_rate')" -lt "$cutoff_minute" ]] || exit 2
auth_before=$(auth_snapshot)
db_url='postgres://postgres:deopt-local-test-only@postgres:5432/deopt_readonly'
docker run --rm --network "$network" --memory 512m --cpus 1 \
  -e DATABASE_URL="$db_url" -e CHAIN_ID=84532 -e DEOPT_READ_ONLY_RELEASE=true \
  deopt-v21-candidate:local --migrate-only >/dev/null
[[ "$(sql 'SELECT COUNT(*) FROM _sqlx_migrations')" == 74 ]] || exit 2
[[ "$(fingerprint)" == "$before" ]] || { echo 'HISTORICAL_DATA_CHANGED' >&2; exit 2; }
sql "CREATE ROLE deopt_readonly LOGIN PASSWORD 'deopt-local-test-only'" >/dev/null
sql 'GRANT CONNECT ON DATABASE deopt_readonly TO deopt_readonly; GRANT USAGE ON SCHEMA public TO deopt_readonly; GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA public TO deopt_readonly; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO deopt_readonly' >/dev/null
printf '%s' 'deopt-local-test-only' > "$work/password"
chmod 0444 "$work/password"
start_api() {
  local image=$1 entry=$2
  local flags=()
  [[ -z "$entry" ]] || flags+=(--entrypoint "$entry" -v "$work/password:/run/secrets/postgres_password:ro")
  docker run --rm -d --name "$api" --network "$network" -p "127.0.0.1:$port:8080" \
    --memory 768m --cpus 1 "${flags[@]}" \
    -e CHAIN_ID=84532 -e PERSISTENCE_ENABLED=true -e DEOPT_READ_ONLY_RELEASE=true \
    -e DATABASE_URL='postgres://deopt_readonly:deopt-local-test-only@postgres:5432/deopt_readonly' \
    -e WALLET_READ_AUTH_ENABLED=true -e WALLET_READ_AUTH_ORIGIN=https://deoptv2-frontend.vercel.app \
    -e OPTIONS_ENABLED=false -e EXECUTION_ENABLED=false -e INDEXER_ENABLED=false \
    "$image" >/dev/null
  for _ in {1..50}; do
    curl --fail --silent "http://127.0.0.1:$port/ready" >/dev/null && return 0
    sleep 1
  done
  echo "IMAGE_START_FAILED=$image" >&2
  docker logs "$api" 2>&1 | tail -15 >&2
  return 1
}
stop_api() { docker stop "$api" >/dev/null; }
probe() {
  [[ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/auth/wallet/session")" == 401 ]] || return 1
  [[ "$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $token" "http://127.0.0.1:$port/auth/wallet/session")" == 200 ]] || return 1
  [[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$port/options/orders")" == 405 ]] || return 1
}
start_api deopt-v21-candidate:local ''
key="0x$(openssl rand -hex 32)"
wallet=$(cast wallet address --private-key "$key")
challenge=$(curl --fail --silent -H 'content-type: application/json' -d "{\"address\":\"$wallet\"}" "http://127.0.0.1:$port/auth/wallet/challenge")
nonce=$(printf '%s' "$challenge" | python3 -c 'import json,sys;print(json.load(sys.stdin)["nonce"])')
message=$(printf '%s' "$challenge" | python3 -c 'import json,sys;print(json.load(sys.stdin)["message"])')
signature=$(cast wallet sign --private-key "$key" "$message")
unset key message
verify=$(curl --fail --silent -H 'content-type: application/json' -d "{\"nonce\":\"$nonce\",\"signature\":\"$signature\"}" "http://127.0.0.1:$port/auth/wallet/verify")
unset signature nonce
token=$(printf '%s' "$verify" | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
unset challenge verify
probe
stop_api
start_api deopt-v21-rollback:local /usr/local/bin/deopt-readonly-entrypoint
probe
stop_api
start_api deopt-v21-candidate:local ''
probe
stop_api
unset token
[[ "$(sql 'SELECT COUNT(*) FROM _sqlx_migrations')" == 74 ]] || exit 2
[[ "$(fingerprint)" == "$before" ]] || { echo 'HISTORICAL_DATA_CHANGED' >&2; exit 2; }
[[ "$(auth_snapshot)" == "$auth_before" ]] || { echo 'HISTORICAL_AUTH_DATA_CHANGED' >&2; exit 2; }
echo "REAL_BACKUP_BASELINE_MIGRATIONS=$baseline"
echo 'SCHEMA_74_CANDIDATE_ROLLBACK_CANDIDATE=PASS'
echo 'WALLET_AUTH_PERSISTENCE=PASS'
echo 'ECONOMIC_HTTP_WRITE_GUARD=PASS'
echo "HISTORICAL_FINGERPRINT_PRESERVED=$before"
echo 'HISTORICAL_AUTH_ROWS_PRESERVED=PASS'
