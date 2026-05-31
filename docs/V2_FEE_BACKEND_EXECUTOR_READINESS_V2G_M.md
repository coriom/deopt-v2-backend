# V2G-M — V2 Fee Backend Executor Readiness

## Status

- Milestone: **V2G-M** — backend executor readiness + admin UI prep
  for V2 fee smokes through the backend/operator flow (no more ad
  hoc Solidity scripts).
- Date: 2026-05-31.
- Outcome:
  - **Audit complete.** `ExecutorSigner` + `PrivateKeySecret` already
    redact in Debug; `sign_perp_trade` already refuses on role address
    mismatch unless `--allow-address-mismatch`; `sign_option_execution_intent`
    refuses on buyer/seller mismatch built-in via `--private-key-env`.
    Broadcast surfaces gated by `EXECUTION_ENABLED` +
    `OPTION_EXECUTION_BROADCAST_ENABLED` + `EXECUTOR_REAL_BROADCAST_ENABLED`.
  - **New backend module `src/fees/smoke_readiness.rs`.** Pins the
    V2G-D2 EOAs as constants (addresses only), defines the unified
    `SmokeDryRunPacket` shape, ships `default_perp_packet` /
    `default_option_packet` builders that mirror the V2G-E live
    rebate trades, and `validate_numeric_invariants` that re-derives
    `feeAmount = ceil(basis * taker_ppm / 1e6)` /
    `rebateAmount = floor(basis * |maker_ppm| / 1e6)` /
    `expected_rebate_budget_delta = -rebateAmount` and refuses if
    `fee <= rebate` (protocol breaks even).
  - **New endpoint `GET /admin/fees/v2/smoke/readiness`.** Surfaces
    the engines snapshot, the V2G-D2 EOA registry, the
    fee-profile expectations, the dry-run packet skeletons, and the
    `broadcast_gates` snapshot (booleans only — never echoes a
    private key value). Refuses to mark the soak window safe if
    `active_perp_is_old_engine == true`.
  - **Frontend admin section.** New `V2FeeSmokeReadinessSection`
    auto-loads on token-ready, renders the EOA / engine / broadcast
    gate cards + per-product packet cards with explicit `null`
    placeholders for the numeric expectations. No write buttons, no
    wallet signing, no private-key input.
  - **15 new tests** (8 in `smoke_readiness.rs`, 7 in `routes.rs::tests`):
    address derivation never exposes keys, V2G-E reference math
    passes, mismatched fee math is rejected, broadcast gates default
    to safe, OLD-equals-active is refused, env-var values never echo
    into the JSON.
  - Backend: 679 → **694 passed**, 0 failed, 0 ignored.
  - Frontend lint / `tsc --noEmit` / `next build` all ✅.
  - **Soak preserved.** The running backend PID 56199 (V2G-G era
    binary) is untouched. Compiling new sources via `cargo
    build/test` produces a new `target/release/deopt-v2-backend`
    but does not restart the running one — the V2G-K /
    V2G-L0..L4 soak metrics history continues uninterrupted.
- Hard gates respected: no broadcast, no chain mutation, no DB
  writes, no real `.env` edit, no private-key handling, no
  governance/timelock action, no `docker compose down -v`, no
  Prometheus reset.

## Audit findings

| Surface | Path | Hygiene state |
|--------|------|---------------|
| `ExecutorSigner` | `src/execution/signer.rs` | `Debug` writes `"<redacted>"`; constructor consumes `&PrivateKeySecret` not `&str`. ✅ |
| `PrivateKeySecret` | `src/execution/config.rs` | wraps `String`; `Debug` writes `"<redacted>"`; only `expose_secret()` returns the value. ✅ |
| `EXECUTOR_PRIVATE_KEY` | `src/config/env.rs` | Only required when broadcast enabled; never logged. ✅ |
| `BUYER/SELLER/SIGNER_PRIVATE_KEY` | `src/bin/sign_perp_trade.rs` | Reads from env (`env_private_key`); falls back to `SIGNER_PRIVATE_KEY`; never echoed. ✅ |
| `--private-key-env <VAR>` | `src/bin/sign_option_execution_intent.rs` | Operator picks the env var name; CLI verifies signer-derived address matches buyer/seller in payload. ✅ |
| Broadcast gates | `src/execution/config.rs` + `src/options/types.rs` | `execution_enabled`, `dry_run`, `real_broadcast_enabled`, `option_execution_broadcast_enabled`. ✅ |

Existing CLIs already meet most of the V2G-M shell-only secret
hygiene requirements:

- **No committed keys** ✅ (all reads from `env::var`).
- **No `.env` secret edits** ✅ (operator shell-export pattern).
- **Derive addresses for sanity** ✅ (`ExecutorSigner::from_private_key` + `signer.address()`).
- **Refuse mismatch** ✅ (`validate_role_address` + buyer/seller check).
- **Never log private keys** ✅ (Debug-redacted everywhere).

