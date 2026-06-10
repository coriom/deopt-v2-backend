# BACKEND-LIVE-PROVIDER-PFV-CONFIG — result

**Status:** SHIPPED 2026-06-09.
**Scope:** add a first-class typed config path for the `ProtocolFeeVault`
address; plumb it from env loader through `OptionEventIndexerConfig`
into the runtime `LiveBroadcastPolicyDataProvider`; expose a
configured-status gauge to `/metrics`. **No mainnet tx. No live
broadcast. No `.env` edit. No webhook secrets.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/docs/BACKEND_LIVE_PROVIDER_PFV_CONFIG_RESULT.md` —
  this close-out doc.

### Modified (5)

- `src/options/event_indexer.rs`:
  - `OptionEventIndexerConfig::protocol_fee_vault_address: Option<AccountId>`
    new field.
  - `OptionEventIndexerConfig::disabled()` initializes the new field
    to `None`.
  - 2 test fixture constructions in the existing test mod extended.
- `src/config/env.rs`:
  - Env loader reads (in order): `PROTOCOL_FEE_VAULT_ADDRESS` →
    `OPTION_EVENT_INDEXER_PROTOCOL_FEE_VAULT_ADDRESS` →
    `PROTOCOL_FEE_VAULT`; first non-empty value wins.
  - Address shape validated via existing
    `signing::eip712::parse_evm_address` at config load; malformed
    input → `BackendError::Config("invalid PROTOCOL_FEE_VAULT_ADDRESS: …")`.
  - Empty string treated as not-set (no malformed-address error).
  - 7 new env-loader tests.
- `src/options/service.rs`:
  - `build_runtime_policy_data_provider` sources `pfv_address` from
    `state.option_event_indexer_config.protocol_fee_vault_address`
    instead of hardcoded `None`.
  - 2 new integration tests verifying the PFV pass-through.
- `src/monitoring.rs`:
  - 3 new typed-config-state gauges in
    `append_broadcast_observability_metrics` complementing the
    existing `vault_observability` layer's
    `deopt_protocol_fee_vault_configured`:
    - `deopt_option_broadcast_protocol_fee_vault_configured`.
    - `deopt_option_broadcast_fees_manager_v2_configured`.
    - `deopt_option_broadcast_collateral_vault_configured`.
- `src/api/routes.rs`:
  - 3 test-fixture `OptionEventIndexerConfig` literal constructions
    extended with `protocol_fee_vault_address: None`.

## 2. Current PFV config gap (closed)

Prior to this PR, the runtime helper hardcoded `pfv_address = None`
because no env key was wired into a typed config field. PFV reads
(`PFV.feeBalance(asset)` + `PFV.rebateReserve(asset)`) were silently
skipped at the LiveProvider layer in production traffic.

Cluster 4 launch-invariant primary teeth (`policy:rebate-reserve`)
still fired correctly because the call-site default for
`pfv_rebate_reserve_asset` is `0` on mainnet, but the **chain-side
backstop** (live read of `PFV.rebateReserve(asset)`) was inactive.

This PR closes that gap: when `PROTOCOL_FEE_VAULT_ADDRESS` is
configured, the runtime helper threads the address into the
LiveProvider, which then issues the `feeBalance(asset)` +
`rebateReserve(asset)` eth_calls. The Cluster 4 primary teeth now have
their **chain-side backstop** wired end-to-end.

## 3. Env / config field added

| Aspect              | Detail                                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------------------------ |
| Canonical env key   | `PROTOCOL_FEE_VAULT_ADDRESS`                                                                                 |
| Alternate keys      | `OPTION_EVENT_INDEXER_PROTOCOL_FEE_VAULT_ADDRESS` (namespaced), `PROTOCOL_FEE_VAULT` (short alias).           |
| Resolution order    | First non-empty match wins (canonical → namespaced → short alias).                                            |
| Validation          | `signing::eip712::parse_evm_address` at config load. Malformed → `BackendError::Config("invalid PROTOCOL_FEE_VAULT_ADDRESS: …")` (process refuses to start). |
| Empty string        | Treated as unset; no malformed-address error.                                                                 |
| Default if unset    | `None`. No permissive default. Production posture unchanged from prior milestone.                             |
| Config field        | `OptionEventIndexerConfig::protocol_fee_vault_address: Option<AccountId>`.                                    |

## 4. Runtime provider wiring

`build_runtime_policy_data_provider<P>(state, provider)` now sources
PFV from typed config:

```rust
let pfv_address = state
    .option_event_indexer_config
    .protocol_fee_vault_address
    .clone();
