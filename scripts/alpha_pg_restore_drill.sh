#!/usr/bin/env bash
# Disposable local-only PostgreSQL dump/restore acceptance. Never aim at a
# production database. Both databases must already exist and be test-only.
set -euo pipefail

: "${DEOPT_RESTORE_SOURCE_URL:?set an isolated source URL}"
: "${DEOPT_RESTORE_TARGET_URL:?set an empty isolated target URL}"
source_name="${DEOPT_RESTORE_SOURCE_URL##*/}"
target_name="${DEOPT_RESTORE_TARGET_URL##*/}"
if [[ ! "$source_name" =~ ^deopt_[a-z0-9_]*_test$ ]] ||
   [[ ! "$target_name" =~ ^deopt_[a-z0-9_]*_test$ ]] ||
   [[ "$source_name" == "$target_name" ]]; then
  echo "Refusing non-disposable or identical database names" >&2
  exit 2
fi
if [[ "$(psql -X -A -t "$DEOPT_RESTORE_TARGET_URL" -c "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")" != "0" ]]; then
  echo "Refusing nonempty restore target" >&2
  exit 2
fi
tmpdir="$(mktemp -d /tmp/deopt-restore-drill.XXXXXXXX)"
trap 'rm -rf "$tmpdir"' EXIT
pg_dump --format=custom --no-owner --no-acl --file "$tmpdir/backup.dump" "$DEOPT_RESTORE_SOURCE_URL"
test -s "$tmpdir/backup.dump"
pg_restore --exit-on-error --no-owner --no-acl --dbname "$DEOPT_RESTORE_TARGET_URL" "$tmpdir/backup.dump"

read -r -d '' integrity_sql <<'SQL' || true
SELECT md5(concat_ws('|',
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(order_id || ':' || status || ':' || remaining_size_1e8, ',' ORDER BY order_id)), '') FROM orders),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(id::text || ':' || status || ':' || remaining_size_1e8, ',' ORDER BY id)), '') FROM perp_orders),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(id::text || ':' || size_1e8, ',' ORDER BY id)), '') FROM perp_fills),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(id::text || ':' || size_1e8 || ':' || status, ',' ORDER BY id)), '') FROM perp_positions),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(event_id || ':' || coalesce(block_hash, ''), ',' ORDER BY event_id)), '') FROM indexed_perp_trades),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(chain_id::text || ':' || name || ':' || last_indexed_block::text || ':' || coalesce(last_indexed_block_hash, ''), ',' ORDER BY chain_id, name)), '') FROM indexer_cursors),
  (SELECT count(*)::text || ':' || coalesce(md5(string_agg(fee_event_id || ':' || fee_amount_1e8, ',' ORDER BY fee_event_id)), '') FROM fee_events)
));
SQL
source_hash="$(psql -X -A -t "$DEOPT_RESTORE_SOURCE_URL" -c "$integrity_sql")"
target_hash="$(psql -X -A -t "$DEOPT_RESTORE_TARGET_URL" -c "$integrity_sql")"
if [[ "$source_hash" != "$target_hash" ]]; then
  echo "Restore logical-state digest mismatch" >&2
  exit 1
fi
# Reprocessing an already committed chain event must not create a duplicate.
psql -X -q "$DEOPT_RESTORE_TARGET_URL" -c \
  "INSERT INTO indexed_perp_trades SELECT * FROM indexed_perp_trades ON CONFLICT DO NOTHING" >/dev/null
replayed_hash="$(psql -X -A -t "$DEOPT_RESTORE_TARGET_URL" -c "$integrity_sql")"
if [[ "$replayed_hash" != "$target_hash" ]]; then
  echo "Duplicate replay changed restored logical state" >&2
  exit 1
fi
printf 'RESTORE_INTEGRITY=PASS\nLOGICAL_STATE_SHA_MD5=%s\nDUPLICATE_REPLAY=NO_CHANGE\n' "$target_hash"