What was **missing** before V2G-M:

1. A unified backend-side **readiness snapshot** that combines engines
   + EOAs + fee profile + gate state into a single read-only artefact
   the operator can consume from an HTTP endpoint (and the frontend
   can render).
2. An **explicit `OLD == active` refusal** at the readiness layer (the
   running backend can have `PERP_ENGINE_ADDRESS=OLD` set
   accidentally — the metric classifier catches it via the unknown
   bucket, but the readiness packet now refuses outright).
3. Canonical **dry-run packet templates** the operator builds against
   before broadcast, with numeric placeholders that explicitly start
   `null` so missing expectations fail loudly.

## Key hygiene design (V2G-M shell-only pattern)

### Shell-only key injection

The operator sources two env vars at smoke time. Names are pinned in
`smoke_readiness::{MAKER_KEY_ENV, TAKER_KEY_ENV}`:

```sh
# Operator workstation — keys never echoed, never written to disk.
export PERP_SMOKE_BUYER_PRIVATE_KEY=$(read-from-secret-store ...)
export PERP_SMOKE_SELLER_PRIVATE_KEY=$(read-from-secret-store ...)
```

### Address derivation + mismatch refusal

The standalone CLIs are the canonical signing path; V2G-M does NOT
add a new HTTP-side signing surface (which would be a step backward
on secret hygiene).

```sh
# Sign a PERP trade as the Tier 4 maker (V2G-D2):
PERP_SMOKE_BUYER_PRIVATE_KEY="$PERP_SMOKE_BUYER_PRIVATE_KEY" \
  cargo run --bin sign_perp_trade -- \
    --payload <path>/perp_maker_payload.json \
    --role buyer
# Refuses if signer-derived address != payload.message.buyer (Tier 4 maker).

# Sign an OPTION execution intent against an arbitrary env var name:
MAKER_PK="$PERP_SMOKE_BUYER_PRIVATE_KEY" \
  cargo run --bin sign_option_execution_intent -- \
    --payload-file <path>/maker_payload.json \
    --private-key-env MAKER_PK
# Refuses if signer-derived address is neither buyer nor seller.
```

### Never log private keys

`ExecutorSigner` + `PrivateKeySecret` Debug-impls render `"<redacted>"`.
The V2G-M endpoint emits only the env-var **names** and a boolean
"is set?" status — never the values. Tests
(`admin_v2_smoke_readiness_never_leaks_env_key_value`,
`smoke_packet_serialization_never_includes_a_private_key_word`) pin
this invariant.

## V2G-D2 EOA registry (addresses only, never keys)

| Role | Address | Tier | Env var name (operator picks the value) |
|------|---------|------|------------------------------------------|
| Maker | `0x290bd12c93e467bf51c51f5273d35bddb19e9274` | 4 | `PERP_SMOKE_BUYER_PRIVATE_KEY` |
| Taker | `0x77ca9dd6ccce2d692fb23877a2db7178807b0020` | 2 | `PERP_SMOKE_SELLER_PRIVATE_KEY` |

The constants live in
`src/fees/smoke_readiness.rs::{TIER4_MAKER_ADDRESS, TIER2_TAKER_ADDRESS,
MAKER_KEY_ENV, TAKER_KEY_ENV}` and are surfaced verbatim by the
readiness endpoint + frontend tile.

## Unified dry-run packet format

`SmokeDryRunPacket` (`src/fees/smoke_readiness.rs`):

```rust
pub struct SmokeDryRunPacket {
    pub milestone: &'static str,           // "V2G-M"
    pub product: ProductKind,              // PERP | OPTION
    pub flow: FlowKind,                    // ORDERBOOK | RFQ
    pub maker_address: String,             // lowercased
    pub taker_address: String,             // lowercased
    pub fee_consumer_address: String,      // NEW PerpEngine / NEW MarginEngine, lowercased
    pub fees_manager_v2_address: String,   // lowercased
    pub settlement_asset_address: String,  // lowercased mUSDC / equivalent
    pub maker_profile: FeeProfile,         // {tier, product, maker_ppm, taker_ppm}
    pub taker_profile: FeeProfile,
    pub basis_amount_native: Option<u128>,                 // operator fills
    pub expected_fee_amount_native: Option<u128>,          // operator fills
    pub expected_rebate_amount_native: Option<u128>,       // operator fills
    pub expected_rebate_budget_delta_native: Option<i128>, // operator fills (negative)
    pub maker_key_env: &'static str,       // env-var NAME only
    pub taker_key_env: &'static str,
    pub safe_to_broadcast_today: bool,     // false during soak
    pub notes: Vec<&'static str>,
}
```

Builders + invariants:

