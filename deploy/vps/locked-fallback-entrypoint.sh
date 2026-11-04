#!/bin/sh
# Schema-72 compatible emergency API posture; keeps private wallet routes shut.
set -eu
export WALLET_READ_AUTH_ENABLED=false
exec /usr/local/bin/deopt-runtime-entrypoint
