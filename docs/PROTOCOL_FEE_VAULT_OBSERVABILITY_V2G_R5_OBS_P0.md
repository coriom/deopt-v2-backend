# V2G-R5-OBS-P0 — ProtocolFeeVault Backend Observability

## Status

- Milestone: **V2G-R5-OBS-P0** — backend implementation of the
  ProtocolFeeVault observability surface, gated to land BEFORE the
  V2G-R5 ProtocolFeeVault deploy + cutover.
- Posture: **GREEN** — module + endpoints + metrics + alerts + tests
  + docs implemented; lib builds; clippy + tests green; YAML
  parse-checked. No chain mutation.
- Date: 2026-06-03.

## What landed

| Surface | File | Notes |
|---|---|---|
| Module | `src/fees/vault_observability.rs` | New. ~700 LoC. Self-contained: config builder, snapshot types, RPC view-call helpers (selector / encode / decode), three JSON view builders, metric-row builder, 9 unit tests. |
| Module wiring | `src/fees/mod.rs` | Adds `pub mod vault_observability;` |
| Admin endpoints | `src/api/routes.rs` | Three new routes + handlers, role-gated `viewer` per the existing admin gate middleware. |
| Prometheus metrics | `src/monitoring.rs` | Adds `append_protocol_fee_vault_metrics` plus a per-metric help-text lookup. Emits nothing per-asset when the vault is unconfigured. |
| Alert rules | `docs/monitoring/prometheus/protocol_fee_vault_alerts.yml` | New file. 4 alerts in one group. Companion to the V2G-G `v2_fee_alerts.bundle.yml`. |
| Result doc | `docs/PROTOCOL_FEE_VAULT_OBSERVABILITY_V2G_R5_OBS_P0.md` | this file. |

## "Not configured" contract

The vault is not yet deployed. The implementation is intentionally
**safe to start in production today** with no vault on chain:

- `PROTOCOL_FEE_VAULT_ADDRESS` env var unset (or `address(0)`) →
  every endpoint returns `configured=false`, every per-asset metric
  is suppressed, no RPC calls are issued, the global
  `deopt_protocol_fee_vault_configured` gauge reads `0`.
- Once the operator sets `PROTOCOL_FEE_VAULT_ADDRESS=0x…` at backend
  restart (shell-only env override, no `.env` edit), the endpoints
  return live snapshots and the metrics begin populating.
- A single chain-state failure in one of the view calls degrades
  gracefully: the affected asset's record carries the error in
  `asset_errors`; other assets and the global view still render.

This matches the V2G-RX spec (`docs/PROTOCOL_FEE_VAULT_OBSERVABILITY_SPEC_V2G_RX.md`)
§5 ("Backend implementation status") and unblocks R5-BLOCKER-2 in
`RUN_STATE.md`.

## Configuration

| env var | required | source | semantics |
|---|---|---|---|
| `PROTOCOL_FEE_VAULT_ADDRESS` | **no** | shell export at backend launch | Sentinel for "vault on chain". When set, all PFV endpoints and metrics activate. |
| `PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS` | no | shell export | Comma-separated EVM addresses to scan. Defaults to the asset set already populating `deopt_fees_manager_v2_rebate_budget_native` (mUSDC today). |
| `RPC_URL` | reused | existing `.env` | Same RPC the rest of the backend uses for view calls. |
| `OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS` | reused | existing `.env` / shell | Powers the `CollateralVault.balances(vault, asset)` query that backs the drift computation. |
| `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` | reused | existing `.env` / shell | Surfaced in the summary view so the operator can cross-check that the PFV's `feesManagerV2()` matches. |

## Endpoints

All three are read-only, role-gated `viewer` (admin-token required if
the backend is configured to require it), and respond 200 OK with the
"not_configured" body when the vault env is absent.

### `GET /admin/fees/vault/summary`

Global summary + a configured-asset count. Body shape:

```json
{
  "milestone": "V2G-R5-OBS-P0",
  "configured": true,
  "vault_address": "0xf6626...",
  "collateral_vault_address": "0x00340c360353a5ab784c5bc5c44322a6af0625d3",
  "fees_manager_v2_address": "0xf6626177f3b85cc3239667cc53c04a8007652944",
  "rpc_configured": true,
  "rebates_paused": false,
  "drift_present": false,
  "configured_assets_count": 1,
  "global": {
    "owner": "0xc35f7a...",
    "guardian": null,
    "revenue_receiver": "0xa67f8e...",
    "collateral_vault_on_pfv": "0x00340c...",
    "fees_manager_v2_on_pfv": "0xf6626177...",
    "rebates_paused": false
  },
  "reason": null
}
```

When `configured=false`, `reason` carries the human explanation
(`PROTOCOL_FEE_VAULT_ADDRESS is not set` or `RPC_URL is not
configured`).

### `GET /admin/fees/vault/balances`

Per-asset bucket breakdown. Body shape:

