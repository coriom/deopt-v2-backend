#!/usr/bin/env bash
# Operator-only restore of an encrypted offsite dump into a precreated,
# empty, local disposable database. Never run against a production database.
set -euo pipefail

: "${DEOPT_BACKUP_OBJECT:?set reviewed encrypted offsite object}"
: "${DEOPT_RESTORE_DB:?set disposable test database name}"
: "${PGHOST:?set local disposable PostgreSQL host}"
: "${PGPORT:?set local disposable PostgreSQL port}"
: "${PGUSER:?set disposable PostgreSQL user}"
[[ "$DEOPT_BACKUP_OBJECT" == *:*.dump.gpg && "$DEOPT_BACKUP_OBJECT" != *$'\n'* ]] || {
  echo 'Reviewed encrypted dump object required' >&2; exit 2;
}
[[ "$DEOPT_RESTORE_DB" =~ ^deopt_[a-z0-9_]*_test$ ]] || {
  echo 'Disposable test database name required' >&2; exit 2;
}
[[ "$PGHOST" == 127.0.0.1 || "$PGHOST" == localhost || "$PGHOST" == ::1 ]] || {
  echo 'Restore target must be local' >&2; exit 2;
}
[[ "$PGPORT" =~ ^[0-9]{2,5}$ ]] || { echo 'Explicit local PostgreSQL port required' >&2; exit 2; }
for program in rclone gpg psql pg_restore; do
  command -v "$program" >/dev/null || { echo 'Restore tool missing' >&2; exit 2; }
done
tables="$(psql -X -A -t -v ON_ERROR_STOP=1 -d "$DEOPT_RESTORE_DB" -c \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema')")"
[[ "$tables" == 0 ]] || { echo 'Restore target is not empty' >&2; exit 2; }
rclone cat "$DEOPT_BACKUP_OBJECT" |
  gpg --batch --quiet --decrypt --output - |
  pg_restore --single-transaction --exit-on-error --no-owner --no-acl --dbname "$DEOPT_RESTORE_DB"
echo 'ISOLATED_RESTORE_COMMAND=PASS; LOGICAL_INTEGRITY_REVIEW_REQUIRED'
