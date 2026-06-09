# BACKEND-LIVE-PROVIDER-IN-MAIN-WIRING — result

**Status:** SHIPPED 2026-06-09.
**Scope:** thread the production `Arc<BroadcastObservability>` handle into
the runtime LiveProvider construction so real broadcast attempts feed the
new failure counters into `/metrics`. **No mainnet tx. No live broadcast.
No `.env` edit.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/docs/BACKEND_LIVE_PROVIDER_IN_MAIN_WIRING_RESULT.md`
  — this close-out doc.

### Modified (1)

- `src/options/service.rs`:
  - `broadcast_option_execution_intent` (the public no-provider entry
    point) now routes through the **full-fidelity**
    `broadcast_option_execution_intent_with_provider_signer_and_data_provider`
    using a `LiveBroadcastPolicyDataProvider` constructed via the new
    helper. The signer + provider are still built from state config
    (unchanged behaviour) but the data provider is now the production
    live impl with the observability handle attached.
  - New public `build_runtime_policy_data_provider<P>(state, provider)`
    helper that:
    - Sources `cv_address` from
      `state.option_event_indexer_config.collateral_vault_address` (empty
      string → `None`).
    - Sources `fees_manager_v2_address` from
      `state.option_event_indexer_config.fees_manager_v2_address`.
    - PFV address: deferred → `None` (operator follow-on; documented in
      `BACKEND-LIVE-PROVIDER-PFV-CONFIG`).
    - Attaches `state.broadcast_observability.clone()` via
      `LiveBroadcastPolicyDataProvider::with_observability(...)`.
  - `MockBroadcastProvider` (test fixture) gained `EthCallProvider` +
    `EthBalanceProvider` impls that return failures by default — every
    runtime-helper-driven call records a `policy_data_failures_total`
    increment, useful for verifying the observability wiring is hooked
    end-to-end.
  - 3 new integration tests.

**Unchanged:** `broadcast_option_execution_intent_with_provider` (used by
direct test calls with `MockBroadcastProvider`) continues to delegate
to `_with_provider_and_signer` (Sepolia-permissive stub). Existing test
suite (~640 tests) untouched by the runtime wiring.

## 2. Runtime construction path

Before this PR:

```text
broadcast_option_execution_intent(state, intent_id)
  └→ broadcast_option_execution_intent_with_provider(state, intent_id, &HttpJsonRpcProvider)
        └→ broadcast_option_execution_intent_with_provider_and_signer(...)
              └→ StubBroadcastPolicyDataProvider::sepolia_permissive()  ← runtime never used LiveProvider
```

After this PR:

```text
broadcast_option_execution_intent(state, intent_id)
  ├→ HttpJsonRpcProvider::new(rpc_url)
  ├→ build_signer_for_state(state)
  ├→ build_runtime_policy_data_provider(state, provider)        ← NEW
  │     └→ LiveBroadcastPolicyDataProvider::new(provider, None_pfv, cv_addr, fm_v2_addr)
  │           .with_observability(state.broadcast_observability.clone())  ← NEW
  └→ broadcast_option_execution_intent_with_provider_signer_and_data_provider(
       state, intent_id, &provider, signer, &data_provider)
