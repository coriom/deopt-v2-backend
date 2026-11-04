#!/usr/bin/env bash
# Future approved cutover only. Creates separate DB credentials without printing them.
set -euo pipefail
set +x
umask 077

[[ $EUID == 0 && "$(hostname -s)" == v2202611448349549977 ]] || {
  echo 'Reviewed Netcup root context required' >&2; exit 2;
}
directory=$(cd -- "$(dirname -- "$0")" && pwd -P)
[[ "$(stat -c '%u:%a' "$directory")" == '0:700' ]] || {
  echo 'Root-owned staging directory required' >&2; exit 2;
}
for name in postgres-least-privilege-pre.sql postgres-least-privilege-ownership.sql; do
  [[ ! -L "$directory/$name" && "$(stat -c '%u:%a' "$directory/$name")" == '0:600' ]] || {
    echo 'Root-owned reviewed SQL required' >&2; exit 2;
  }
done
project=dxco90jwpdxyennmquv2elyj
container=$(docker ps --filter "label=com.docker.compose.project=$project" \
  --filter 'label=com.docker.compose.service=postgres' --format '{{.ID}}')
[[ "$container" =~ ^[0-9a-f]{12,64}$ ]] || { echo 'Expected one production PostgreSQL' >&2; exit 2; }
[[ "$(docker inspect "$container" --format '{{.Config.Image}}')" == \
  'postgres:16@sha256:5db6dac2ef5dcff512f4f00a009c3c332b9f2f28f24277175bf3e4a3d02ef83a' &&
  "$(docker inspect "$container" --format '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}')" == "${project}_postgres_data" ]] || {
  echo 'Production image or data volume differs' >&2; exit 2;
}
admin_psql() {
  docker exec -i "$container" sh -ec '
    PGPASSWORD=$(cat /run/secrets/postgres_password)
    export PGPASSWORD
    exec psql -X -q -v ON_ERROR_STOP=1 -U deopt_readonly -d deopt_readonly "$@"
  ' sh "$@"
}
[[ "$(admin_psql -A -t -c 'SELECT count(*) FROM _sqlx_migrations')" == 71 ]] || {
  echo 'Expected 71 migrations before cutover' >&2; exit 2;
}
[[ "$(admin_psql -A -t -c 'SHOW log_statement')" == none ]] || {
  echo 'SQL statement logging must be disabled before credential setup' >&2; exit 2;
}
secret_dir=/srv/deopt/production/secrets
[[ ! -L "$secret_dir" && "$(stat -c '%u:%g:%a' "$secret_dir")" == '0:0:700' ]] || {
  echo 'Production secret directory differs' >&2; exit 2;
}
for role in runtime migrator backup; do
  file="$secret_dir/${role}_password"
  [[ ! -L "$file" ]] || { echo 'Secret symlink refused' >&2; exit 2; }
  if [[ ! -e "$file" ]]; then
    ( set -C; openssl rand -hex 32 >"$file" )
  fi
  group=1001; mode=640
  if [[ "$role" == backup ]]; then group=0; mode=600; fi
  chown "root:$group" "$file"
  chmod "0$mode" "$file"
  [[ "$(stat -c '%u:%g:%a' "$file")" == "0:$group:$mode" ]] || exit 2
  [[ "$(cat "$file")" =~ ^[0-9a-f]{64}$ ]] || { echo 'Secret format invalid' >&2; exit 2; }
done
admin_psql <"$directory/postgres-least-privilege-pre.sql"
{
  printf "SET password_encryption = 'scram-sha-256';\n"
  for role in runtime migrator backup; do
    password=$(cat "$secret_dir/${role}_password")
    printf "ALTER ROLE deopt_%s LOGIN PASSWORD '%s';\n" "$role" "$password"
    unset password
  done
} | admin_psql
admin_psql <"$directory/postgres-least-privilege-ownership.sql"
printf 'LEAST_PRIVILEGE_ROLES=PREPARED\nSECRETS_PRINTED=NO\nPRODUCTION_MIGRATION=NOT_PERFORMED\n'
