# V2G-J — Backend `.env` patch packet

This packet is the **operator-runnable** version of the V2G-F /
V2G-G env patch. The agent did NOT edit the real `.env` file — the
V2G hard rules forbid it.

## What this patch does

The metric classifier in `src/monitoring.rs` reads five env vars at
boot. If the vars are wrong, **every V2 fee event lands in the
`consumer="unknown"` bucket** and the V2G-G OLD-engine /
unknown-consumer alerts cannot reliably fire. The patch flips
`PERP_ENGINE_ADDRESS` from the V2F-O OLD carry-over to NEW and adds
the four observability metadata vars that V2G-F / V2G-G introduced.

## Patch values

```ini
# Base Sepolia V2G-band canonical addresses.
PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
```

## Pre-condition checks (read-only, no mutation)

```sh
# 1. Confirm the backend binary is up-to-date with the V2G band.
~/DEOPT/deopt-v2-backend/target/release/deopt-v2-backend --version 2>&1 \
  | head -1 || true
# (Optional: build is current if the V2G-G test count is 679.)

# 2. Snapshot the current .env state (path placeholders mirror
# questionnaire §E2).
ENV_PATH="${ENV_PATH:-$HOME/DEOPT/deopt-v2-backend/.env}"
grep -nE '^(PERP_ENGINE_ADDRESS|OLD_PERP_ENGINE_ADDRESS|MARGIN_ENGINE|OLD_MARGIN_ENGINE_ADDRESS|FEES_MANAGER_V2)=' "$ENV_PATH" || \
  echo "(no V2G classifier vars set yet)"

# 3. Confirm the file is gitignored — never commit a .env.
git -C ~/DEOPT/deopt-v2-backend check-ignore -v "$ENV_PATH"
# Expected: .gitignore line referencing /.env or .env

# 4. Confirm the agent did NOT print private keys here.
grep -cE 'PRIVATE_KEY' "$ENV_PATH" || true   # may be > 0; do not paste
```

## Apply (one-shot, idempotent-ish)

```sh
ENV_PATH="${ENV_PATH:-$HOME/DEOPT/deopt-v2-backend/.env}"
BAK_PATH="${ENV_PATH}.bak.$(date -u +%Y%m%dT%H%M%SZ)"

# 1. Backup BEFORE editing.
cp -p "$ENV_PATH" "$BAK_PATH"
echo "backup at $BAK_PATH"

# 2. Flip PERP_ENGINE_ADDRESS to NEW (idempotent — if already NEW, the
# sed is a no-op).
sed -i.tmp \
  's@^PERP_ENGINE_ADDRESS=.*@PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c@' \
  "$ENV_PATH"
rm -f "${ENV_PATH}.tmp"

# 3. Append the four observability vars if they are not already
# present.
needed=(
  "OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B"
  "MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc"
  "OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8"
  "FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f"
)
{
  printf '\n# --- V2G-J observability classifier (added %s) ---\n' "$(date -u +%FT%TZ)"
  for kv in "${needed[@]}"; do
    key="${kv%%=*}"
    if ! grep -qE "^${key}=" "$ENV_PATH"; then
      printf '%s\n' "$kv"
    fi
  done
} >> "$ENV_PATH"

echo "patch applied — restart the backend to pick up new env vars."
```

## Verification (only after applying)