```

The legacy `_with_provider` variant is preserved verbatim — it still uses
the Sepolia-permissive stub, so the 11+ direct `MockBroadcastProvider`
test calls (`option_execution_broadcast_*`) continue to pass unchanged.

## 3. Observability handle wiring

```rust
pub fn build_runtime_policy_data_provider<P>(
    state: &AppState,
    provider: P,
) -> LiveBroadcastPolicyDataProvider<P>
where
    P: TransactionBroadcastProvider + EthCallProvider + EthBalanceProvider + Clone + 'static,
{
    let cv_address = ...;
    let fm_v2_address = state.option_event_indexer_config.fees_manager_v2_address.clone();
    let pfv_address: Option<AccountId> = None;
    LiveBroadcastPolicyDataProvider::new(provider, pfv_address, cv_address, fm_v2_address)
        .with_observability(state.broadcast_observability.clone())
}
```

- No global singleton. The Arc is cloned per-provider, sharing the
  in-process counter state with the `/metrics` renderer.
- `HttpJsonRpcProvider` implements all four required traits
  (`TransactionBroadcastProvider`, `EthCallProvider`, `EthBalanceProvider`,
  `Clone`) so the production binary's runtime path satisfies the bounds
  unmodified.
- Metric names + labels unchanged from the prior milestone (no new
  cardinality introduced).
- Fail-closed semantics preserved end-to-end: any read failure → `None`
  in `BroadcastPolicyInputs` → mainnet rejects via the chain-state gates
  from prior milestones; signer is never contacted.

## 4. Preserved fail-closed behavior

- **Mainnet read failure** → `BroadcastPolicyInputs` field is `None` →
  `run_should_broadcast_policy` rejects via the structured code path
  (e.g. `policy:ome-paused`, `policy:rebate-reserve`,
  `policy:rebate-budget`, `policy:policy-internal:...`). Signer is not
  contacted. Counter increments at failure site (now visible to
  `/metrics`).
- **FM_V2 RPC / decode failure** → `fee_split = None` →
  `econ_data_available = false` → §8 steps 4 / 5 / 7 skip → boundary
  mode; mainnet still fail-closed via chain-state gates.
- **Signer call ordering** unchanged: `gather_inputs` → `should_broadcast`
  → signer. Regression-tested by prior 5 data-provider tests + 6
  observability integration tests + the new
  `legacy_with_provider_path_does_not_invoke_live_provider`.

## 5. Tests added (3 new)

### Integration (3 in `options::service::tests`)

- `build_runtime_policy_data_provider_attaches_observability` — builds a
  `LiveProvider` via the helper with a mock that fails every
  `eth_call` + `eth_get_balance`; asserts the state's observability
  snapshot records the failures for `be_balance`, `ome_paused`,
  `ome_is_executor`.
- `build_runtime_policy_data_provider_records_fm_v2_rpc_failure` —
  same shape but with FM_V2 address threaded through
  `state.option_event_indexer_config.fees_manager_v2_address`; asserts
  `fm_v2_rpc_failures_total == 2` (maker + taker) +
  `policy_data_failures_total{fm_v2_quote_fees_rpc} == 2` and that
  `fm_v2_decode_failures_total == 0` (cross-mode isolation preserved).
- `legacy_with_provider_path_does_not_invoke_live_provider` — pins the
  contract that the existing `_with_provider` path continues to use the
  Sepolia-permissive stub; no FM_V2 RPC / decode failures or
  policy-data failures are observed because no live reads happen in
  that path. Protects backwards compatibility for ~11 existing direct
  test callers.

**`MockBroadcastProvider` enhancements:** added `EthCallProvider` +
`EthBalanceProvider` impls that return failures by default. Both used
exclusively by the new runtime-helper tests; existing
`MockBroadcastProvider`-using tests (which call `_with_provider_and_signer`
directly with the Sepolia stub) never invoke these methods, so their
behaviour is unchanged.

## 6. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 639 / 639 ✓ (was 636)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 895 / 895 ✓ (was 892; +3 from this milestone)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Preserved unchanged: all 22 `broadcast_policy_data::tests::*` (incl. 9
LiveProvider failure-metric tests from the prior milestone); 9
`broadcast_observability::tests`; 31 `broadcast_policy::tests`; 16
`remote_signer::tests`; 7 config-startup-guard tests; all 53+
`options::service::tests` (incl. 11+ direct `_with_provider`
`MockBroadcastProvider` tests).

## 7. Remaining gaps

- **PFV address sourcing** — `pfv_address` defaults to `None` in the
  runtime helper. PFV reads (`PFV.feeBalance(asset)` +
  `PFV.rebateReserve(asset)`) are silently skipped at the LiveProvider
  layer until an env key is wired. Follow-on track:
  `BACKEND-LIVE-PROVIDER-PFV-CONFIG` (add
  `PROTOCOL_FEE_VAULT_ADDRESS` env key + plumb through
  `OptionEventIndexerConfig`).
  - **Impact during the gap:** mainnet `policy:rebate-reserve` /
    `policy:rebate-budget` rejects still fire via the chain-state
    gates from prior milestones because `pfv_rebate_reserve_asset =
    None → 0 (mainnet fail-closed default)`. The Cluster 4 launch
    invariant remains enforced; the PFV read just yields zero values
    instead of live ones.
- **Native latency histograms** — operators can derive rates from
  `*_total` counters via PromQL `rate()`. Native histograms deferred.
- **JSON `/executor_health` summary** — still deferred to
  `EXECUTOR-HEALTH-ENDPOINT-V2` per the prior milestone.
- **PagerDuty / Discord route deployment** — operator-side per the V1
  alerts spec §4 mapping; no webhook secrets committed.

## 8. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe / Insurance Safe creation.
- No webhook secrets committed.
- No `EXECUTOR_PRIVATE_KEY` / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No high-cardinality labels introduced (metric vocabulary unchanged
  from prior milestone).
- No fallback path that allows mainnet local-key signing.
- Remote signer cannot be called before policy approval — unchanged
  call-site ordering verified by prior regression tests + new
  `legacy_with_provider_path_does_not_invoke_live_provider`.
- No sol/ source touched.
- No DB schema migration.
- Provider failures cannot silently allow mainnet broadcast — `fee_split
  = None` still fails-closed via chain-state gates + the no-PFV-read
  case defaults to `pfv_rebate_reserve_asset = 0` on mainnet which
  triggers `policy:rebate-reserve` reject on any rebate-positive intent.

## 9. Cross-references

- Predecessor milestone:
  `BACKEND_LIVE_PROVIDER_FAILURE_METRICS_RESULT.md` (this PR wires the
  observability handle that the prior milestone made optional).
- Earlier predecessors:
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`,
  `BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`,
  `WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md`,
  `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`.

## 10. Next milestone recommendation

**Primary backend-side:** `BACKEND-LIVE-PROVIDER-PFV-CONFIG` — add
`PROTOCOL_FEE_VAULT_ADDRESS` env key + plumb through
`OptionEventIndexerConfig::protocol_fee_vault_address` so the runtime
helper threads a real PFV address into the LiveProvider. Closes the
last documented "None for now" address in
`build_runtime_policy_data_provider`.

**Parallel backend-side:** `EXECUTOR-HEALTH-ENDPOINT-V2` — JSON
summary endpoint for non-Prometheus consumers.

**Parallel operator-side (unchanged):** Deploy V1 alerts spec §4
PromQL rules → PagerDuty / Discord webhooks against your secret
store; `MAINNET-KMS-VENDOR-SELECTION` (Q-CD-5);
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION`; `MAINNET-AUDIT-EXT-KICKOFF`;
`MAINNET-TREASURY-SAFE-CREATION-PACKET`; `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`;
`FRONTEND-V2G-W3-SSR-PROXY`.

**Auditor anchors strengthened:** Q-34 (Cluster 4 launch invariant) —
production broadcast attempts now feed `fm_v2_rpc_failures_total` +
`fm_v2_decode_failures_total` + `r5_drift_observed_total` directly,
giving operators a live signal that the chain-side primary teeth could
fail under infrastructure or ABI drift. Q-26 / Q-29 unaffected (signer
surface unchanged).
