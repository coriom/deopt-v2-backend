#!/usr/bin/env bash
# Future approved cutover only. Run after PostgreSQL uses the reviewed socket profile.
set -euo pipefail
set +x
umask 077
[[ $EUID == 0 && "$(hostname -s)" == v2202611448349549977 ]] || exit 2
[[ $# == 2 ]] || { echo 'usage: script reviewed-image-digest source-commit' >&2; exit 2; }
image=$1; commit=$2
[[ "$image" =~ ^ghcr\.io/coriom/deopt-v2-backend@sha256:[0-9a-f]{64}$ &&
   "$commit" =~ ^[0-9a-f]{40}$ ]] || exit 2
[[ "$(docker image inspect "$image" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$commit" ]] || {
  echo 'Image source revision differs' >&2; exit 2;
}
directory=$(cd -- "$(dirname -- "$0")" && pwd -P)
[[ "$(stat -c '%u:%a' "$directory")" == '0:700' ]] || exit 2
post_sql="$directory/postgres-least-privilege-post.sql"
[[ ! -L "$post_sql" && "$(stat -c '%u:%a' "$post_sql")" == '0:600' ]] || exit 2
project=dxco90jwpdxyennmquv2elyj
container=$(docker ps --filter "label=com.docker.compose.project=$project" \
  --filter 'label=com.docker.compose.service=postgres' --format '{{.ID}}')
[[ "$container" =~ ^[0-9a-f]{12,64}$ &&
   "$(docker inspect "$container" --format '{{.HostConfig.NetworkMode}}')" == none ]] || {
  echo 'Socket-only PostgreSQL required' >&2; exit 2;
}
volume="${project}_postgres_socket"
[[ "$(docker inspect "$container" --format '{{range .Mounts}}{{if eq .Destination "/var/run/postgresql"}}{{.Name}}{{end}}{{end}}')" == "$volume" ]] || exit 2
secret=/srv/deopt/production/secrets/migrator_password
[[ ! -L "$secret" && "$(stat -c '%u:%g:%a' "$secret")" == '0:1001:640' ]] || exit 2
admin_psql() {
  docker exec -i "$container" sh -ec '
    PGPASSWORD=$(cat /run/secrets/postgres_password)
    export PGPASSWORD
    exec psql -X -q -v ON_ERROR_STOP=1 -U deopt_readonly -d deopt_readonly "$@"
  ' sh "$@"
}
[[ "$(admin_psql -A -t -c 'SELECT count(*) FROM _sqlx_migrations')" == 71 ]] || exit 2
docker run --rm --pull never --network none --user 1001:1001 --group-add 999 --read-only \
  --pids-limit 64 --memory 256m --cap-drop ALL --security-opt no-new-privileges \
  -v "$volume:/var/run/postgresql:ro" \
  -v "$secret:/run/secrets/migrator_password:ro" \
  -e CHAIN_ID=84532 -e DEOPT_READ_ONLY_RELEASE=true \
  --entrypoint /bin/sh "$image" -ec '
    password=$(cat /run/secrets/migrator_password)
    export DATABASE_URL="postgres://deopt_migrator:$password@%2Fvar%2Frun%2Fpostgresql/deopt_readonly"
    unset password
    exec /usr/local/bin/deopt-v2-backend --migrate-only
  '
[[ "$(admin_psql -A -t -c 'SELECT count(*) FROM _sqlx_migrations')" == 72 ]] || exit 2
admin_psql <"$post_sql"
printf 'MIGRATION_0072=PASS\nRUNTIME_GRANTS=APPLIED\n'
