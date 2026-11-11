#!/usr/bin/env bash
# Local-only encrypted real-data migration drill. Never targets Netcup.
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
for tool in docker gpg python3 cargo; do command -v "$tool" >/dev/null || exit 2; done
if ! gpg --batch --quiet --decrypt --output /dev/null "$ciphertext" 2>/dev/null; then
  echo 'LOCAL_GPG_UNLOCK_REQUIRED' >&2
  exit 2
fi
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
name="deopt-options-v20-real-pg-$$"
port=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    print(sock.getsockname()[1])
PY
)
work=$(mktemp -d /tmp/deopt-options-v20-restore.XXXXXX)
started=0
cleanup() {
  if (( started )); then docker stop "$name" >/dev/null 2>&1 || true; fi
  rm -rf -- "$work"
}
trap cleanup EXIT
docker run --rm -d --name "$name" -p "127.0.0.1:$port:5432" \
  --memory 768m --cpus 1 -e POSTGRES_PASSWORD=deopt-local-test-only \
  -e POSTGRES_DB=deopt_options_v20_real postgres:16 \
  -c fsync=off -c synchronous_commit=off >/dev/null
started=1
for _ in {1..60}; do
  if docker exec "$name" pg_isready -U postgres -d deopt_options_v20_real >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$name" pg_isready -U postgres -d deopt_options_v20_real >/dev/null
# First unlock via the operator's WSL terminal; only decrypted bytes enter disposable pg_restore.
gpg --batch --quiet --decrypt --output - "$ciphertext" |
  docker exec -i "$name" pg_restore --single-transaction --exit-on-error --no-owner --no-acl \
    -U postgres -d deopt_options_v20_real >/dev/null
fingerprint() {
  python3 - "$name" "$1" <<'PY'
import hashlib, json, subprocess, sys
container, path = sys.argv[1:]
base = ['docker','exec',container,'psql','-X','-q','-A','-t','-U','postgres','-d','deopt_options_v20_real']
def query(sql):
    return subprocess.run(base + ['-c', sql], check=True, capture_output=True, text=True).stdout.strip()
if path.endswith('before.json'):
    rows = query("SELECT table_name, column_name FROM information_schema.columns WHERE table_schema='public' ORDER BY table_name, ordinal_position")
    tables = {}
    for line in rows.splitlines():
        table, col = line.split('|')
        if table == '_sqlx_migrations': continue
        tables.setdefault(table, []).append(col)
else:
    with open(path.replace('after.json','before.json'), encoding='utf8') as stream:
        tables = json.load(stream)['columns']
def quoted(name): return '"' + name.replace('"','""') + '"'
hashes = {}
for table, columns in tables.items():
    select = ','.join(quoted(col) for col in columns)
    sql = (f'SELECT COUNT(*), md5(COALESCE(string_agg(rowdata, E\'\\n\' ORDER BY rowdata), \'\')) '
           f'FROM (SELECT row_to_json(t)::text AS rowdata FROM '
           f'(SELECT {select} FROM public.{quoted(table)}) t) x')
    hashes[table] = query(sql)
with open(path, 'w', encoding='utf8') as stream:
    json.dump({'columns':tables, 'hashes':hashes}, stream, sort_keys=True)
PY
}
fingerprint "$work/before.json"
export OPTIONS_V20_REAL_PG_URL="postgres://postgres:deopt-local-test-only@127.0.0.1:$port/deopt_options_v20_real"
(cd "$repo" && CARGO_BUILD_JOBS=1 cargo test --test options_v20_real_migration -- --ignored --nocapture)
fingerprint "$work/after.json"
python3 - "$work/before.json" "$work/after.json" <<'PY'
import json, sys
with open(sys.argv[1], encoding='utf8') as stream: before=json.load(stream)
with open(sys.argv[2], encoding='utf8') as stream: after=json.load(stream)
changed=[name for name, value in before['hashes'].items() if after['hashes'].get(name) != value]
assert not changed, 'historical table content changed: ' + ','.join(changed)
print('REAL_HISTORICAL_TABLES_PRESERVED=' + str(len(before['hashes'])))
print('REAL_NONEMPTY_TABLES=' + str(sum(int(value.split('|')[0]) > 0 for value in before['hashes'].values())))
print('REAL_DATA_0074_MIGRATION=PASS')
PY
