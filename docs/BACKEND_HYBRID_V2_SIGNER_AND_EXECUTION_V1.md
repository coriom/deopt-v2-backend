# BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1 — Closure

Status: **CLOSED (pre-broadcast surface). Broadcast is disabled by construction.**

- Signer verdict: **`BACKEND_HYBRID_V2_SIGNER_INTERFACE_READY_EXTERNAL_SIGNER_REQUIRED`**
- Security verdict: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_SECURITY_VALIDATED`**
- Performance verdict: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_PERFORMANCE_BOUNDED`**
- CI verdict: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_CI_GATE_VALIDATED`**
- Documentation verdict: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_DOCUMENTATION_COMPLETE`**

Next stage: **`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`**.

## 1. Authority model

- On-chain: **permissionless**. `OptionMatchingEngineV2::executeMatch`
  accepts pre-signed EIP-712 envelopes from any msg.sender. The recovered
  owner on-chain is the OWNER, not the backend.
- Backend: **gas payer / batch relayer**. The backend chooses whether to
  spend gas to submit a validly-signed intent. It has no authority to
  create signatures the owner did not sign.
- This milestone: **pre-broadcast only**. Broadcast is not implemented;
  no `send_*` method exists in the execution module (compile-time firewall).

## 2. 18-step orchestrator flow

For a single execution intent, `ExecutionOrchestrator::prepare` performs
the following steps (each is one persisted phase transition, per the
state-machine matrix in `src/hybrid_v2/execution/state.rs`):

1. Base mainnet firewall (refuse chain_id == 8453 outright).
2. Derive `canonical_execution_id` from intent.
3. Idempotent seed row (ON CONFLICT DO NOTHING).
4. Acquire the deployment-scoped `Execution` lock (contention → error).
5. Re-read the row (in case another writer advanced it).
6. Transition `Discovered → Validating`.
7. Preflight: readiness / drift / order state / correlation. On reject:
   route to `Cancelled` (order cancelled) or `Failed(PREFLIGHT_REJECTED)`.
8. Transition `Validating → ReadyToSimulate`.
9. Reserve the executor nonce (RPC pending nonce + persisted candidates).
10. Persist `plan_hash + calldata_hash + target + selector + reserved_nonce`
    and transition `ReadyToSimulate → Simulating`.
11. Run `eth_call` bound to head block number + hash (bounded transport
    retries via `MAX_TRANSPORT_RETRIES = 3`).
12. On deterministic revert: transition `Simulating → SimulationFailed →
    Failed(SIMULATION_FAILED_DETERMINISTIC)`.
13. On success: transition `Simulating → SimulationSucceeded`.
14. Sample `fee_history` and compute gas via `GasFeePolicy::compute`.
    On reject: `Failed(GAS_POLICY_REJECTED)`.
15. Persist `gas_limit + max_fee + max_priority + signing_payload_hash`
    and transition `SimulationSucceeded → AwaitingSignature`.
16. Transition `AwaitingSignature → Signing`; run the independent
    `SignerPolicyFirewall::revalidate`. On reject: `Failed(FIREWALL_REJECTED)`.
17. Invoke the signer; verify signature locally via `verify_signed_tx`.
    On unavailability: `Failed(SIGNER_UNAVAILABLE)`. On verify failure:
    `Failed(SIGNATURE_VERIFICATION_FAILED)`.
18. Persist signature; transition `Signing → SignatureVerified →
    ReadyForBroadcast → BroadcastDisabled` (terminal).

Every phase transition is a conditional forward-only SQL UPDATE
(`WHERE phase = $from`); lost updates return `Ok(false)` and surface
as `StoreFailure("lost update from … to …")`.

## 3. State machine (14 phases + legal transitions)

See `src/hybrid_v2/execution/state.rs`. The 14 phases are:

`Discovered → Validating → ReadyToSimulate → Simulating →
{SimulationSucceeded | SimulationFailed} → AwaitingSignature →
Signing → SignatureVerified → ReadyForBroadcast → BroadcastDisabled`

Off-happy-path terminals: `Cancelled` (operator or preflight), `Stale`
(age-based), `Failed` (any other classified failure). All four terminals
have zero outgoing edges (unit tests enforce this).

## 4. Transaction-plan schema

The plan is the deterministic ABI-encoded input to
`OptionMatchingEngineV2::executeMatch`. Layout:

```rust
pub struct ExecutionPlan {
  canonical_execution_id: CanonicalExecutionId,
  chain_id: u64,
  deployment_id: i64,
  target: [u8; 20],           // engine address (from manifest allowlist)
  selector: [u8; 4],          // executeMatchCall::SELECTOR
  calldata: Vec<u8>,          // ABI-encoded (buyerEnv, buyerSig, buyerOrder,
                              // sellerEnv, sellerSig, sellerOrder, fillQty,
                              // buyerActiveSeries[], sellerActiveSeries[])
  calldata_hash: [u8; 32],    // keccak256(calldata)
  value_wei: U256,            // always 0 (executeMatch is not payable)
  expected_module_version: String,
  deadline_ms: Option<u64>,
  plan_hash: [u8; 32],        // deterministic hash over all above
}
```

