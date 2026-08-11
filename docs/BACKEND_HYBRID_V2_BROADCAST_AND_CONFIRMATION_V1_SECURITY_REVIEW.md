# BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1 — Security Review

Milestone status: **Broadcast + confirmation pipeline wired against mock RPC. NO REAL PUBLIC-CHAIN BROADCAST WAS PERFORMED.**

Final verdict: **`BROADCAST_CONFIRMATION_SECURITY_VALIDATED`** (see closing section).

Frozen safety invariants enforced by this milestone (grep-able tokens):
* `NO_REAL_PUBLIC_CHAIN_BROADCAST`
* `BASE_MAINNET_8453_IS_FORBIDDEN`
* `NO_AUTOMATIC_NONCE_REPLACEMENT`
* `NO_AUTOMATIC_FEE_BUMP_OR_RBF`
* `REORGED_TRANSACTION_IS_NEVER_LEFT_CONFIRMED`
* `WRITE_RPC_METHOD_ALLOWLIST_IS_ETH_SENDRAWTRANSACTION_ONLY`

## 1. Scope and threat model

This milestone extends the pre-broadcast execution pipeline with:
1. A narrow broadcast RPC trait (`ExecutionBroadcastRpcClient`) with the exact 8-method
   allowlist (`src/hybrid_v2/execution/broadcast_rpc.rs:204`);
2. A transactional outbox (`BroadcastOutbox`) that persists the immutable envelope hash
   BEFORE the network send and NEVER re-signs
   (`src/hybrid_v2/execution/broadcast_outbox.rs:166`);
3. A confirmation worker (`BroadcastConfirmationWorker`) that polls receipts,
   verifies canonicality, walks the depth threshold, and hands the row to the
   indexer correlation module (`src/hybrid_v2/execution/broadcast_worker.rs:128`);
4. Reorg recovery + restart safety modules (Package C);
5. 6 operator-gated admin routes (Package C);
6. Startup wiring into `AppState::with_hybrid_v2_broadcast(...)` (Package D,
   `src/hybrid_v2/startup.rs:222`).

Broadcast is exercised end-to-end against a deterministic in-process
`MockBroadcastRpc` harness (`tests/hybrid_v2_broadcast_mock_rpc.rs`).
No test binary and no runtime code path ever contacts a public chain.

Threat model:
* an operator/API caller with valid admin credentials trying to
  influence target / calldata / gas / nonce / chain_id via the
  request body;
* a **compromised broadcast RPC provider** returning `Accepted` with a
  divergent `provider_tx_hash`, an `already_known` reply with a wrong
  hash, or a mined receipt whose `tx_hash` disagrees with our
  persisted envelope hash;
* the same provider returning `Timeout` / `Transport` /
  `Unavailable` / `RateLimited` — introducing an ambiguous state
  where the tx MAY or MAY NOT have entered the mempool;
* a chain reorg after a receipt has been observed — the orphaned
  block leaves the tx dropped, mined in a replacement branch, or
  mined by a different tx that consumed the nonce;
* a stale simulation reused after chain-state change;
* an attacker with write access to the projection PostgreSQL trying
  to mutate the persisted `tx_hash` / `envelope_hash` /
  `submission_attempt_count` columns;
* concurrent operator requests racing on the same
  `canonical_execution_id`;
* a full backend process restart during any phase of the pipeline;
* a network position between backend and RPC provider (mTLS is
  optional — the frozen posture is to refuse plaintext to any
  non-loopback URL, enforced at `HttpExecutionBroadcastRpcClient::new`).

## 2. Broadcast RPC allowlist (Part T)

`src/hybrid_v2/execution/broadcast_rpc.rs:201` declares the exact
`BROADCAST_ALLOWED_METHODS` constant:
```
"eth_chainId", "eth_blockNumber", "eth_getBlockByNumber",
"eth_getBlockByHash", "eth_getTransactionByHash",
"eth_getTransactionReceipt", "eth_getTransactionCount",
"eth_sendRawTransaction"
```

`HttpExecutionBroadcastRpcClient::check_method(method)` (line 266)
funnels every JSON-RPC round trip through this list; non-allowlisted
methods return `MethodNotAllowed` BEFORE reaching the wire. The
allowlist has **exactly ONE write method** — `eth_sendRawTransaction`.

