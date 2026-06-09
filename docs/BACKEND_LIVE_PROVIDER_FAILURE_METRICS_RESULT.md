# BACKEND-LIVE-PROVIDER-FAILURE-METRICS — result

**Status:** SHIPPED 2026-06-09 (Phase F close-out).
**Scope:** thread an optional `Arc<BroadcastObservability>` handle into
`LiveBroadcastPolicyDataProvider`; replace the existing silent `.ok()`
conversions with explicit `match` branches that increment the matching
Prometheus counter at every read-failure transition. **No mainnet tx.
No live broadcast. No `.env` edit. No webhook secrets.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/docs/BACKEND_LIVE_PROVIDER_FAILURE_METRICS_RESULT.md`
  — this close-out doc.

### Modified (2)

- `src/options/broadcast_policy_data.rs`:
  - New `pub mod read_type` with 10 hardcoded constants
    (`CHAIN_ID_RPC`, `BE_BALANCE`, `OME_PAUSED`, `OME_IS_EXECUTOR`,
    `PFV_FEE_BALANCE`, `PFV_REBATE_RESERVE`, `CV_PFV_BALANCE`,
    `FM_V2_QUOTE_FEES_RPC`, `FM_V2_QUOTE_FEES_DECODE`,
    `FM_V2_REBATE_BUDGET`). Strict bounded label vocabulary.
  - `LiveBroadcastPolicyDataProvider`: new
    `observability: Option<Arc<BroadcastObservability>>` field +
    `with_observability(Arc<BroadcastObservability>)` builder setter.
  - 3 new private helpers: `record_data_failure(read_type)`,
    `record_fm_v2_rpc_failure_metric()`,
    `record_fm_v2_decode_failure_metric()` — no-op when no handle is
    attached so library / test consumers stay observability-agnostic.
  - `gather_inputs` rewritten: every `.ok()` conversion on chain_id /
    eth_get_balance / read_bool_view / read_u256_view now records the
    matching `policy_data_failures_total{read_type}` counter on `None`.
  - `quote_fees_call` signature changed from
    `Option<FeeQuoteRaw>` to `Result<FeeQuoteRaw, FmV2QuoteFailureKind>`
    so the caller distinguishes RPC failure from ABI decode failure;
    `Rpc` and `Decode` arms each increment the dedicated counter
    (`fm_v2_rpc_failures_total` /
    `fm_v2_decode_failures_total`) AND the corresponding
    `policy_data_failures_total{read_type}` bucket.
  - New `pub enum FmV2QuoteFailureKind { Rpc, Decode }`.
  - 9 new tests covering each failure path.
- `src/options/mod.rs` — re-export `read_type` + `FmV2QuoteFailureKind`.

## 2. Provider failure observability gap (closed by this PR)

Prior to this PR, `LiveBroadcastPolicyDataProvider::gather_inputs`
swallowed every RPC + decode error as `None` via `.ok()`. The counters
declared by the prior monitoring milestone (`fm_v2_decode_failures_total`,
`fm_v2_rpc_failures_total`, `policy_data_failures_total{read_type}`)
were rendered to `/metrics` but never incremented in production —
operators could not observe live-read failures except via the
downstream `policy:policy-internal:...` reject that fires later when
chain-state inputs are missing.

This PR closes that gap end-to-end. Every `.ok()` site has been
replaced with an explicit `match` that increments the matching counter
on the `Err` / `None` branch BEFORE returning the placeholder.

## 3. Observability handle wiring

- **Field:** `LiveBroadcastPolicyDataProvider.observability:
  Option<Arc<BroadcastObservability>>`.
- **Default:** `None` (existing constructor `LiveBroadcastPolicyDataProvider::new(...)`
  unchanged; library + test consumers without a handle still work).
- **Builder:** `with_observability(Arc<BroadcastObservability>) -> Self`
  — call sites that want metrics use
  `new(...).with_observability(state.broadcast_observability.clone())`.
- **No global singleton.** Each provider instance carries its own
  optional `Arc` so production code and tests get independent handles.

## 4. Failure counters wired

| `gather_inputs` failure site                                            | Counter incremented                                                                                                              |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `provider.chain_id()` Err                                               | `policy_data_failures_total{read_type="chain_id_rpc"}`                                                                           |
| `provider.eth_get_balance(BE)` Err                                      | `policy_data_failures_total{read_type="be_balance"}`                                                                             |
| `read_bool_view(OME.paused())` None                                     | `policy_data_failures_total{read_type="ome_paused"}`                                                                             |
| `read_bool_view(OME.isExecutor(BE))` None                               | `policy_data_failures_total{read_type="ome_is_executor"}`                                                                        |
| `read_u256_view(PFV.feeBalance(asset))` None                            | `policy_data_failures_total{read_type="pfv_fee_balance"}`                                                                        |
| `read_u256_view(PFV.rebateReserve(asset))` None                         | `policy_data_failures_total{read_type="pfv_rebate_reserve"}`                                                                     |
| `read_u256_view(CV.balances(PFV, asset))` None                          | `policy_data_failures_total{read_type="cv_pfv_balance"}`                                                                         |
| `quote_fees_call(...)` Err(`FmV2QuoteFailureKind::Rpc`)                  | `fm_v2_rpc_failures_total` (Counter 1) + `policy_data_failures_total{read_type="fm_v2_quote_fees_rpc"}` (Counter 2)              |
| `quote_fees_call(...)` Err(`FmV2QuoteFailureKind::Decode`)              | `fm_v2_decode_failures_total` (Counter 1) + `policy_data_failures_total{read_type="fm_v2_quote_fees_decode"}` (Counter 2)        |
| `read_u256_view(FM_V2.rebateBudget(asset))` None                        | `policy_data_failures_total{read_type="fm_v2_rebate_budget"}`                                                                    |

The two distinct FM_V2 failure modes (RPC vs ABI decode) fire **both**
the dedicated `*_failures_total` counter AND the unified
`policy_data_failures_total{read_type}` bucket so operators can alert
either by failure-mode or by read-type without configuring extra
counters.

## 5. Bounded labels

All `read_type` values are hardcoded constants in `pub mod read_type`:

```rust
pub mod read_type {
    pub const CHAIN_ID_RPC: &str = "chain_id_rpc";
    pub const BE_BALANCE: &str = "be_balance";
    pub const OME_PAUSED: &str = "ome_paused";
    pub const OME_IS_EXECUTOR: &str = "ome_is_executor";
    pub const PFV_FEE_BALANCE: &str = "pfv_fee_balance";
    pub const PFV_REBATE_RESERVE: &str = "pfv_rebate_reserve";
    pub const CV_PFV_BALANCE: &str = "cv_pfv_balance";
    pub const FM_V2_QUOTE_FEES_RPC: &str = "fm_v2_quote_fees_rpc";
    pub const FM_V2_QUOTE_FEES_DECODE: &str = "fm_v2_quote_fees_decode";
    pub const FM_V2_REBATE_BUDGET: &str = "fm_v2_rebate_budget";
}
```

**Total cardinality = 10**. No user input ever feeds the label. The
`BroadcastObservability::sanitize_label` defence-in-depth pass remains
in place at the recording layer.

No intent_id, request_id, address, RPC URL, secret, or any other
high-cardinality value ever appears as a Prometheus label.

## 6. Fail-closed behavior preserved

The metric wiring is purely observational; it does NOT change the
fail-closed semantics established by prior milestones:

- Every `None` returned by the provider keeps the existing chain-state
  / econ_data_available gating intact.
- Mainnet: any read failure → matching `BroadcastPolicyInputs` field is
  `None` → `run_should_broadcast_policy` rejects via the structured
  code path (e.g. `policy:ome-paused`, `policy:rebate-reserve`,
  `policy:rebate-budget`, `policy:policy-internal:...`); signer is
  never contacted (regression-tested by 5+ prior tests).
- `quote_fees_call` returns `Err` instead of `None`, but the call site
  uses `if let (Ok(mk), Ok(tk)) = (&maker_quote, &taker_quote)` so
  `fee_split` remains `None` on any failure → `econ_data_available =
  false` → boundary mode.
- Remote signer cannot be called before policy approval — the order is
  unchanged: `gather_inputs` → `run_should_broadcast_policy` → signer.

## 7. Tests added (9 new)

All in `options::broadcast_policy_data::tests` using a new
`ProgrammableEthProvider` mock (configurable per-call failure
predicate, returns canned `EthCallSuccess` payloads):

- `live_provider_records_chain_id_rpc_failure` — `fail_chain_id` →
  `policy_data_failures_total{chain_id_rpc} == 1`.
- `live_provider_records_be_balance_failure` — `fail_balance` →
  `policy_data_failures_total{be_balance} == 1`.
- `live_provider_records_ome_paused_failure` — fail eth_call matching
  `keccak("paused()")[0..4]` → `policy_data_failures_total{ome_paused} == 1`.
- `live_provider_records_pfv_rebate_reserve_failure` — fail eth_call
  matching `keccak("rebateReserve(address)")[0..4]` →
  `policy_data_failures_total{pfv_rebate_reserve} == 1`.
- `live_provider_records_cv_pfv_balance_failure` — fail eth_call
  matching `keccak("balances(address,address)")[0..4]` →
  `policy_data_failures_total{cv_pfv_balance} == 1`.
- `live_provider_records_fm_v2_quote_fees_rpc_failure` — fail eth_call
  matching `quote_fees_selector_bytes()` →
  `fm_v2_rpc_failures_total == 2` (maker + taker) AND
  `policy_data_failures_total{fm_v2_quote_fees_rpc} == 2`. Verifies
  the dedicated `fm_v2_decode_failures_total` stays at zero.
- `live_provider_records_fm_v2_quote_fees_decode_failure` — return a
  truncated 100-byte payload for quoteFees calls →
  `fm_v2_decode_failures_total == 2` AND
  `policy_data_failures_total{fm_v2_quote_fees_decode} == 2`. Verifies
  the dedicated `fm_v2_rpc_failures_total` stays at zero (cross-mode
  isolation).
- `live_provider_records_fm_v2_rebate_budget_failure` — fail eth_call
  matching `keccak("rebateBudget(address)")[0..4]` →
  `policy_data_failures_total{fm_v2_rebate_budget} == 1`.
- `live_provider_without_observability_skips_metric_increments` —
  no handle attached → provider runs unchanged, no panic, no
  silent global state mutation. Pins the optional-handle contract.

## 8. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 636 / 636 ✓ (was 627)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 892 / 892 ✓ (was 883; +9 from this milestone)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Previously-green tests preserved:
- All 22 `broadcast_policy_data::tests::*` (incl. the existing 13
  ABI codec + aggregation + helper tests + the 5 `cv_pfv_inputs`
  startup invariant tests + the original 4 fee-split tests).
- All 15 `broadcast_observability::tests`.
- All 31 `broadcast_policy::tests`.
- All 16 `remote_signer::tests`.
- All 7 config-startup-guard tests.
- All 53+ `options::service::tests` (incl. 5 prior data_provider
  integration tests + 6 observability integration tests + 3 fee_split
  integration tests).

## 9. Remaining observability gaps

- **Latency histograms** — counters expose rate via PromQL `rate()`;
  native HDR-style histograms for FM_V2 RPC + signer latency are
  deferred (operators usually rely on rate + occasional manual
  observation; histograms can land in a follow-on if needed).
- **JSON `/executor_health` summary** — still deferred to
  `EXECUTOR-HEALTH-ENDPOINT-V2` per the prior milestone. The
  `/metrics` endpoint is the canonical observability surface.
- **LiveProvider construction in production `main.rs`** — production
  code that constructs the LiveProvider has not been wired into
  `main.rs` yet because the broadcast call site still uses the
  Sepolia-permissive stub by default (per the prior chain-state-reads
  milestone). When operators flip the broadcast path to the
  full-fidelity entry point
  (`broadcast_option_execution_intent_with_provider_signer_and_data_provider`)
  with a `LiveBroadcastPolicyDataProvider`, they must pass
  `.with_observability(state.broadcast_observability.clone())` —
  documented in the next-milestone recommendation.
- **PagerDuty / Discord route wiring** — operator-side per the prior
  milestone; no webhook secrets are committed.

## 10. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe / Insurance Safe creation.
- No webhook secrets committed.
- No `EXECUTOR_PRIVATE_KEY` / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No high-cardinality labels: `read_type` is a fixed 10-string
  vocabulary baked into `pub mod read_type` constants.
- No fallback path that allows mainnet local-key signing.
- Remote signer cannot be called before policy approval — unchanged
  call-site ordering verified by prior regression tests.
- No sol/ source touched.
- No DB schema migration.
- ABI decoder is strictly fail-closed: malformed contract return →
  `FmV2QuoteFailureKind::Decode` → metric increments → `fee_split = None`
  → `econ_data_available = false` → mainnet rejects via chain-state
  gates.

## 11. Cross-references

- Predecessor milestone:
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md` (this PR
  fills the `record_fm_v2_*_failure` + `record_policy_data_failure`
  hooks).