## 5. Target / selector policy

Only `OptionMatchingEngineV2::executeMatch` is enrolled at this milestone.

- Enrollment: `TargetPolicy::from_manifest` reads the engine address
  from `ManifestParams::module_addresses.option_matching_engine`.
- Refusal: unknown targets (`UnknownTarget`), wrong selectors
  (`UnknownSelector`), chain mismatch (`ChainMismatch`), Base mainnet
  (`BaseMainnetForbidden`).
- Adding a new target/selector requires a docs update here + a
  corresponding entry in `target_policy.rs`.

## 6. Pre-execution validation

`PreflightChecker` runs the following against the projection state:

- Base mainnet firewall (always first).
- Row not already terminal.
- `ReadinessReport::is_ready`. If not:
  - `ReorgDetected` → `ActiveReorg`
  - `RebuildInProgress` → `ActiveRebuild`
  - `ReconciliationDrift` → `ReconciliationDrift(detail)`
  - otherwise → `DeploymentNotReady(reason)`
- `matched_executions` correlation: if the pair already settled on-chain,
  refuse `ExecutionAlreadySettledOnChain`.
- Order lifecycle: `OrderCancelled`, `OrderExpired`,
  `QuantityExceedsRemaining`. Missing rows produce a
  `TrustLevel::UnverifiedFromProjection` warning (non-blocking) —
  simulation is the authoritative last-word check.
- Owner recovery + subaccount finalization.

## 7. Simulation

- `eth_call` bound to `BlockTag::Number(head_block)` (snapshot).
- `simulation_block_hash` is persisted so downstream code can verify the
  block hash matches on subsequent reads.
- Revert path: decodes the 4-byte custom-error selector via
  `KNOWN_CUSTOM_ERROR_SELECTORS`; unknown reverts surface as raw bytes.
- Retries: transport-only (`Transport`, `RateLimited`, `ServerError`),
  bounded to a single retry per call (see `simulator.rs`), with a hard
  `MAX_TRANSPORT_RETRIES = 3` cap on the orchestrator's per-attempt
  loop.

## 8. Gas / fee policy

`GasFeePolicy::compute` enforces:

- `estimate == 0` → `EstimateZero`.
- `estimate * multiplier_bps` overflow → `EstimateOverflow`.
- Rounded `gas_limit > max_gas_limit` → `EstimateExceedsCeiling`.
- Newest base fee > `abnormal_estimate_reject_threshold × median` →
  `ProviderFeeAnomaly`.
- Computed `max_fee > max_fee_per_gas_wei` → `FeeCapExceeded` (defensive;
  cannot fire because of the `.min` cap).
- Computed `total = gas_limit * max_fee > max_total_native_cost_wei` →
  `TotalCostExceeded`.

Config validation runs at startup via
`GasFeePolicy::validate_config_startup`.

## 9. Nonce model

- `hybrid_v2_executor_nonces(chain_id, signer_identity, reserved_nonce)`
  carries a UNIQUE constraint. Two workers racing on the same nonce
  see one INSERT succeed and the other collide (`rows_affected = 0`);
  the loser advances.
- Candidates come from `max(on_chain_pending, max_persisted + 1)`.
- Persisted rows survive process restarts — the reserver seeds its
  starting candidate from the DB, so a restarted worker never re-issues
  a used slot.

## 10. Signer boundary

- Narrow `SigningRequest`: chain_id, nonce, target, value, calldata_hash,
  gas_limit, fees, tx_type, plan_hash, signing_payload_hash, calldata.
  No raw tx bytes.
- Narrow `SignedTx`: `(signature_r, signature_s, signature_v,
  recovered_signer, tx_type)`. No raw hex broadcast payload.
- The signer MUST recompute `keccak256(calldata) == calldata_hash`
  before signing (`TestEphemeralSigner` and any production signer).
- Verification is independent: the orchestrator calls
  `verify_signed_tx` after every signer response and refuses signatures
  that don't recover the expected signer address.

**Production signer status: `SIGNER_INTERFACE_READY_EXTERNAL_SIGNER_REQUIRED`.**

The default `SignerBackend::Production` returns
`ProductionSignerUnavailable`, which yields
`SignerError::SignerUnavailable`. A production deployment MUST attach
a `SignerBackend::RemoteKMS` (or equivalent) implementation. That
integration is a separate downstream milestone.

## 11. Restart / idempotency

