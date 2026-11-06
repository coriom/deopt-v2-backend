#!/usr/bin/env bash
# Isolated PostgreSQL 16 check for migration 0074; no production connection.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
name="deopt-v15-registry-pg-$$"
cleanup() { docker stop "$name" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker run --rm -d --network none --memory 256m --cpus 1 --name "$name" \
  -e POSTGRES_PASSWORD=local-test-only postgres:16 >/dev/null
for _ in {1..45}; do
  if docker exec "$name" pg_isready -U postgres >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$name" pg_isready -U postgres >/dev/null
docker exec -i "$name" psql -v ON_ERROR_STOP=1 -U postgres <<'SQL' >/dev/null
CREATE TABLE option_series (
  option_series_id TEXT PRIMARY KEY,
  onchain_series_id TEXT
);
CREATE TABLE option_event_indexer_state (cursor_id TEXT PRIMARY KEY);
INSERT INTO option_series VALUES ('historical-opaque-key', '7');
SQL
docker exec -i "$name" psql -v ON_ERROR_STOP=1 -U postgres \
  < "$repo/migrations/0073_option_indexer_canonical_hash.sql" >/dev/null
docker exec -i "$name" psql -v ON_ERROR_STOP=1 -U postgres \
  < "$repo/migrations/0074_option_registry_identity.sql" >/dev/null
result=$(docker exec -i "$name" psql -v ON_ERROR_STOP=1 -At -U postgres <<'SQL'
INSERT INTO option_series(option_series_id,onchain_series_id,onchain_chain_id,onchain_registry_address)
VALUES
('0x0ffd65bace87c3ecbd455fa36283257624b7a3dcdfaa610f6f37d30dd1cbbf19','7',84532,'0x1111111111111111111111111111111111111111'),
('0x087dd8abfe553aafb24a76bcaf303238f31223b3fd9026d940d67b141adb6dbc','7',84532,'0x2222222222222222222222222222222222222222');
SELECT COUNT(*) FROM option_series WHERE onchain_series_id='7';
SELECT COUNT(*) FROM option_series WHERE option_series_id='historical-opaque-key'
  AND onchain_chain_id IS NULL;
DO $$
BEGIN
  BEGIN
    INSERT INTO option_series(option_series_id,onchain_series_id,onchain_chain_id,onchain_registry_address)
    VALUES ('noncanonical-duplicate','07',84532,'0x1111111111111111111111111111111111111111');
    RAISE EXCEPTION 'noncanonical ID accepted';
  EXCEPTION WHEN check_violation THEN NULL;
  END;
END $$;
SQL
)
[[ "$result" == $'INSERT 0 2\n3\n1\nDO' ]] || { echo 'V15_MIGRATION=FAIL'; exit 1; }
echo 'V15_MIGRATION=PASS'
echo 'SAME_RAW_ID_TWO_REGISTRIES=PASS'
echo 'HISTORICAL_ROW_PRESERVED=PASS'