LiveBroadcastPolicyDataProvider::new(provider, pfv_address, cv_address, fm_v2_address)
    .with_observability(state.broadcast_observability.clone())
```

When set:
- LiveProvider issues `PFV.feeBalance(asset)` + `PFV.rebateReserve(asset)`
  eth_calls on every broadcast attempt.
- Failures increment `policy_data_failures_total{read_type="pfv_fee_balance"}`
  + `policy_data_failures_total{read_type="pfv_rebate_reserve"}`.
- CV(PFV, asset) R5 precheck now operates with a real PFV address.
- Cluster 4 launch invariant primary teeth — `policy:rebate-reserve`
  reject on rebate-positive intent — now fires off the live read.

When unset:
- PFV reads silently skipped at the LiveProvider's
  `if let Some(pfv) = self.pfv_address.as_ref()` branch.
- `inputs.pfv_rebate_reserve_asset` stays `None` → call site defaults
  to `0` on mainnet → `policy:rebate-reserve` still fires on any
  rebate-positive intent (Cluster 4 default-0 fail-closed posture
  unchanged from prior milestone).

## 5. Status / visibility change

Three new typed-config-state gauges in `/metrics`:

| Metric                                                       | Description                                                                                    |
| ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `deopt_option_broadcast_protocol_fee_vault_configured`       | `1` if `PROTOCOL_FEE_VAULT_ADDRESS` is set via typed config; `0` if PFV reads are skipped.    |
| `deopt_option_broadcast_fees_manager_v2_configured`          | `1` if `FEES_MANAGER_V2` address is set; `0` if `quoteFees` + `rebateBudget` reads are skipped. |
| `deopt_option_broadcast_collateral_vault_configured`         | `1` if `COLLATERAL_VAULT` address is set; `0` if `CV(PFV,asset)` R5 precheck read is skipped.  |

These complement the pre-existing `deopt_protocol_fee_vault_configured`
gauge from the `vault_observability` layer (which reads
`std::env::var` directly at metric render time). The new gauges
expose the typed-config view used by the runtime broadcast path so
operators can verify the two layers agree.

No new high-cardinality labels. No raw addresses or secrets emitted.

## 6. Fail-closed behavior preserved

- **Malformed env key** → process refuses to start (`BackendError::Config`).
- **Empty / unset env key** → `None` flows through; mainnet
  `policy:rebate-reserve` still fires via the default-0 path.
- **Valid env key + RPC failure** → `pfv_rebate_reserve_asset = None` →
  mainnet `policy:rebate-reserve` fires via the default-0 path AND
  `policy_data_failures_total{pfv_rebate_reserve}` counter increments
  (alertable from `/metrics`).
- **Valid env key + decode failure** → same as RPC failure.
- **No silent permissive default introduced.** All policy semantics
  unchanged from prior milestones.
- **Remote signer cannot be called before policy approval** — order
  unchanged; signer remains uncontacted on any policy reject.

## 7. Tests added (11 new)

### Env loader (7 in `config::env::tests`)

- `protocol_fee_vault_address_absent_yields_none` — no env key set →
  typed field stays `None`.
- `protocol_fee_vault_address_canonical_env_key_parses` —
  `PROTOCOL_FEE_VAULT_ADDRESS` → field populated.
- `protocol_fee_vault_address_namespaced_env_key_parses` —
  `OPTION_EVENT_INDEXER_PROTOCOL_FEE_VAULT_ADDRESS` → populated.
- `protocol_fee_vault_address_short_alias_parses` →
  `PROTOCOL_FEE_VAULT` → populated.
- `protocol_fee_vault_address_invalid_hex_rejects` — non-hex string →
  config error with `"invalid PROTOCOL_FEE_VAULT_ADDRESS"` message.
- `protocol_fee_vault_address_short_hex_rejects` — `0xabc` →
  config error with `"invalid PROTOCOL_FEE_VAULT_ADDRESS"`.
- `protocol_fee_vault_address_empty_string_yields_none` — empty
  string → `None` (treat as unset, not malformed).

### Runtime helper (2 in `options::service::tests`)

- `build_runtime_policy_data_provider_threads_pfv_address_into_live_reads`
  — set PFV via state; mock fails every eth_call → snapshot records
  `policy_data_failures_total{pfv_fee_balance} == 1` +
  `policy_data_failures_total{pfv_rebate_reserve} == 1`. Proves
  the typed-config plumbing now drives the LiveProvider's PFV branch.
- `build_runtime_policy_data_provider_skips_pfv_reads_when_address_unset`
  — PFV not configured → no PFV failure counter increments. Pins
  backwards-compat with the prior milestone posture.

### Cross-cutting

The existing 11 broadcast / observability / FM_V2 tests pass unchanged.
The 3 test-fixture `OptionEventIndexerConfig` constructions in
`routes.rs` updated to include the new field.

## 8. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 648 / 648 ✓ (was 639)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 904 / 904 ✓ (was 895; +9 from this milestone)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Preserved: all 22 `broadcast_policy_data::tests::*` (incl. 9 LiveProvider
failure-metric tests); 9 `broadcast_observability::tests`; 31
`broadcast_policy::tests`; 16 `remote_signer::tests`; 7 config-startup-guard
tests; all 53+ `options::service::tests` (incl. 5 prior runtime helper
tests).

## 9. Remaining gaps

- **vault_observability / typed-config consistency** — the existing
  `vault_observability::build_config` (used by the `/metrics` rebate
  budget rendering) reads `PROTOCOL_FEE_VAULT_ADDRESS` directly from
  `std::env::var` at render time, whereas the runtime helper reads
  from the typed-config field loaded at startup. Both honour the same
  canonical env key; in practice they agree, but a future PR
  (`BACKEND-VAULT-OBSERVABILITY-USE-TYPED-CONFIG`) should unify the
  two so a single source of truth feeds both layers. The 3 new
  config-state gauges expose any inconsistency to operators.
- **Native latency histograms / JSON `/executor_health` summary /
  PagerDuty + Discord webhook routes** — unchanged from prior
  milestones (deferred follow-ons; operator-side or follow-on PRs).

## 10. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe creation.
- No webhook secrets committed.
- No real KMS provider credentials.
- No `EXECUTOR_PRIVATE_KEY` / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No high-cardinality labels (metric vocabulary unchanged + 3
  bool-gauges added).
- No guessed PFV mainnet address — no hardcoded production address
  anywhere in code or docs; operators set the env key against their
  own deployment.
- No sol/ source touched.
- No DB schema migration.
- No fallback path that allows mainnet local-key signing.
- Config-load malformed address → process refuses to start; no
  permissive default introduced.

## 11. Cross-references

- Predecessor milestone:
  `BACKEND_LIVE_PROVIDER_IN_MAIN_WIRING_RESULT.md` (this PR closes the
  documented "PFV address None for now" placeholder from §7).
- Earlier predecessors:
  `BACKEND_LIVE_PROVIDER_FAILURE_METRICS_RESULT.md`,
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`,
  `BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`.