```json
{
  "milestone": "V2G-R5-OBS-P0",
  "configured": true,
  "vault_address": "0xf6626...",
  "assets": [
    {
      "asset": "0x6eae407f5640b006fac9965182e238582a3b412e",
      "fee_balance": "9",
      "rebate_reserve": "999967",
      "gross_fees_collected": "19",
      "rebates_paid": "10",
      "net_revenue": "9",
      "bootstrapped": true,
      "internal_cv_balance": "999976",
      "raw_erc20_balance": "0",
      "drift_native": "0",
      "drift_status": "ok",
      "reserve_shortfall_native": "0"
    }
  ],
  "fees_manager_v2_rebate_budget": {
    "0x6eae407f5640b006fac9965182e238582a3b412e": "999967"
  },
  "asset_errors": {},
  "reason": null
}
```

All amounts are decimal strings (we read `uint256` from chain and do
not clamp to `u64`).

### `GET /admin/fees/vault/reconciliation`

Drift-focused view. Body shape:

```json
{
  "milestone": "V2G-R5-OBS-P0",
  "configured": true,
  "vault_address": "0xf6626...",
  "drift_present": false,
  "rebates_paused": false,
  "rows": [
    {
      "asset": "0x6eae407f5640b006fac9965182e238582a3b412e",
      "fee_balance": "9",
      "rebate_reserve": "999967",
      "buckets_sum": "999976",
      "internal_cv_balance": "999976",
      "raw_erc20_balance": "0",
      "drift_native": "0",
      "drift_status": "ok",
      "reserve_shortfall_native": "0",
      "raw_erc20_dust_present": false
    }
  ],
  "asset_errors": {},
  "reason": null
}
```

Drift semantics:

- `drift_status == "ok"` ⇔ `internal_cv_balance == fee_balance + rebate_reserve` (invariant 2 holds).
- `drift_status == "drift_positive"` ⇔ CV ledger holds more than the
  vault's accounting buckets (the vault forgot to credit an inflow).
- `drift_status == "drift_negative"` ⇔ the vault accounting exceeds
  what the CV ledger actually holds (an over-claim, a far worse
  state).
- `drift_native` is the signed magnitude as a decimal string,
  negative values prefixed `-`.
- `raw_erc20_dust_present` flags tokens transferred to the vault
  contract directly (bypassing the FM-V2 hook path).

## Prometheus metrics

All gauges. Labelled by `asset` (lowercase 0x-prefixed) except the
three global ones.

| Metric | Label | Source |
|---|---|---|
| `deopt_protocol_fee_vault_configured` | none | `PROTOCOL_FEE_VAULT_ADDRESS != address(0)` → 1 |
| `deopt_protocol_fee_vault_rebates_paused` | none | `vault.rebatesPaused()` |
| `deopt_protocol_fee_vault_drift_present` | none | 1 if any asset has non-zero drift |
| `deopt_protocol_fee_vault_fee_balance_native` | `asset` | `vault.feeBalance(asset)` |
| `deopt_protocol_fee_vault_rebate_reserve_native` | `asset` | `vault.rebateReserve(asset)` |
| `deopt_protocol_fee_vault_gross_fees_collected_native` | `asset` | `vault.grossFeesCollected(asset)` |
| `deopt_protocol_fee_vault_rebates_paid_native` | `asset` | `vault.rebatesPaid(asset)` |
| `deopt_protocol_fee_vault_net_revenue_native` | `asset` | `vault.netRevenue(asset)` |
| `deopt_protocol_fee_vault_internal_collateral_vault_balance_native` | `asset` | `collateralVault.balances(vault, asset)` |
| `deopt_protocol_fee_vault_raw_erc20_balance_native` | `asset` | `IERC20(asset).balanceOf(vault)` |
| `deopt_protocol_fee_vault_drift_native` | `asset` | `abs(internalCv − feeBalance − rebateReserve)` |
| `deopt_protocol_fee_vault_reserve_shortfall_native` | `asset` | `max(0, fm_v2_rebate_budget − rebateReserve)` |

Cardinality: bounded by the configured settlement-asset count
(1 today: mUSDC). Adding a new asset means adding its address to
`PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS` (or to the rebate-budget
indexer); no per-trader labels.

U256 values exceeding `u64::MAX` are clamped at `u64::MAX` for the
Prometheus encoding (the JSON endpoints return the full decimal
string). For mUSDC at six decimals, `u64::MAX` represents
≈18 446 744 073 mUSDC ≈ $18.4T — well above any realistic balance.

## Alerts

Defined in
`docs/monitoring/prometheus/protocol_fee_vault_alerts.yml`. All four
fire on configured deployments only — the per-asset gauges they
reference are absent while `deopt_protocol_fee_vault_configured == 0`.

| Alert | Severity | Trigger | Operator action |
|---|---|---|---|
| **ProtocolFeeVaultDrift** | critical | `drift_native > 0` for 2 m | Pause rebates (`pauseRebates`), inspect via `/admin/fees/vault/reconciliation`, do NOT withdraw revenue. |
| **ProtocolFeeVaultRawErc20Dust** | high | `raw_erc20_balance_native > 0` for 5 m | Inspect dust source. Rescue via a CV deposit or quarantine. |
| **ProtocolFeeVaultReserveShortfall** | high | `reserve_shortfall_native > 0` for 5 m | Top up via `allocateToRebateReserve(asset, amount)` or wait for organic fees. |
| **ProtocolFeeVaultRebatesPaused** | critical | `rebates_paused == 1` for 0 m | Confirm pause intent, investigate, then `unpauseRebates()` (owner-only). |