- `default_perp_packet(perp_engine_new, fees_manager_v2, settlement_asset)`
  pins the V2G-D2 EOAs, the Tier 4 / Tier 2 fee profiles, and the
  V2G-E ORDERBOOK flow defaults.
- `default_option_packet(margin_engine_new, fees_manager_v2, settlement_asset)`
  same shape for OPTION, with Tier 4 / Tier 2 OPTION fee profile.
- `validate_numeric_invariants(&packet)` re-derives the expected
  numeric fields and returns a list of findings. Empty list = packet
  is internally consistent. Catches:
  - missing `basis_amount_native`,
  - `fee != ceil(basis * taker_ppm / 1e6)`,
  - `rebate != floor(basis * |maker_ppm| / 1e6)`,
  - `expected_rebate_budget_delta_native != -expected_rebate_amount_native`,
  - `fee <= rebate` (protocol earns no net).

### V2G-E reference values (pinned in tests)

| Tx | basis | feePpm/rebatePpm | feeAmount | rebateAmount | budget delta |
|----|------:|------------------|-----------|--------------|--------------|
| PERP `0x5c15e923…` | 30 000 | taker 200 / maker −100 | 6 | 3 | −3 |
| OPTION `0x9a85cbce…` | 200 000 | taker 125 / maker −50 | 25 | 10 | −10 |

Both reference cases pass `validate_numeric_invariants` cleanly.

## Backend admin endpoint — `GET /admin/fees/v2/smoke/readiness`

Returns the aggregated readiness snapshot:

```jsonc
{
  "milestone": "V2G-M",
  "soak_safe_for_local_compose": true,
  "active_perp_is_old_engine": false,
  "engines": {
    "perp_engine_new": "0xc6c592...141c",
    "perp_engine_old": "0xb363...b53b",
    "margin_engine_new": "0x287cef...48cc",
    "margin_engine_old": "0x6c5665...b5f8",
    "fees_manager_v2": "0x00da0b...774f"
  },
  "smoke_eoas": {
    "tier4_maker_address": "0x290bd1...9274",
    "tier2_taker_address": "0x77ca9d...0020",
    "key_env_vars": {
      "maker": "PERP_SMOKE_BUYER_PRIVATE_KEY",
      "taker": "PERP_SMOKE_SELLER_PRIVATE_KEY"
    },
    "key_hygiene": [ /* never-print rules */ ]
  },
  "broadcast_gates": {
    "execution_enabled": false,
    "executor_dry_run": true,
    "executor_real_broadcast_enabled": false,
    "option_execution_broadcast_enabled": false,
    "executor_private_key_set": false,
    "maker_key_env_set": false,
    "taker_key_env_set": false,
    "maker_key_env_name": "PERP_SMOKE_BUYER_PRIVATE_KEY",
    "taker_key_env_name": "PERP_SMOKE_SELLER_PRIVATE_KEY"
  },
  "dry_run_packets": {
    "perp":   { /* default_perp_packet ... */ },
    "option": { /* default_option_packet ... */ }
  },
  "anomaly_totals": { "old_consumer_events": 0, "unknown_consumer_events": 0 },
  "metrics_snapshot": { /* V2 fee metric baseline */ }
}
```

Refusals:

- If `active_perp == old_perp` (operator misconfig), the endpoint
  still returns 200 but sets `active_perp_is_old_engine: true` and
  `soak_safe_for_local_compose: false`. Frontend tile renders the
  flag as red.

Admin-token gated — same auth path as the other `/admin/*` endpoints.

## Frontend — `V2FeeSmokeReadinessSection`

Renders, in this order:

1. **Status flags** — `safe_to_broadcast_today`,
   `active_perp_is_old_engine`, milestone, soak mode.
2. **Smoke EOAs (V2G-D2)** — Tier 4 maker / Tier 2 taker addresses +
   the env-var names + boolean "set?" status.
3. **Engine wiring** — NEW / OLD PerpEngine, NEW / OLD MarginEngine,
   FeesManagerV2. OLD tiles muted; NEW tiles warn when unset.
4. **Broadcast gates** — five boolean toggles, colour-coded:
   `executor_real_broadcast_enabled = true` → red, `executor_dry_run = true` → green.
5. **Dry-run packet skeletons** — per-product cards with explicit
   `null (operator fills)` placeholders for the trade-specific
   numerics.

Hard refusals on the UI:

- No write buttons.
- No wallet signing.
- No private-key input field.
- No copy-to-clipboard for any key-shaped string (we don't have one
  anyway — the section never embeds a key).

## Tests added