- Cluster anchor:
  `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3` (launch
  invariant primary teeth).

## 12. Next milestone recommendation

**Primary backend-side:** `BACKEND-VAULT-OBSERVABILITY-USE-TYPED-CONFIG`
— unify the two PFV-env code paths so `vault_observability::build_config`
sources `PROTOCOL_FEE_VAULT_ADDRESS` from
`state.option_event_indexer_config.protocol_fee_vault_address` instead
of `std::env::var`. Eliminates any potential drift between dotenv
loader behaviour and `std::env::var` at runtime.

**Parallel backend-side:** `EXECUTOR-HEALTH-ENDPOINT-V2` — JSON
summary endpoint for non-Prometheus consumers; consider exposing
the 3 new `*_configured` booleans alongside other status fields.

**Parallel operator-side (unchanged):** Deploy V1 alerts spec §4
PromQL rules → PagerDuty / Discord webhooks against your secret store;
`MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5);
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`; `MAINNET-AUDIT-EXT-KICKOFF`;
`MAINNET-TREASURY-SAFE-CREATION-PACKET`;
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`;
`FRONTEND-V2G-W3-SSR-PROXY`.

**Auditor anchors strengthened:** Q-34 (Cluster 4 launch invariant) —
the chain-side backstop is now wired end-to-end: operators set
`PROTOCOL_FEE_VAULT_ADDRESS`, the runtime helper threads it into the
LiveProvider, the LiveProvider issues `PFV.rebateReserve(asset)` per
broadcast attempt, and the rejection or success directly feeds the
`/metrics` counters. Q-26 + Q-29 unaffected (signer surface unchanged).
