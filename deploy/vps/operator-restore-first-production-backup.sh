#!/usr/bin/env bash
# Operator-run only: decrypt off VPS and restore into a disposable local DB.
set -euo pipefail
set +x
umask 077
[[ $# == 2 ]] || { echo 'USAGE: ciphertext_file expected_sha256' >&2; exit 2; }
ciphertext=$1
expected_hash=$2
[[ "$ciphertext" == /* && -f "$ciphertext" && ! -L "$ciphertext" &&
   "$(stat -c %a "$ciphertext")" == 600 &&
   "$(stat -c %u "$ciphertext")" == "$EUID" &&
   "$expected_hash" =~ ^[0-9a-f]{64}$ ]] || {
  echo 'REVIEWED_LOCAL_CIPHERTEXT_REQUIRED' >&2; exit 2;
}
[[ "$(sha256sum "$ciphertext" | cut -d ' ' -f 1)" == "$expected_hash" ]] || {
  echo 'CIPHERTEXT_HASH_MISMATCH' >&2; exit 2;
}
secret_key_inventory="$(gpg --batch --with-colons --list-secret-keys 2>/dev/null)" || {
  echo 'LOCAL_GPG_KEYRING_UNAVAILABLE' >&2; exit 2;
}
[[ "$secret_key_inventory" == sec:* || "$secret_key_inventory" == *$'\nsec:'* ]] || {
  echo 'LOCAL_PRIVATE_GPG_KEY_UNAVAILABLE' >&2; exit 2;
}
container=deopt-production-restore-drill
db=deopt_prod_restore_test
image='postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a'
[[ "$(docker inspect "$container" --format '{{.Config.Image}}')" == "$image" &&
   "$(docker inspect "$container" --format '{{.HostConfig.NetworkMode}}')" == none &&
   "$(docker inspect "$container" --format '{{json .HostConfig.PortBindings}}')" == '{}' &&
   "$(docker inspect "$container" --format '{{.State.Running}}')" == true ]] || {
  echo 'ISOLATED_POSTGRES_TARGET_REQUIRED' >&2; exit 2;
}
tables="$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -d "$db" -c \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
[[ "$tables" == 0 ]] || { echo 'RESTORE_TARGET_NOT_EMPTY' >&2; exit 2; }
if ! gpg --quiet --decrypt --output /dev/null "$ciphertext"; then
  echo 'LOCAL_DECRYPTION_PREFLIGHT_FAILED' >&2
  exit 2
fi
gpg --quiet --decrypt --output - "$ciphertext" |
  docker exec -i "$container" pg_restore --single-transaction --exit-on-error \
    --no-owner --no-acl -U postgres --dbname "$db"
migrations="$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -d "$db" -c \
  'SELECT count(*) FROM _sqlx_migrations')"
tables="$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -d "$db" -c \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
[[ "$migrations" == 71 && "$tables" =~ ^[1-9][0-9]*$ ]] || {
  echo 'RESTORED_SCHEMA_MISMATCH' >&2; exit 2;
}
printf 'ISOLATED_POSTGRES_RESTORE=PASS\nRESTORED_MIGRATIONS=%s\nRESTORED_PUBLIC_TABLES=%s\n' \
  "$migrations" "$tables"