- `derive_canonical_execution_id(deployment_id, chain_id,
  buyer_order_hash, seller_order_hash, fill_quantity_1e8)` is a SHA-256
  over a domain-tagged preimage. Two `prepare` calls for the same
  intent converge on the same row.
- SQL immutability triggers on `plan_hash` and `calldata_hash` refuse
  any UPDATE that would mutate an already-set value (migration 0049).
- `resume(canonical_execution_id)` re-enters the pipeline using the
  persisted row. Idempotent — a terminal row returns as-is. If the
  row already carries a signature, the resume path SKIPS the signer
  call and re-verifies the persisted bytes.

## 12. Operator controls

Five admin routes (`src/api/hybrid_v2_execution_admin.rs`), all behind
`ensure_admin`:

- `POST …/prepare` — currently returns `503 EXECUTION_ORCHESTRATOR_NOT_WIRED`.
- `GET …/executions/:canonical_execution_id` — sanitized row.
- `GET …/executions` — bounded listing.
- `POST …/cancel` — refused past `AWAITING_SIGNATURE`.
- `POST …/retry` — returns 409 with `RETRY_MUST_ISSUE_NEW_CANONICAL_ID`
  (terminal rows never resurrect; re-issue `prepare`).

## 13. Public boundary

There is **NO** public execution route. The read-only public router
(`src/api/hybrid_v2_read/router.rs`) carries an audit test that
enumerates every mounted verb and refuses any mutating method
(`POST/PUT/PATCH/DELETE`) or execution-adjacent shorthand.

## 14. Broadcast kill switch (compile-time + runtime + source-scan)

Three independent defenses:

1. Compile-time: the `ExecutionRpcClient` trait has no `send_*` method.
2. Runtime: `ALLOWED_METHODS` (in `rpc.rs`) is a curated allowlist;
   anything else is refused at the wire.
3. Source-scan: `tests/hybrid_v2_execution_zero_broadcast_scan.rs`
   walks every `.rs` file under `src/hybrid_v2/execution/` and asserts
   no forbidden token (`send_raw_transaction`, `eth_sendTransaction`,
   `personal_sendTransaction`, …) is present. Two intentional
   exceptions: `rpc.rs` (the allowlist-defense file) and `mod.rs`
   (module-level doc comment).

## 15. Security model

See `BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1_SECURITY_REVIEW.md`.

Return value on that doc's conclusion:
**`BACKEND_HYBRID_V2_SIGNER_EXECUTION_SECURITY_VALIDATED`**.

## 16. Test evidence

| Binary | Test count | Feature |
|---|---|---|
| `hybrid_v2_execution_foundation_pg_integration` | 10 | default |
| `hybrid_v2_execution_zero_broadcast_scan` | 2 | default |
| `hybrid_v2_execution_simulator_and_signer_integration` | 6 | `test-signer` |
| `hybrid_v2_execution_orchestrator_pg_integration` | 13 | `test-signer` |
| `hybrid_v2_execution_full_pg_matrix` (Part V) | 34 | `test-signer` |
| `hybrid_v2_execution_properties` (Part W) | 18 | `test-signer` |
| `hybrid_v2_execution_performance_bounds` (Part X) | 10 | `test-signer` |
| **total** | **93** | |

Every test that boots the mock RPC server asserts
`mock.prohibited_calls_seen()` is EMPTY on exit.

## 17. Limitations (documented, not concealed)

1. **Production signer is NOT integrated.** The default backend is
   `ProductionSignerUnavailable`. Wiring lands in a downstream
   milestone.
2. **Orchestrator is NOT wired into the live AppState.** The admin
   `prepare` route returns `503 EXECUTION_ORCHESTRATOR_NOT_WIRED`.
   Live wiring lands with the production signer milestone.
3. **Broadcast is disabled by construction.** The next milestone
   (`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`) introduces the
   broadcast surface and MUST re-verify simulation freshness before
   broadcast (TOCTOU class).
4. **Database tampering** is not fully mitigated by application code —
   full defense requires DB-level access control (Postgres role
   hardening, network isolation). Documented as out-of-scope.

## 18. Next stage

**`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`** — broadcast
surface + confirmation tracking + reconciliation of on-chain
execution outcome vs the pre-broadcast row. That milestone MUST:

- Add a new trait method distinct from `ExecutionRpcClient` (so the
  compile-time firewall remains on `ExecutionRpcClient`).
- Re-verify simulation freshness immediately before broadcast.
- Add a `Broadcasted` phase and its legal edges to the state machine.
- Extend the `TargetPolicy` and firewall to check the broadcast payload
  matches the persisted signed row bit-for-bit.
- Extend this closure doc's status to CLOSED (with-broadcast) once
  landed.

---

Return value: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_DOCUMENTATION_COMPLETE`**.