| Test | Asserts |
|------|---------|
| `default_perp_packet_uses_v2g_d2_eoas_and_lowercases_engines` | V2G-D2 addresses pinned; product / flow / env-var names correct; numeric fields start `None`; safety default off. |
| `default_option_packet_swaps_role_assignment_but_keeps_addresses` | OPTION packet uses same EOAs + OPTION fee profile. |
| `validate_numeric_invariants_v2g_e_perp_reference_passes` | V2G-E PERP reference math passes. |
| `validate_numeric_invariants_v2g_e_option_reference_passes` | V2G-E OPTION reference math passes. |
| `validate_numeric_invariants_rejects_missing_basis_amount` | Refuses null basis. |
| `validate_numeric_invariants_rejects_wrong_fee_math` | Refuses if `fee != ceil(basis * taker_ppm / 1e6)`. |
| `validate_numeric_invariants_rejects_budget_delta_mismatch` | Refuses if budget delta != −rebate. |
| `validate_numeric_invariants_rejects_protocol_breaks_even` | Refuses if `fee <= rebate`. |
| `smoke_packet_serialization_never_includes_a_private_key_word` | Cardinality contract: no key-shaped tokens in serialized packet. |
| `broadcast_gates_default_to_safe` | All gates false / dry-run on a fresh state. |
| `readiness_snapshot_refuses_to_mark_safe_when_active_equals_old` | `active_perp_is_old_engine=true` and `safe_to_broadcast_today=false` when active == OLD. |
| `admin_v2_smoke_readiness_returns_packet_with_default_eoas` | HTTP endpoint surfaces the pinned EOAs + env-var names; no key-shaped tokens in body. |
| `admin_v2_smoke_readiness_requires_admin_token_when_configured` | Admin token gate. |
| `admin_v2_smoke_readiness_refuses_when_active_equals_old` | Endpoint-level OLD-as-active refusal. |
| `admin_v2_smoke_readiness_never_leaks_env_key_value` | Even when the operator's env carries a secret-shaped value for the maker/taker env vars, the HTTP body never echoes it. |

All 15 new tests pass under cargo's parallel runner thanks to a
crate-test-internal `TEST_ENV_GUARD: Mutex<()>` serialising process-env
mutation.

## Files changed

Backend:

- **NEW** `src/fees/smoke_readiness.rs` (412 LOC + tests).
- `src/fees/mod.rs` — `pub mod smoke_readiness;`.
- `src/api/routes.rs` — new handler `admin_fees_v2_smoke_readiness` +
  route registration + 4 HTTP tests.
- **NEW** `docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` (this doc).
- `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` — V2G-M
  pointer note.
- `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` — V2G-M
  pointer note.

Frontend:

- `src/types/admin.ts` — `AdminFeeV2SmokeReadiness*` types.
- `src/lib/admin-api.ts` — `fetchAdminFeesV2SmokeReadiness` helper.
- `src/app/admin/admin-dashboard.tsx` — `V2FeeSmokeReadinessSection`
  + view + per-packet card + abort ref + auto-load effect.

Sol: untouched.

## Validation

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **694 / 0 / 0** |
| `npm run lint` (frontend) | ✅ |
| `npx tsc --noEmit` (frontend) | ✅ |
| `npm run build` (frontend) | ✅ |
| Soak: `/health`, `/-/healthy`, 4/4 compose containers running | ✅ throughout |

## Soak stack status

The running backend PID 56199 + the 4-container compose stack
remained healthy across every cargo / npm invocation. No
`docker compose down`, no Prometheus reset. The soak record at
`docs/V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` does NOT get a new
soak day ticked — V2G-M is unrelated to the calendar gate. The
canonical day-1 24h marker remains reserved for `2026-06-01T17:38Z`.

## Remaining blockers

1. **Operator must restart the backend at a maintenance window to
   pick up the new endpoint.** The new
   `GET /admin/fees/v2/smoke/readiness` only serves once the
   binary is restarted with the V2G-M build. Until then it returns
   `404` (confirmed against the running V2G-G binary in this
   milestone).
2. **`v2_fee_alerts.stalled.yml`** still deferred (V2G-K day 5
   re-eval).
3. **V2G-J target-host F1-F11 cutover** still pending.

## Next recommended milestone

**V2G-N — operator restarts the backend during a soak-maintenance
window and exercises the new V2G-M endpoint + frontend tile against
the live local-compose stack.**

- Stop / re-launch the backend with the same V2G-K shell-override
  stack (the V2G-K daily checklist accepts a single non-destructive
  restart as a "warn" not "fail" — operator records the restart
  timestamp in the soak row).
- Confirm `/admin/fees/v2/smoke/readiness` returns 200 with the
  documented payload.
- Confirm the new "V2 Fee Smoke Readiness (V2G-M)" admin tile
  renders against the running backend.
- Optionally: try a Tier 4 maker / Tier 2 taker shell-key export
  pattern and confirm the endpoint flips
  `maker_key_env_set=true` / `taker_key_env_set=true` without
  echoing the key.
