#!/bin/sh
# Schema-75 emergency read-only API posture. Wallet reads remain authenticated.
set -eu
export DEOPT_READ_ONLY_RELEASE=true
export EXECUTION_ENABLED=false
export EXECUTOR_REAL_BROADCAST_ENABLED=false
export HV2_BROADCAST_ENABLED=false
export PERPS_PUBLIC_TRADING_ENABLED=false
export PERPS_CLOSED_TEST_ENABLED=false
export OPTIONS_ENABLED=false
export OPTION_EXECUTION_ENABLED=false
export OPTION_EXECUTION_BROADCAST_ENABLED=false
export INDEXER_ENABLED=false
export OPTION_EVENT_INDEXER_ENABLED=false
exec /usr/local/bin/deopt-runtime-entrypoint
