# BACKEND-VAULT-OBSERVABILITY-USE-TYPED-CONFIG — result

**Status:** SHIPPED 2026-06-10.
**Scope:** unify the `PROTOCOL_FEE_VAULT_ADDRESS` source-of-truth so
`fees::vault_observability::build_config` consumes the typed
`OptionEventIndexerConfig::protocol_fee_vault_address` instead of
reading `std::env::var` directly at metric-render time. Removes the
last remaining direct env-var read for the PFV address in the backend
runtime path. **No mainnet tx. No live broadcast. No `.env` edit.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/docs/BACKEND_VAULT_OBSERVABILITY_USE_TYPED_CONFIG_RESULT.md`
  — this close-out doc.

### Modified (3)

- `src/fees/vault_observability.rs`:
  - `build_config` signature gained a `protocol_fee_vault_address: Option<AccountId>`
    parameter (positioned after `rpc_url`, before `collateral_vault_address`).
  - Body removed the `std::env::var("PROTOCOL_FEE_VAULT_ADDRESS")` read;
    `vault_address` now derives from the typed parameter via the same
    `sanitize_address` helper (preserves case normalisation + zero-address rejection).
  - `PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS` env read retained — that
    knob is an observability-scan-only asset list, NOT a chain-side
    launch-invariant input, and remains decoupled from the broadcast
    policy chain reads.
  - 1 existing test (`build_config_pulls_assets_from_env_then_falls_back`)
    updated to call the new 5-arg signature; removed its
    `std::env::remove_var("PROTOCOL_FEE_VAULT_ADDRESS")` setup line
    (no longer needed).
  - 3 new tests verifying the typed-config contract.
- `src/monitoring.rs::append_protocol_fee_vault_metrics` — call site
  threads `state.option_event_indexer_config.protocol_fee_vault_address.clone()`
  into `build_config`.
- `src/api/routes.rs::vault_observability_config` — same change for
  the admin-endpoints code path.

## 2. Old PFV config path (before this PR)

```text
operator: PROTOCOL_FEE_VAULT_ADDRESS env var
  │
  ├──► env.rs::AppConfig::from_lookup (parses + validates at startup)
  │        └──► OptionEventIndexerConfig.protocol_fee_vault_address
  │                  └──► build_runtime_policy_data_provider
  │                            └──► LiveBroadcastPolicyDataProvider
  │                                  (broadcast policy chain reads — fail-closed)
  │
  └──► vault_observability::build_config::std::env::var
           (separate code path, no typed validation, at metric render time)
              └──► VaultObservabilityConfig
                    (admin endpoints + /metrics PFV gauges)
```

Two independent readers of the same canonical env key. Both honoured
`PROTOCOL_FEE_VAULT_ADDRESS` but the second skipped the canonical env
loader's `parse_evm_address` validation and used a parallel
`sanitize_address` helper. Risk: dotenv-loaded values not visible to
direct `std::env::var` calls at runtime → drift between the two views.

## 3. New typed config path (after this PR)

```text
operator: PROTOCOL_FEE_VAULT_ADDRESS env var
  │
  └──► env.rs::AppConfig::from_lookup (single source — parses + validates at startup)
       └──► OptionEventIndexerConfig.protocol_fee_vault_address
            ├──► build_runtime_policy_data_provider
            │       └──► LiveBroadcastPolicyDataProvider
            │             (broadcast policy chain reads — fail-closed)
            │
            └──► vault_observability::build_config (typed param)
                 └──► VaultObservabilityConfig
                       (admin endpoints + /metrics PFV gauges)
```

Single source of truth: the canonical env loader at startup. Both
observability layers + the broadcast policy receive the same typed
address through different consumers of `OptionEventIndexerConfig`.

## 4. Vault observability wiring

`fees::vault_observability::build_config` signature:

```rust
pub fn build_config(
    rpc_url: Option<String>,
    protocol_fee_vault_address: Option<AccountId>,   // NEW typed parameter
    collateral_vault_address: Option<AccountId>,
    fees_manager_v2_address: Option<AccountId>,
    fallback_assets: Vec<String>,
) -> VaultObservabilityConfig
```

Body:
```rust
let vault_address = protocol_fee_vault_address
    .as_ref()
    .map(|addr| addr.0.as_str())
    .and_then(sanitize_address);