**Refused methods** (asserted at
`tests/hybrid_v2_broadcast_full_matrix_pg_integration.rs:2789 matrix_53_broadcast_rpc_allowlist_is_narrow`):
* `eth_sendTransaction`
* `personal_sendTransaction`
* `eth_sign`
* `eth_signTypedData`
* `eth_signTransaction`

Every mock-based test asserts `mock.write_method_calls()` contains
ONLY `"eth_sendRawTransaction"` via the shared `assert_only_send_raw`
helper (see full-matrix binary + properties + performance binaries).

## 3. Base mainnet refusal (frozen)

Base mainnet chain id `8453` is refused at **three independent
gates**:
1. env validation — `HybridV2ExecutionConfig::validate_startup`
   (`src/hybrid_v2/config.rs:1316`) refuses when the configured
   chain id equals `8453` OR when `allowed_broadcast_chain_ids`
   contains `8453`;
2. RPC constructor —
   `HttpExecutionBroadcastRpcClient::new(...expected_chain_id...)`
   (`src/hybrid_v2/execution/broadcast_rpc.rs:240`) returns
   `BroadcastRpcError::BaseMainnetForbidden` when
   `expected_chain_id == Some(8453)`;
3. wire helper — `wire_hybrid_v2_broadcast`
   (`src/hybrid_v2/startup.rs:222`) refuses at function entry AND
   again after loading the allowed-chain list.

Tests asserting each gate:
* env — `hybrid_v2_broadcast_foundation_pg_integration` +
  `matrix_04_validate_startup_refuses_missing_rpc_url` +
  `matrix_05_validate_startup_refuses_chain_not_allowed`;
* RPC — `matrix_02_wire_refuses_base_mainnet_at_rpc_construction`
  + `prop_6_base_mainnet_refused_at_every_seed`;
* wire — `startup::tests::wire_hybrid_v2_broadcast_refuses_base_mainnet`.

## 4. Persist-before-send safety

`src/hybrid_v2/execution/broadcast_outbox.rs:239-246` documents the
frozen invariant: the immutable `tx_hash` + `envelope_hash` are written
to `hybrid_v2_broadcast_state` **BEFORE** `rpc.send_raw_transaction(...)`
is called. The Postgres implementation of `set_broadcast_tx_hash`
(`src/hybrid_v2/persistence.rs:2589`) uses:
```
UPDATE hybrid_v2_broadcast_state
   SET tx_hash = $2, envelope_hash = $3, envelope_bytes_hash = $4,
       updated_at_ms = $5
 WHERE canonical_execution_id = $1
   AND (tx_hash IS NULL OR tx_hash = $2)
```
so a divergent overwrite is rejected as a persistence error and an
idempotent re-write of the same hash is a no-op.

**Recovery contract** (from `broadcast_outbox.rs:31-42`): after a
mid-call crash the row is `BROADCASTING` with a persisted
`envelope_hash`; `resume()` uses `transaction_by_hash(envelope_hash)` to
determine the on-chain fate and NEVER re-signs.

Tests: `matrix_44_restart_while_pending_row_intact`,
`matrix_45_restart_while_confirming_no_resend`,
`matrix_46_restart_while_reorged_row_intact`,
`prop_5_persisted_tx_hash_never_mutates_on_resubmit`.

## 5. No automatic remediation

Frozen posture: NO automatic nonce replacement, NO automatic fee bump,
NO automatic re-sign, NO automatic RBF.

* NonceTooLow / NonceTooHigh / ReplacementUnderpriced →
  `MANUAL_INTERVENTION_REQUIRED` with `NONCE_CONFLICT_*` failure class
  (`broadcast_outbox.rs:78-90` + `classify_and_persist` mapping).
* ProviderRejection → `MANUAL_INTERVENTION_REQUIRED` with
  `PROVIDER_REJECTED`.
* PROVIDER_HASH_MISMATCH → `MANUAL_INTERVENTION_REQUIRED` (critical).
* Timeout / Transport / Unavailable → `SUBMISSION_UNKNOWN`; a
  bounded resume-only investigation via `transaction_by_hash`.

Tests verifying the frozen posture:
* `matrix_25_nonce_too_low_manual_intervention` /
  `matrix_26_nonce_too_high` / `matrix_27_replacement_underpriced`;
