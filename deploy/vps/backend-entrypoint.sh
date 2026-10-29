#!/bin/sh
# Invoked by the read-only Compose profile; no secret is printed or embedded.
set -eu
test "${DEOPT_READ_ONLY_RELEASE:-}" = true || exit 2
test "${CHAIN_ID:-}" = 84532 || exit 2
password_file=/run/secrets/postgres_password
test -r "$password_file" || exit 2
password=$(cat "$password_file")
case "$password" in
    ''|*[!A-Za-z0-9._~-]*)
        echo "Database password must be nonempty and URL-safe" >&2
        exit 2
        ;;
esac
export DATABASE_URL="postgres://deopt_readonly:${password}@postgres:5432/deopt_readonly"
unset password
exec /usr/local/bin/deopt-v2-backend