```

`sanitize_address` continues to:
- Lower-case the supplied address.
- Reject `0x0000...0000`.
- Reject malformed hex / wrong length.

The canonical env loader already rejects malformed input at config
load via `parse_evm_address`, so the typed value arriving here is
well-formed. The `sanitize_address` pass at this layer is a defence-in-depth
re-check (lower-case normalisation + zero-address rejection) preserved
from the prior implementation.

## 5. Metrics consistency

| Metric                                                       | Source                                                          | Truth column                                |
| ------------------------------------------------------------ | --------------------------------------------------------------- | ------------------------------------------- |
| `deopt_protocol_fee_vault_configured`                        | `vault_observability::build_config` → `cfg.is_configured()`     | Now reads typed config (was: `std::env::var`) |
| `deopt_option_broadcast_protocol_fee_vault_configured`       | `monitoring.rs::append_broadcast_observability_metrics`         | Reads typed config directly                  |
| `deopt_option_broadcast_fees_manager_v2_configured`          | same                                                            | Reads typed config directly                  |
| `deopt_option_broadcast_collateral_vault_configured`         | same                                                            | Reads typed config directly                  |

**Both `*_configured` PFV gauges now agree by construction** — they
both derive from `state.option_event_indexer_config.protocol_fee_vault_address`.
Prior to this PR, drift was possible (the vault_observability layer
read `std::env::var` while the broadcast layer read typed config).

Metric names unchanged. No new high-cardinality labels. No raw
addresses or secrets emitted.

Per-asset vault gauges (`deopt_protocol_fee_vault_fee_balance_native`,
`_rebate_reserve_native`, etc.) — emitted only when `is_configured() == true`
— continue to fire identically; the asset-list env-var fallback path
remains unchanged.

## 6. Fail-closed behaviour preserved

- **Malformed env value** at startup → `BackendError::Config("invalid PROTOCOL_FEE_VAULT_ADDRESS: …")`
  via the canonical env loader (unchanged from prior milestone).
- **Empty / unset env value** → `protocol_fee_vault_address = None` →
  both observability layers report `configured = false`; per-asset
  vault gauges suppressed. Mainnet broadcast policy `policy:rebate-reserve`
  reject still fires via the call-site default-0 path (unchanged).
- **Zero address (0x0000...0000)** → `sanitize_address` defence-in-depth
  reject → `cfg.is_configured() == false`. Unchanged from prior behaviour.
- **No silent permissive default** introduced.
- **Remote signer cannot be called before policy approval** — unchanged.

## 7. Tests added (3 new + 1 updated)

### Vault observability (`fees::vault_observability::tests`)

- `build_config_receives_typed_pfv_address_from_caller` — typed PFV
  passed in → `cfg.is_configured() == true`; `vault_address` carries
  the lowercased address.
- `build_config_typed_pfv_address_is_normalised_to_lowercase` —
  mixed-case input is normalised via `sanitize_address`. Pins the
  case-normalisation contract preserved across the signature change.
- `build_config_ignores_legacy_protocol_fee_vault_address_env_var` —
  even if `PROTOCOL_FEE_VAULT_ADDRESS` is set in the process env at
  runtime, `build_config(typed=None, ...)` yields
  `vault_address = None`. **Pins the single-source-of-truth contract**:
  the env-var-direct path is gone.

### Updated

- `build_config_pulls_assets_from_env_then_falls_back` — updated to
  pass `None` for the new typed parameter; removed the now-unnecessary
  `std::env::remove_var("PROTOCOL_FEE_VAULT_ADDRESS")` setup line.

## 8. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 651 / 651 ✓ (was 648)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 907 / 907 ✓ (was 904; +3 from this milestone)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Preserved unchanged:
- 7 prior env-loader PFV tests (`config::env::tests::protocol_fee_vault_*`).
- 5 prior runtime helper PFV tests in `options::service::tests`.
- 9 prior LiveProvider failure-metric tests.
- 12 vault_observability tests (incl. 3 new + 1 updated).
- 9 `broadcast_observability::tests`.
- 31 `broadcast_policy::tests`.
- 16 `remote_signer::tests`.
- 7 config-startup-guard tests.
- All 53+ `options::service::tests`.

## 9. Remaining gaps

- **`PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS` is still read via
  `std::env::var`** inside `build_config`. This is an observability-scan-only
  asset list (not a chain-side launch-invariant input), and it has a
  fallback to the `fallback_assets` vec when unset. Promoting it to
  typed config would be a separate follow-on (e.g. add
  `protocol_fee_vault_reconciliation_assets: Option<Vec<AccountId>>`
  to OptionEventIndexerConfig). Not blocking — the launch-critical
  PFV address is unified; the asset list is operator-side observability
  tuning.
- **JSON `/executor_health` summary endpoint** — still deferred to
  `EXECUTOR-HEALTH-ENDPOINT-V2`.
- **Native latency histograms** — operators can derive rates from
  `*_total` counters via PromQL `rate()`.
- **PagerDuty / Discord route deployment** — operator-side per the
  V1 alerts spec §4 mapping; no webhook secrets committed.

## 10. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe / Insurance Safe creation.
- No webhook secrets committed.
- No `EXECUTOR_PRIVATE_KEY` / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No high-cardinality labels (metric vocabulary unchanged).
- No fallback path that allows mainnet local-key signing.
- Remote signer cannot be called before policy approval — unchanged
  call-site ordering.
- No sol/ source touched.
- No DB schema migration.
- Config remains fail-closed on malformed PFV address: rejected at
  startup by the canonical env loader (the only code path that parses
  the env var).
- Metric names + label vocabularies unchanged. Backwards-compatible
  with operators' existing dashboards + alert rules.

## 11. Cross-references

- Predecessor milestone:
  `BACKEND_LIVE_PROVIDER_PFV_CONFIG_RESULT.md` (this PR closes the
  documented "vault_observability / typed-config consistency" gap from
  §9 of that doc).
- Earlier predecessors:
  `BACKEND_LIVE_PROVIDER_IN_MAIN_WIRING_RESULT.md`,
  `BACKEND_LIVE_PROVIDER_FAILURE_METRICS_RESULT.md`,
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`.
- Cluster anchor:
  `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3` (launch
  invariant primary teeth — chain-side backstop now unified across
  both observability + broadcast policy paths).

## 12. Next milestone recommendation

**Primary backend-side:** `EXECUTOR-HEALTH-ENDPOINT-V2` — JSON
summary endpoint exposing a subset of `BroadcastObservabilitySnapshot`
+ the 4 `*_configured` booleans for non-Prometheus consumers
(internal dashboards / admin UI).

**Parallel backend-side (optional consistency):** promote
`PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS` to typed config so the
last `std::env::var` direct read inside `build_config` goes away.
Small, non-launch-critical.

**Parallel operator-side (unchanged):** Deploy V1 alerts spec §4
PromQL rules → PagerDuty / Discord webhooks against your secret store;
`MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5);
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`; `MAINNET-AUDIT-EXT-KICKOFF`;
`MAINNET-TREASURY-SAFE-CREATION-PACKET`;
`MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`;
`FRONTEND-V2G-W3-SSR-PROXY`.

**Auditor anchors strengthened:** Q-34 (Cluster 4 launch invariant)
— operator-side observability + per-broadcast policy enforcement now
draw from a single typed-config source. Any drift between the
`/metrics` PFV gauges and the broadcast-side reject behavior is
eliminated by construction. Q-26 + Q-29 unaffected (signer surface
unchanged).