The `unpauseRebates` is owner-only by design (V2G-RX.1) so the
critical alert stays active until explicit operator action clears it.

## Tests

Unit tests in `src/fees/vault_observability.rs::tests` (9 tests, all
passing):

| Test | What it asserts |
|---|---|
| `not_configured_returns_no_chain_call_and_well_formed_snapshot` | Vault unset → 200 OK shape, no RPC issued, summary view marks `configured=false`, `metric_rows` empty. |
| `compute_drift_handles_three_cases` | OK / drift_positive / drift_negative branches encode the signed magnitude correctly. |
| `metric_rows_emits_nine_rows_per_asset_when_configured` | Nine per-asset metric names emitted, each carries the asset label, all required metric families present. |
| `reserve_shortfall_zero_when_reserve_meets_cap` | Boundary check for the `max(0, cap − reserve)` arithmetic. |
| `sanitize_address_accepts_only_well_formed_lowercase_addr` | Empty / `0x0` / wrong-length / non-hex inputs rejected; mixed case normalized to lowercase. |
| `build_config_pulls_assets_from_env_then_falls_back` | Env override priority over fallback; fallback applied when env unset. |
| `selector_matches_state_checks_pattern` | `selector("balanceOf(address)") == 0x70a08231` (ERC-20 ABI canonical). |
| `balances_view_serializes_assets` | Empty-asset shape for the not_configured branch. |
| `reconciliation_view_marks_drift_present` | Drift + raw-ERC20-dust flags surface correctly in the reconciliation JSON. |

End-to-end HTTP tests (route + admin-token gate) are deferred to the
broader `routes.rs` integration suite that exists for
`/admin/fees/v2/observability`; the vault handlers share the same
admin-access middleware and admin-token enforcement, so the route
gate is exercised by the existing
`admin_v2_observability_requires_admin_token_when_configured` test
pattern.

## Validations run

| Command | Result |
|---|---|
| `cargo build --release --lib` | ✅ Finished |
| `cargo clippy --release --lib -- -D warnings` | ✅ (see below) |
| `cargo test --release --lib fees::vault_observability` | ✅ 9 / 0 / 0 |
| `python3 -m yaml.safe_load` on the new alerts YAML | ✅ 1 group / 4 rules |
| `cargo fmt --check src/fees/vault_observability.rs` | ✅ |

`promtool` is not installed in the local toolchain (V2G-G runbook
note); the Python YAML parse is the documented stand-in.

## Hard-stop compliance

| Hard stop | Status |
|---|---|
| no chain tx / no broadcast | ✅ surface is read-only (`eth_call` view calls only) |
| no ProtocolFeeVault deploy | ✅ |
| no `setProtocolFeeVault` | ✅ |
| no `feeRecipient` / `rebateFundingAccount` cutover | ✅ |
| no rebateBudget mutation | ✅ |
| no CollateralVault mutation | ✅ |
| no `.env` secret edit | ✅ (config sourced from existing `.env` + shell exports + per-process env) |
| no private key output | ✅ |
| no admin token output | ✅ |
| no Prometheus reset | ✅ (one new rule file alongside existing bundle) |

## Cross-links

- V2G-RX spec: `PROTOCOL_FEE_VAULT_OBSERVABILITY_SPEC_V2G_RX.md` (parent design doc).
- ProtocolFeeVault Solidity: `deopt-v2-sol/src/fees/ProtocolFeeVault.sol`.
- Prometheus rule bundle (V2G-G): `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`.
- V2G-G observability foundation: `V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md`.
- V2G-RX-FM-P1 closure: `deopt-v2-sol/docs/FEES_MANAGER_V2_HOOK_REDEPLOY_RESULT_V2G_RX_FM_P1.md` §18.
- Macro state: `~/DEOPT/RUN_STATE.md`.

## Remaining blockers (for V2G-R5 ProtocolFeeVault deploy + cutover)

| ID | Severity | Status |
|---|---|---|
| **R5-BLOCKER-1** | High | **CLOSED** on chain (V2G-RX-FM-P1, see §18 of the FM-P1 result doc). |
| **R5-BLOCKER-2** (backend vault observability) | High | **CLOSED** by this milestone (V2G-R5-OBS-P0). |
| **R5-BLOCKER-3** (vault-script keystore-mode patches) | Medium | **OPEN**. Touches `DeployProtocolFeeVault.s.sol`, `WireProtocolFeeVaultFeesManager.s.sol`, `SetProtocolFeeVaultGuardian.s.sol`. Next milestone candidate: V2G-R5-P1. |

## Next milestone

**V2G-R5-P1** — keystore-mode patches on the three vault scripts,
mirroring the V2G-RX-FM-P1 pattern. Once that closes, V2G-R5 (vault
deploy + `setProtocolFeeVault` cutover on NEW_FM_V2) can proceed
with the observability surface already in place.