```sh
ENV_PATH="${ENV_PATH:-$HOME/DEOPT/deopt-v2-backend/.env}"

# 1. Confirm every required var is present and pointing at NEW.
grep -nE '^(PERP_ENGINE_ADDRESS|OLD_PERP_ENGINE_ADDRESS|MARGIN_ENGINE|OLD_MARGIN_ENGINE_ADDRESS|FEES_MANAGER_V2)=' "$ENV_PATH"
# Expected:
#   PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
#   OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
#   MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
#   OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
#   FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f

# 2. Re-confirm PERP_ENGINE_ADDRESS is NEW, not OLD.
new="0xc6C592100723Fe0C66343A16e95eC34cC0c2141c"
old="0xB36395b67D0798ADA981731c9Fa5239F4362b53B"
val="$(grep -E '^PERP_ENGINE_ADDRESS=' "$ENV_PATH" | head -1 | cut -d= -f2-)"
case "$val" in
  "$new") echo "OK: PERP_ENGINE_ADDRESS is NEW" ;;
  "$old") echo "FAIL: PERP_ENGINE_ADDRESS still points at OLD — abort cutover" ; exit 1 ;;
  *)      echo "FAIL: PERP_ENGINE_ADDRESS is unexpected value: $val"           ; exit 1 ;;
esac

# 3. Restart the backend.
# Pick the right line per your deployment shape (Section E3 of the
# questionnaire):
#   systemctl restart deopt-backend
#   docker compose restart backend
#   kubectl rollout restart deployment/deopt-backend -n deopt

# 4. After restart, hit the V2G-G admin probe (admin token from $E4).
ADMIN_TOKEN="${ADMIN_TOKEN:?set ADMIN_TOKEN before running}"
BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:8080}"
curl -sH "x-admin-token: ${ADMIN_TOKEN}" \
  "${BACKEND_URL}/admin/fees/v2/observability" \
  | jq '.contracts, .anomaly_totals, .metrics'
# Expected on Base Sepolia post-V2G-E:
#   contracts.perp_engine_new   == 0xc6C592...141c
#   contracts.perp_engine_old   == 0xB363...b53B
#   contracts.margin_engine_new == 0x287Cef...48Cc
#   contracts.margin_engine_old == 0x6C5665...b5F8
#   contracts.fees_manager_v2   == 0x00dA0B...774f
#   anomaly_totals.old_consumer_events     == 0
#   anomaly_totals.unknown_consumer_events == 0
#   metrics.*_by_consumer.new == 3 (PERP charged / OPTION charged)
#                            or 1 (PERP rebated / OPTION rebated)
#   metrics.*_by_consumer.old     == 0
#   metrics.*_by_consumer.unknown == 0
#   metrics.fees_manager_v2_rebate_budget_native["0x6eae..."] == 999987

# 5. Scrape /metrics and confirm Prometheus sees the same numbers.
curl -sH "x-admin-token: ${ADMIN_TOKEN}" "${BACKEND_URL}/metrics" \
  | grep -E '^deopt_(perp|option)_fee_(charged|rebated)_v2_total|^deopt_fees_manager_v2_rebate_budget_native'
```

## Rollback

```sh
ENV_PATH="${ENV_PATH:-$HOME/DEOPT/deopt-v2-backend/.env}"
# Pick the most recent backup created by the apply step.
BAK_PATH="$(ls -1t "${ENV_PATH}".bak.* | head -1)"
cp -p "${BAK_PATH}" "${ENV_PATH}"
echo "restored from ${BAK_PATH}"

# Restart the backend per your deployment shape.
```

## Safety warnings

- **NEVER point `PERP_ENGINE_ADDRESS` at the OLD address.** The OLD
  PerpEngine is stranded under the V2F-A3 fallback and
  `FeesManagerV2.isFeeConsumer(OLD) == false` — every broadcast
  routed at it will revert. The V2G-G `PerpFeeChargedFromOldEngine`
  alert depends on OLD being observability-only.
- The four observability vars **never** route broadcast or execution
  traffic. They are read by `src/monitoring.rs` and
  `src/fees/v2_observability.rs` only.
- The patch does **not** touch any `*_PRIVATE_KEY` or admin-token
  variable. If your `.env` carries private keys, they remain in
  place; the agent does not read or print them.
- The verification curl uses an admin token. Source the token from
  your secret manager (`$ADMIN_TOKEN`), do not paste it inline.
- Do not run this patch against a `.env` that lives in a Git working
  tree under a different repo — the `sed` is keyed on the exact
  `PERP_ENGINE_ADDRESS=` line at column 0.
