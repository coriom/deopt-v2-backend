#!/usr/bin/env bash
# V2G-H — render the V2 fee observability dashboard JSON with the
# Prometheus datasource name baked in, ready for Grafana provisioning.
#
# Usage:
#
#   render_dashboard.sh <prom_datasource_name> > /var/lib/grafana/dashboards/deopt/v2_fee_observability_dashboard.json
#
# Reasoning: Grafana's dashboard provisioning loader does NOT substitute
# the `${DS_PROMETHEUS}` template input that the committed dashboard
# JSON ships with — that input is only resolved at UI import time. For
# a provisioned dashboard the datasource name needs to be embedded in
# the file before it lands in the provisioning folder.
#
# This is a read-only file transformation. It does not touch Grafana.

set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "usage: $0 <prom_datasource_name>" >&2
  echo "example: $0 Prometheus" >&2
  exit 64
fi

DS="$1"

# Locate the source dashboard relative to this script's directory.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="${HERE}/../v2_fee_observability_dashboard.json"

if [[ ! -f "${SRC}" ]]; then
  echo "error: cannot find ${SRC}" >&2
  exit 66
fi

# Replace every `${DS_PROMETHEUS}` reference with the supplied
# datasource name. Done in pure sed (no jq dependency) so the script
# works on minimal operator hosts.
sed "s/\\\${DS_PROMETHEUS}/${DS}/g" "${SRC}"
