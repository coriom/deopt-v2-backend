#!/usr/bin/env bash
# V2G-L2 — re-sync the local-stack Prometheus rule files from the
# canonical docs/monitoring/prometheus/ source.
#
# V2G-L0 originally shipped these as symlinks to avoid drift, but
# Docker bind-mounts do not follow symlinks whose target lives
# outside the bound directory tree — and the V2G-L0 symlinks pointed
# at `../../../prometheus/`, outside the `local-stack/prometheus/`
# bind. That made the prometheus container fail with
# `lstat /etc/../prometheus/v2_fee_alerts.bundle.yml: no such file or directory`.
#
# V2G-L2 replaces the symlinks with literal copies and ships this
# script so a single command re-syncs after every canonical edit.
#
# Usage:
#   ./sync_from_canonical.sh
#   # then reload Prometheus inside the compose stack:
#   docker compose -f ~/DEOPT/deopt-v2-backend/docs/monitoring/local-stack/compose.yml \
#     exec prometheus wget -qO- http://127.0.0.1:9090/-/reload >/dev/null \
#     && echo prometheus reloaded
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="${HERE}/../../../prometheus"

for f in v2_fee_alerts.bundle.yml v2_fee_alerts.stalled.yml protocol_fee_vault_alerts.yml; do
  cp -v "${SRC_DIR}/${f}" "${HERE}/${f}"
done

echo "Synced rules from ${SRC_DIR}"
