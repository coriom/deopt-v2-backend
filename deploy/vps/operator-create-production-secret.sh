#!/usr/bin/env bash
# Operator-run on the reviewed Netcup host; never prints the password.
set +x
set -euo pipefail
umask 077
[[ ${EUID} == 0 && "$(hostname -s)" == 'v2202611448349549977' ]] || {
  echo 'Root on reviewed Netcup host required' >&2; exit 2;
}
secret_dir='/srv/deopt/production/secrets'
secret_file="$secret_dir/postgres_password"
[[ ! -L "$secret_dir" && ! -L "$secret_file" ]] || { echo 'Symlink refused' >&2; exit 2; }
install -d -o root -g root -m 0700 "$secret_dir"
if [[ ! -e "$secret_file" ]]; then
  ( set -C; openssl rand -hex 32 >"$secret_file" )
  chown root:1001 "$secret_file"
  chmod 0640 "$secret_file"
fi
[[ "$(stat -c '%u:%g:%a' "$secret_dir")" == '0:0:700' &&
   "$(stat -c '%u:%g:%a' "$secret_file")" == '0:1001:640' &&
   "$(wc -c <"$secret_file")" == 65 ]] || {
  echo 'Production secret identity or permissions invalid' >&2; exit 2;
}
printf 'PRODUCTION_SECRET_FILE=PREPARED\nSECRET_PATH=%s\nSECRET_FILE_OWNER_GROUP_MODE=0:1001:640\nSECRET_DIRECTORY_OWNER_GROUP_MODE=0:0:700\nSECRET_VALUE_PRINTED=NO\n' "$secret_file"