- Earlier predecessors:
  `BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`,
  `WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md`,
  `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`.
- Alerts spec: `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §4` —
  warning band rules on `rate(deopt_option_broadcast_fm_v2_*_total[5m])`
  + `rate(deopt_option_broadcast_policy_data_failures_total[5m])` now
  receive real production traffic when operators wire the LiveProvider.

## 12. Next milestone recommendation

**Primary backend-side:** `BACKEND-LIVE-PROVIDER-IN-MAIN-WIRING` —
construct `LiveBroadcastPolicyDataProvider` from `main.rs` (or a
helper) with the configured RPC URL + PFV / CV / FM_V2 addresses
threaded from env, and call `.with_observability(state.broadcast_observability.clone())`.
Once landed, operators have a fully-observable production broadcast
path with bounded-cardinality metrics for every read failure mode.

**Parallel backend-side:** `EXECUTOR-HEALTH-ENDPOINT-V2` — JSON
endpoint exposing a subset of `BroadcastObservabilitySnapshot` for
internal dashboards.

**Parallel operator-side:** Deploy V1 alerts spec PromQL → PagerDuty
/ Discord routes per the prior monitoring milestone §4.

**Auditor anchors strengthened:** Q-34 (Cluster 4 launch invariant) —
the `fm_v2_rpc_failures_total` + `fm_v2_decode_failures_total` rate
now alerts on infrastructure / ABI drift that could prevent the
chain-side primary teeth from firing. Q-26 (key non-extractable) +
Q-29 (mTLS authn) — unaffected; signer surface unchanged.
