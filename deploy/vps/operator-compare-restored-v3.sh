#!/usr/bin/env bash
# Run personally in WSL with Pinentry; never sends a private key to the VPS.
set -euo pipefail
set +x
umask 077
[[ $# == 2 ]] || { echo 'usage: ciphertext expected-sha256' >&2; exit 2; }
ciphertext=$1; expected=$2
[[ "$ciphertext" == /* && -f "$ciphertext" && ! -L "$ciphertext" &&
   "$(stat -c %a "$ciphertext")" == 600 &&
   "$expected" =~ ^[0-9a-f]{64}$ &&
   "$(sha256sum "$ciphertext" | cut -d ' ' -f 1)" == "$expected" ]] || exit 2
[[ "${GNUPGHOME:-}" == /home/corio/.local/share/deopt-backup-gpg && -t 0 ]] || {
  echo 'Reviewed local GPG keyring and interactive TTY required' >&2; exit 2;
}
container=deopt-production-restore-drill
[[ "$(docker inspect "$container" --format '{{.Config.Image}}')" == \
   'postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a' &&
   "$(docker inspect "$container" --format '{{.HostConfig.NetworkMode}}')" == none ]] || exit 2
baseline=deopt_prod_baseline_test
migrated=deopt_prod_restore_test
baseline_tables=$(docker exec "$container" psql -X -A -t -U postgres -d "$baseline" -c \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
if [[ "$baseline_tables" == 0 ]]; then
  gpg-connect-agent updatestartuptty /bye >/dev/null
  gpg --quiet --decrypt --output /dev/null "$ciphertext"
  gpg --batch --quiet --decrypt --output - "$ciphertext" |
    docker exec -i "$container" pg_restore --single-transaction --exit-on-error \
      --no-owner --no-acl -U postgres --dbname "$baseline"
elif [[ "$baseline_tables" != 93 ]]; then
  echo 'Unexpected disposable baseline state' >&2; exit 2
fi
baseline_migrations=$(docker exec "$container" psql -X -A -t -U postgres -d "$baseline" \
  -c 'SELECT count(*) FROM _sqlx_migrations')
migrated_migrations=$(docker exec "$container" psql -X -A -t -U postgres -d "$migrated" \
  -c 'SELECT count(*) FROM _sqlx_migrations')
[[ "$baseline_migrations" == 71 && "$migrated_migrations" == 72 ]] || exit 2
[[ "$(docker exec "$container" psql -X -A -t -U postgres -d "$migrated" -c \
  "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")" == 96 ]] || exit 2
data_hash() {
  docker exec "$container" pg_dump --data-only --no-owner --no-acl \
    --exclude-table='_sqlx_migrations' --exclude-table='wallet_read_*' \
    -U postgres -d "$1" |
    awk '/^COPY / {copy=1; print; next} copy {print; if ($0=="\\.") copy=0}
         /^SELECT pg_catalog.setval/ {print}' |
    sha256sum | cut -d ' ' -f 1
}
[[ "$(data_hash "$baseline")" == "$(data_hash "$migrated")" ]] || {
  echo 'REAL_DATA_COMPARISON=FAIL' >&2; exit 1;
}
printf 'REAL_DATA_COMPARISON=PASS\nBASELINE_MIGRATIONS=71\nMIGRATED_MIGRATIONS=72\nLEGACY_COPY_AND_SEQUENCE_DATA=EQUAL\n'