* `matrix_43_no_fee_bump_on_reorg`;
* `prop_11_no_automatic_fee_bump_across_any_outcome`;
* `prop_12_no_automatic_nonce_replacement_across_any_outcome`;
* `matrix_56_no_fee_bump_in_outbox_source` (source-audit);
* `matrix_57_no_signer_call_in_outbox` (source-audit — outbox never
  invokes any signer function).

## 6. Provider hash-mismatch escalation

Both the outbox (post-send classification) and the confirmation
worker (post-receipt observation) escalate a `provider_tx_hash !=
envelope.envelope_hash` OR a `receipt.tx_hash != our_tx_hash`
mismatch to `MANUAL_INTERVENTION_REQUIRED`:
* `critical_hash_mismatch` — `broadcast_outbox.rs:553`;
* `escalate_receipt_hash_mismatch` — `broadcast_worker.rs:213-223`
  invoked by `tick_single` on any receipt whose `tx_hash` disagrees.

Test: `matrix_17_provider_hash_mismatch_manual_intervention` +
`prop_15_receipt_hash_mismatch_always_manual_intervention`.

## 7. Reorg + canonicality safety

`REORGED_TRANSACTION_IS_NEVER_LEFT_CONFIRMED`. The worker's
`verify_canonical_receipt` step (`broadcast_worker.rs:394-467`)
compares the receipt's `block_hash` with the canonical block header at
`receipt.block_number`. On mismatch the row transitions to
`Reorged`; no confirmation-depth count is applied. The reorg
recovery module (`src/hybrid_v2/execution/broadcast_reorg_recovery.rs`)
re-observes on subsequent ticks:
* re-mined at the same envelope hash → row can walk back to
  `Confirming`;
* dropped in the replacement branch → stays `Reorged` with
  `DISAPPEARED` failure class;
* different tx consumed the nonce → escalates to
  `MANUAL_INTERVENTION_REQUIRED`.

Tests: `matrix_33_receipt_block_mismatch_reorged`,
`matrix_40_mined_block_reorg`, `matrix_41_reorg_advances_when_receipt_returns`,
`matrix_42_reorg_drop_stays_reorged`,
`prop_20_reorged_receipt_never_transitions_to_confirmed`.

## 8. Confirmation depth + indexer correlation (final rule)

The worker requires **BOTH** conditions before Confirmed:
1. `receipt.block_number + confirmation_depth <= head_block_number`
2. the projection indexer has a matched execution row for the same
   canonical id AND the row is `Complete`.

`indexer_correlation.rs` (broadcast_indexer_correlation.rs) is the
sole path from `Confirming` → `Confirmed`; a bare depth-satisfied row
without correlation stays `Confirming`. When correlation is missing
after tolerance ticks, the row escalates to
`MANUAL_INTERVENTION_REQUIRED` (`CORRELATION_MISSING`).

Tests: `matrix_35_depth_below_threshold_stays_confirming`,
`matrix_36_depth_at_threshold_reaches_confirming`,
`matrix_37_indexer_behind_stays_confirming`,
`matrix_38_finalized_persisted`.

## 9. Signer boundary

The pre-broadcast execution orchestrator + the broadcast outbox are
**independent** subsystems attached to `AppState` via distinct
constructors (`with_hybrid_v2_execution_orchestrator` +
`with_hybrid_v2_broadcast`). The outbox NEVER calls any signer
function; only the pre-broadcast orchestrator does. Source-audit
tests:
* `matrix_55_signer_source_no_broadcast_calls` — signer modules
  contain zero `send_raw_transaction` invocations;
* `matrix_57_no_signer_call_in_outbox` — outbox contains zero
  `.sign(...)` / `.sign_execution(...)` invocations;
* `prop_18_signer_module_never_appears_in_broadcast_outbox`.

The signer microservice itself is out of scope for this milestone;
its security properties are documented in
`docs/BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1_SECURITY_REVIEW.md`.

## 10. Deny-unknown-fields input hardening

Every admin route body uses `serde(deny_unknown_fields)` so an
operator cannot inject raw tx bytes, calldata, nonce, gas params,
signer identity, or an RPC URL through the request:
* `BroadcastRequestBody` — empty body (`hybrid_v2_execution_admin.rs:927`)
* `EmptyAdminBody` — empty body (line 933)
* `ManualInterventionBody` — `action` + `detail` ONLY (line 938)

Test: `matrix_14_admin_broadcast_rejects_extra_body_field`.

## 11. Admin gate + Base mainnet refusal at handler entry

Every broadcast admin route enforces:
1. `ensure_admin(state, headers)` — admin gate is disabled by default
   in `AdminConfig`;
2. `resolve_deployment(state, deployment_id)` — the deployment must
   exist in the read registry;
3. `refuse_mainnet(entry.manifest.chain_id)` — chain_id 8453
   returns 403 `BASE_MAINNET_FORBIDDEN`;
4. `broadcast_config_or_disabled(state)` — refuses when
   `hybrid_v2_execution_config` is absent OR when
   `broadcast_enabled = false` OR when `allowed_broadcast_chain_ids`
   contains 8453.

Tests: `matrix_13_admin_broadcast_without_token_returns_403` +
`hybrid_v2_broadcast_admin_pg_integration` (Package C, 15 scenarios).

## 12. Read API isolation

The Hybrid V2 read router (`src/api/hybrid_v2_read/router.rs`) exposes
**zero** POST/PUT/DELETE/PATCH surfaces. The line-by-line
comment-stripped audit is
`matrix_52_public_read_router_has_no_write_surface`. When the
broadcast pipeline is completely absent, canonical read routes still
return 200 (`matrix_51_read_api_unaffected_by_broadcast_absence`).

## 13. Startup fail-closed

`wire_hybrid_v2_broadcast` (`src/hybrid_v2/startup.rs:222`) has the
three-outcome contract:
* `Ok(None)` — broadcast disabled by env → AppState carries the
  fail-closed marker; admin returns 503.
* `Ok(Some(_))` — full construction; AppState wires all four handles.
* `Err(reason)` — validation, RPC construction, projection store, or
  chain-id gate failed. Caller downgrades to `outbox = None` + logs a
  WARN; read-side backend keeps serving.

Test: `startup::tests` (5 unit tests, in-tree). Verdict from Package D
wiring commit `feat(subaccounts): wire hybrid v2 broadcast outbox into
app state`.

## 14. Concurrency + operation lock

Concurrent submit / recheck / resend calls on the same canonical id
funnel through the outbox's:
* `insert_broadcast_state` (idempotent on primary key);
* `set_broadcast_tx_hash` immutability trigger;
* `update_broadcast_phase(from, to, ...)` conditional update — a
  concurrent writer that lands first returns `false`, surfaced as
  `OutboxError::LockContention`.

Test: `matrix_18_duplicate_submit_is_idempotent`.

## 15. Restart safety

The pipeline reconstructs identically from persisted state across
process restarts:
* pending rows keep their persisted `tx_hash` (immutable trigger);
* confirming rows keep receipt fields;
* reorged rows keep their `reorg_count` + `canonicality_state`;
* a fresh AppState from the same PG pool observes identical rows.

Tests: `matrix_44` / `45` / `46` / `47` +
`hybrid_v2_broadcast_restart_pg_integration` (Package C, 15
scenarios).

## 16. Metrics / observability posture

`BroadcastObservability` counters live in
`src/options/broadcast_observability.rs`. The Prometheus renderer
(`src/monitoring.rs`) NEVER emits wallets, signatures, envelope
bytes, or provider URLs — only sanitized phase / failure_class
counters. Sanitized admin `broadcast_status` responses redact every
signature byte, raw envelope byte, and provider connection detail
(`SanitizedBroadcastRow` — `hybrid_v2_execution_admin.rs:947`).

## 17. Deferred / out of scope

* **Plan + signed hydrator** — the admin `broadcast_resend_same_bytes`
  route returns a 503 with the wired-broadcast state as an honest
  deferral: reconstructing the exact signed envelope from persisted
  execution row columns is scheduled for
  `BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1`. Until then the
  operator cannot resend from the wired production admin route — but
  the outbox `resend_same_bytes` API itself is complete + tested
  (`matrix_24_same_byte_resend_within_budget`).
* **Live testnet broadcast** — reserved for the next milestone. This
  milestone performed NO real public-chain broadcast; every test used
  the deterministic mock RPC.

## Final verdict

`BROADCAST_CONFIRMATION_SECURITY_VALIDATED`.
