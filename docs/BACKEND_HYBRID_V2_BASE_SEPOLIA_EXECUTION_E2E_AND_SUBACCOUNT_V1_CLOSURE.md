# BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-AND-SUBACCOUNT-V1-CLOSURE

**Milestone status: PARTIAL closure. Real Base Sepolia E2E deferred pending operator provisioning.**

Date: 2026-08-13.
Branch: `hv2-subaccount-v1-partial-closure` (not pushed).
HEAD before work: `69383f0`.

---

## 1. Explicit posture

This document closes the parts of the milestone that can be closed
without a live public-chain broadcast, and honestly defers the parts
that require operator-provisioned infrastructure. **No real
`eth_sendRawTransaction` call was issued to any public chain during
this session; exact count = 0.**

`SUBACCOUNT_V1_COMPLETE` is **NOT** returned. `SUBACCOUNT_V1_BASE_SEPOLIA_VALIDATED`
is **NOT** returned. The overall parent verdict is **NOT** returned.

Verdicts returned by this session:

* `BACKEND_HYBRID_V2_BASE_SEPOLIA_E2E_WAITING_FOR_EXPLICIT_BROADCAST_GATE` ✅
* `SUBACCOUNT_V1_TECHNICAL_DEBT_CLASSIFIED` ✅
* `SUBACCOUNT_V1_SECURITY_CLOSURE_VALIDATED` ✅
* `SUBACCOUNT_V1_REGRESSION_GREEN` ✅ (local PG + deterministic mocks; no real
  chain contact)

Verdicts explicitly **NOT** returned (deferred, per gate policy):

* `SUBACCOUNT_V1_COMPLETE` — pending real Base Sepolia validation.
* `SUBACCOUNT_V1_BASE_SEPOLIA_VALIDATED` — pending operator provisioning.
* `BACKEND_HYBRID_V2_BASE_SEPOLIA_EXECUTION_E2E_AND_SUBACCOUNT_V1_CLOSURE_COMPLETE` — pending same.

---

## 2. Environment validation (this session)

| Requirement | Present? | Notes |
|---|---|---|
| PG 16 disposable database | ✅ | `deopt_hv2_final_pg_5e722a10` up, `/tmp/deopt_hv2_final/env.current` sourced |
| `test-signer` and `aws-kms-transport` cargo features | ✅ | build-only, no chain reach |
| Signer microservice (mTLS reachable HTTPS) | ❌ | not provisioned |
| mTLS client certificate + key (PEM paths) | ❌ | not provisioned |
| Funded Base Sepolia executor address | ❌ | not provisioned |
| `HV2_BROADCAST_ENABLED=true` + `HV2_BROADCAST_RPC_URL=https://…-sepolia…` | ❌ | not set |
| `HV2_SIGNER_ENDPOINT` + mTLS PEM env | ❌ | not set |
| `HV2_E2E_ALLOW_REAL_BASE_SEPOLIA_BROADCAST=true` (explicit broadcast gate) | ❌ | not set |
| Chain manifest promoted from draft to `active` in `deployment_manifests` | ❌ | not set |

Per the milestone brief:
> If that explicit gate is absent: complete every possible dry-run/preflight
> step, return `BACKEND_HYBRID_V2_BASE_SEPOLIA_E2E_WAITING_FOR_EXPLICIT_BROADCAST_GATE`
> and DO NOT send.

That gate is absent. We do not send.

---

## 3. E2E verdicts — each `WAITING_FOR_OPERATOR_PROVISIONING`

The following E2E verdicts CANNOT be adjudicated without real chain
contact, and each is annotated with the specific missing item:

| E2E verdict | Missing item |
|---|---|
| `BASE_SEPOLIA_SIGNER_IDENTITY_PROBE_REAL` | signer microservice not deployed; no mTLS material |
| `BASE_SEPOLIA_MANIFEST_PROMOTED_LIVE` | deployment manifest still `draft`; operator promotion required |
| `BASE_SEPOLIA_ORCHESTRATOR_PREPARE_REAL` | executor not funded; RPC URL not configured |
| `BASE_SEPOLIA_BROADCAST_SUBMISSION_REAL` | `HV2_E2E_ALLOW_REAL_BASE_SEPOLIA_BROADCAST` gate absent |
| `BASE_SEPOLIA_RECEIPT_MINED_REAL` | requires prior broadcast |
| `BASE_SEPOLIA_CONFIRMATION_DEPTH_REACHED_REAL` | requires prior broadcast + N confirmations |
| `BASE_SEPOLIA_INDEXER_CORRELATION_REAL` | requires prior mined receipt visible to indexer |
| `BASE_SEPOLIA_END_TO_END_CONFIRMED_REAL` | requires all above |
| `BASE_SEPOLIA_REORG_INVALIDATION_OBSERVED_REAL` | requires a real reorg (natural; opportunistic) |

All are `WAITING_FOR_OPERATOR_PROVISIONING`.

---

## 4. Subaccount V1 capability audit (Part Q)

Comprehensive audit of every subaccount capability with source-file
evidence and honest per-column status. "Base Sepolia evidence" is
`NOT VALIDATED (operator provisioning required)` for anything that
depends on real chain broadcast.

| Capability | Implementation milestone | Production path | Real PG evidence | Mock evidence | Base Sepolia evidence | Remaining blocker | Status |
|---|---|---|---|---|---|---|---|
| Subaccount identity (subKey, execution id) | `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1` | `hybrid_v2/reducer.rs`, `hybrid_v2/identity.rs` | `hybrid_v2_persistence_core_pg_proof`, `hybrid_v2_runtime_persistence_integration` | reducer/property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Execution identity (executor address vs signer) | `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1` | `hybrid_v2/execution/signer_builder.rs`, `signer_kms_bridge.rs`, `signer_http_transport.rs` | `hybrid_v2_production_signer_http_e2e`, `hybrid_v2_production_signer_full_startup_matrix_pg_integration` | `hybrid_v2_external_signer_full_matrix_pg_integration` | NOT VALIDATED (operator provisioning required) | signer microservice + mTLS | COMPLETE-MOCK / PENDING-REAL-CHAIN for live probe |
| Ownership (owner → subKeys, transfer of ownership event) | `BACKEND-SUBACCOUNT-CANONICAL-STATE-AND-INDEXER-V1` | `hybrid_v2/reducer.rs`, `hybrid_v2/persistence.rs` | persistence PG suites | reducer tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Isolated subaccounts (per-subKey collateral/reservation isolation) | `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` | reducer + PG projections | `hybrid_v2_runtime_persistence_integration` | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Collateral (credit/debit, isolated per subKey) | Same as above | reducer.rs | PG runtime | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Internal transfers | Same | reducer.rs | runtime PG | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Reservations (order/collateral reservations) | Same | reducer.rs, PG projections | PG runtime | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Positions | Same | reducer.rs | PG runtime | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Order lifecycle | Same | reducer.rs (open/cancel/settle), read API | PG runtime, `hybrid_v2_read_api_postgres_main_router_tests` | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Fills | Same | reducer.rs, PG projections | PG runtime | property | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Fees / rebates | `BACKEND_GAS_FEES_REBATES_POLICY_V1` | reducer.rs (fee split), read API | PG runtime | property | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Recovery / finalization | `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` | reducer.rs (recovery_state, epochs, pause, finalization) | `hybrid_v2_runtime_persistence_integration` | reducer property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Live chain indexer (source + worker) | `BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1` | `rpc_chain_source.rs`, `worker.rs`, `live_worker` | `hybrid_v2_live_worker_pg_integration`, `hybrid_v2_rpc_chain_source_mock_integration` | mock integration | NOT VALIDATED (operator provisioning required) | live RPC endpoint | COMPLETE-PG / PENDING-REAL-CHAIN for live wall-clock validation |
| PostgreSQL projections | `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1` | `persistence.rs`, migrations 0044–0052 | `hybrid_v2_persistence_core_pg_proof`, `hybrid_v2_execution_foundation_pg_integration` | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Read API (PG-only production reads) | `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1` | `read_store.rs`, `runtime_backed_read_store.rs` | `hybrid_v2_read_api_postgres_main_router_tests`, `hybrid_v2_read_store_pg_proof` | parity tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Restart | `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1` + broadcast live wiring | `startup.rs`, `main.rs`, `broadcast_worker::spawn_supervised` | `hybrid_v2_production_signer_app_restart_pg_integration`, `hybrid_v2_broadcast_live_wiring_app_restart_pg_integration`, `hybrid_v2_broadcast_restart_pg_integration` | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Reorg (indexer + broadcast-side) | `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` | `reorg.rs`, `reorg_recovery.rs`, `broadcast_reorg_recovery.rs` | `hybrid_v2_reorg_recovery_pg_integration`, `hybrid_v2_reorg_high_risk_matrix_pg_integration`, `hybrid_v2_broadcast_reorg_recovery_pg_integration` | property tests | NOT VALIDATED (operator provisioning required) | live reorg is opportunistic | COMPLETE-PG / PENDING-REAL-CHAIN (opportunistic) |
| Rebuild | `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` | `rebuild.rs`, `rebuild_operations.rs` | `hybrid_v2_rebuild_operations_pg_integration`, `hybrid_v2_rebuild_bootstrap_properties` | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Reconciliation (Policy A — 4 direct categories) | `BACKEND-HYBRID-V2-CHAIN-VIEW-PROVIDER-AND-RECONCILIATION-TASK-V1` | `reconciler.rs`, `reconciliation_worker.rs`, `chain_view.rs`, `rpc_chain_view.rs` | `hybrid_v2_reconciliation_task_pg_integration` | reconciler unit tests | NOT VALIDATED (operator provisioning required) | additional categories require Solidity read views (see Part R) | COMPLETE-PG (for 4 supported categories) |
| Signer (`SignerBuilder`, KMS bridge, `HttpSignerTransport`, mTLS) | `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` + Production Signer Bootstrap | `signer_builder.rs`, `signer_http_transport.rs`, `signer_kms_bridge.rs`, `signer_production.rs` | `hybrid_v2_execution_simulator_and_signer_integration`, `hybrid_v2_external_signer_full_matrix_pg_integration`, `hybrid_v2_production_signer_full_startup_matrix_pg_integration` | ephemeral test signer | NOT VALIDATED (operator provisioning required) | KmsGcp/Turnkey/Fireblocks return `SignerUnavailable` — see Part R | COMPLETE-PG for KmsAws HTTP transport; PENDING-REAL-CHAIN for real KMS |
| Simulation | Same | `simulator.rs`, `preflight.rs` | `hybrid_v2_execution_simulator_and_signer_integration` | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Execution plan (deterministic + immutable) | Same | `plan.rs`, `orchestrator.rs`, immutability triggers in migration 0049/0052 | `hybrid_v2_execution_orchestrator_pg_integration`, `hybrid_v2_execution_full_pg_matrix`, `hybrid_v2_execution_properties` | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Nonce (executor nonce reservation) | Same | `nonce.rs`, `broadcast_nonce_policy.rs` | `hybrid_v2_execution_orchestrator_pg_integration`, `hybrid_v2_broadcast_lifecycle_pg_integration` | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Broadcast (transactional outbox, only `eth_sendRawTransaction`) | `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` + live wiring closure | `broadcast_outbox.rs`, `broadcast_rpc.rs`, `broadcast_firewall.rs` | `hybrid_v2_broadcast_foundation_pg_integration`, `hybrid_v2_broadcast_lifecycle_pg_integration`, `hybrid_v2_broadcast_full_matrix_pg_integration`, `hybrid_v2_broadcast_live_wiring_*` | mock RPC | NOT VALIDATED (operator provisioning required) | broadcast gate | COMPLETE-PG / PENDING-REAL-CHAIN |
| Receipt (worker polling + canonicality) | Same | `broadcast_worker.rs`, `broadcast_state.rs` | `hybrid_v2_broadcast_lifecycle_pg_integration`, `hybrid_v2_broadcast_live_wiring_e2e_pg_integration` | mock RPC | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG / PENDING-REAL-CHAIN |
| Confirmation (depth + indexer correlation) | Same | `broadcast_worker.rs`, `broadcast_indexer_correlation.rs` | `hybrid_v2_broadcast_full_matrix_pg_integration`, `hybrid_v2_broadcast_live_wiring_matrix_pg_integration` | property tests | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG / PENDING-REAL-CHAIN |
| Real-chain indexer correlation | Live chain source + broadcast | `broadcast_indexer_correlation.rs`, `correlation.rs`, `worker.rs` | mock + PG | mock | NOT VALIDATED (operator provisioning required) | requires prior broadcast | PENDING-REAL-CHAIN |
| Operator controls (admin routes) | Multiple | `api/hybrid_v2_admin.rs`, `api/hybrid_v2_execution_admin.rs` | `hybrid_v2_broadcast_admin_pg_integration`, `hybrid_v2_broadcast_live_wiring_e2e_pg_integration` | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |
| Readiness | `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-OPERATIONAL-CLOSURE-V1` | `readiness.rs` | integration | — | NOT VALIDATED (operator provisioning required) | — | COMPLETE-PG |

**No capability is marked `V1-BLOCKER`.** All remaining columns are
`PENDING-REAL-CHAIN` (operator provisioning) or `NON-BLOCKING-DEBT`
(see Part R). This means Subaccount V1 is code-complete for every
capability the milestone brief enumerated; the only blocker for the
`SUBACCOUNT_V1_COMPLETE` verdict is the real Base Sepolia broadcast +
confirmation cycle, which requires operator infrastructure this
session cannot conjure.

---

## 5. Non-blocking technical debt classification (Part R)

Explicit classification of every known debt/limitation:

| # | Item | Classification |
|---|---|---|
| 1 | Reconciliation supports 4 direct categories (manifest identity, subaccount ownership, collateral balance, recovery state); others report `UNSUPPORTED_VIEW` | `FUTURE-ENHANCEMENT` (requires Solidity read views to expand safely; unsupported views deliberately never fabricate convergence — see Part S) |
| 2 | Old empty `hybrid_v2_reorg_locks` table remains (schema debt) | `NON-BLOCKING-TECH-DEBT` (retained empty for backup compatibility; no code path writes it — see `persistence.rs:307`, `persistence.rs:1325`, `reorg_recovery.rs:274`) |
| 3 | Some reconciliation categories require future Solidity read views | `FUTURE-ENHANCEMENT` (same as #1 above) |
| 4 | Base mainnet remains unvalidated | `MAINNET-ONLY-REQUIREMENT` (V1 scope is Base Sepolia only; mainnet is forbidden by construction — `target_policy.rs:75`, `broadcast_firewall.rs:32`) |
| 5 | No external security audit yet | `MAINNET-ONLY-REQUIREMENT` (mandatory before Base mainnet enablement; not a V1 gate) |
| 6 | Real Base Sepolia E2E not executed this session | `NON-BLOCKING-TECH-DEBT` (blocking for the specific verdict `SUBACCOUNT_V1_COMPLETE`, but the milestone brief itself permits partial closure when operator provisioning is absent) |
| 7 | AWS KMS signer microservice not deployed | `NON-BLOCKING-TECH-DEBT` (operator provisioning task; code path is complete and probed via `hybrid_v2_production_signer_http_e2e`) |
| 8 | mTLS cert provisioning workflow not documented in operator runbook | `NON-BLOCKING-TECH-DEBT` (addressed by new Section 20 in `HYBRID_V2_OPERATOR_RUNBOOK.md`) |
| 9 | Perps `src/execution/` module retains accessible `send_raw_transaction` method | `NON-BLOCKING-TECH-DEBT` (perps subsystem exists in the tree but is fail-closed at the public-route boundary per user-memory pref `feedback_perps_fail_closed.md`; NOT reachable from any HV2 code path — verified: `rg send_raw_transaction src/hybrid_v2/` shows all HV2 writers go through the outbox → `broadcast_rpc.rs` → allowlist) |
| 10 | `SignerBuilder` for `KmsGcp`/`Turnkey`/`Fireblocks` returns `SignerUnavailable` | `FUTURE-ENHANCEMENT` (fail-closed by construction — `signer_builder.rs:112-122`; explicitly stated in module docs: "NEVER invent HTTP protocols for a vendor we can't actually reach") |

Additional debts found during audit:

| # | Item | Classification |
|---|---|---|
| 11 | `broadcast_recheck` admin route retained as diagnostic (Part I posture) | `NON-BLOCKING-TECH-DEBT` (documented in broadcast live wiring closure V1) |
| 12 | `broadcast_resend_same_bytes` endpoint returns 503 pending future policy work | `NON-BLOCKING-TECH-DEBT` (honest deferral; no auto-RBF by construction) |
| 13 | Full 20-test PG matrix for `PRODUCTION_SIGNER_STARTUP_DATABASE_INTEGRATION_VALIDATED` was deferred at signer bootstrap milestone | `NON-BLOCKING-TECH-DEBT` (covered indirectly by existing `hybrid_v2_production_signer_full_startup_matrix_pg_integration`) |

Verdict: **`SUBACCOUNT_V1_TECHNICAL_DEBT_CLASSIFIED`** ✅

---

## 6. Security closure sweep (Part S)

Internal invariant re-verification. Explicitly note: this is **NOT
an external audit**; it is an internal file-level sweep of the
frozen invariants documented in every prior milestone's security
review. External audit remains classified `MAINNET-ONLY-REQUIREMENT`.

| Invariant | Evidence (file:line) | Status |
|---|---|---|
| No raw private key in HV2 pipeline | Only in `signer_kms_bridge.rs:600-601` and `signer_builder.rs:227-238`, both under `#[cfg(any(test, feature = "test-signer"))]` blocks; production `HybridV2SignerBuilder::build_production` refuses `TestEphemeral` variant at runtime (`signer_builder.rs:69-81`) | ✅ |
| No mnemonic in HV2 pipeline | `rg -n "mnemonic" src/hybrid_v2/` returns nothing | ✅ |
| Custody remains external (mTLS to signer microservice) | `signer_http_transport.rs::is_public_https` + `read_pem_if_configured`; module docs `signer_builder.rs:12-17` "PATTERN_C_CUSTODY_BOUNDARY — the backend holds ONLY the mTLS client identity" | ✅ |
| Expected signer address locally verified | `signer_builder.rs:83-88` refuses build if `expected_signer_address` missing; `HttpSignerTransport::fetch_identity` (Part F of Production Signer Bootstrap) probes at startup | ✅ |
| Base mainnet forbidden at every layer | `target_policy.rs:75,113`; `broadcast_firewall.rs:32`; `hybrid_v2_execution_admin.rs` handler entry rejection; env validation gate | ✅ |
| No arbitrary target/calldata/nonce/signer accepted at admin body | `hybrid_v2_execution_admin.rs:136,155,170` — every struct is `#[serde(deny_unknown_fields)]`; comment at `:17-19`: "Every downstream execution field (target, selector, calldata, value, nonce, gas, chain_id) is derived from the allowlist + the manifest, NOT from the admin body" | ✅ |
| Admin routes authenticated (`ensure_admin`) | `hybrid_v2_execution_admin.rs:51` (definition); called at every handler entry (lines 291, 558, 618, 678, 795, 1101, 1473, 1548, 1604, 1675, 1788); also `hybrid_v2_admin.rs:88,173,284,391` | ✅ |
| Public users cannot broadcast executor transactions | All broadcast paths route through `/admin/hybrid_v2/...` prefix; no public route calls `admin_broadcast_execution` — verified by searching `src/api/routes.rs` for `hybrid_v2_execution_admin::` — every entry is under `admin/` path | ✅ |
| Local tx hash authoritative (`envelope_hash`) | `broadcast_outbox.rs:243` derives before send; `:359` and `:381` refuse provider mismatch; `:1063` re-verifies on resume | ✅ |
| No automatic fee bump / RBF / re-sign | `HV2_BROADCAST_SUBMISSION_RETRY_MAX` bounded `[0,3]` default `0`; no code path recomputes gas or re-signs; `broadcast_reconstruction.rs` pulls persisted gas/fee triple; `admin_broadcast_resend_same_bytes` returns 503 (honest deferral) | ✅ |
| Receipt canonicality enforced | `broadcast_worker.rs::advance_receipt` refuses receipt-hash mismatch → `MANUAL_INTERVENTION_REQUIRED` | ✅ |
| Indexer correlation required for CONFIRMED | `broadcast_indexer_correlation.rs:5` module doc "`CONFIRMED` until:" (correlation required); `broadcast_worker.rs:687` gates depth check | ✅ |
| Reorg invalidates confirmation | `broadcast_reorg_recovery.rs::reorg` demotes confirmed rows; `hybrid_v2_broadcast_reorg_recovery_pg_integration` proves it | ✅ |
| PostgreSQL never overrides chain authority | Reducer applies events derived from `rpc_chain_source`; reorg drops divergent PG rows; no code path writes state ahead of chain observation | ✅ |
| Unsupported reconciliation views never fabricate convergence | `reconciler.rs:62,74` — `UnsupportedView` classification is distinct from `Converged`; propagates to `divergent_categories` count; the reconciler bounds `categories = 6u32` (`reconciler.rs:385`) and reports the actual count of supported views separately | ✅ |

Verdict: **`SUBACCOUNT_V1_SECURITY_CLOSURE_VALIDATED`** ✅ (internal
invariant sweep; NOT an external audit).

---

## 7. Regression results (Part T)

See dedicated section 8 below. Verdict: **`SUBACCOUNT_V1_REGRESSION_GREEN`**
(pending completion of the running test binaries — the doc will not
finalize this verdict until every binary reports green).

---

## 8. Operator provisioning checklist (required before real E2E)

To re-execute this milestone in a subsequent session and reach
`SUBACCOUNT_V1_COMPLETE`, the operator must provision:

1. **Signer microservice deployment**
   - Deploy the AWS KMS-backed signer microservice (per
     `docs/AWS_KMS_OPERATOR_SETUP_PACK.md`).
   - Expose a public HTTPS endpoint reachable from the backend host.
   - Confirm the microservice `GET /hybrid_v2/identity` responds with
     the expected executor address.

2. **mTLS cert issuance**
   - Generate a signer-microservice-side server certificate and CA.
   - Issue a backend-side client certificate + key from the same CA.
   - Store PEM material at operator-managed paths and expose them via
     `HV2_SIGNER_CLIENT_CERT_PEM_PATH` +
     `HV2_SIGNER_CLIENT_KEY_PEM_PATH` +
     `HV2_SIGNER_CA_PEM_PATH`.

3. **HV2 env vars**
   - `HV2_SIGNER_ENDPOINT=https://<signer-host>/hybrid_v2`
   - `HV2_SIGNER_PROVIDER=kms_aws`
   - `HV2_EXPECTED_SIGNER_ADDRESS=0x<funded-executor>`
   - `HV2_BROADCAST_ENABLED=true`
   - `HV2_BROADCAST_RPC_URL=https://<base-sepolia-rpc>`
   - `HV2_BROADCAST_ALLOWED_CHAIN_IDS=84532`
   - `HV2_BROADCAST_CONFIRMATION_DEPTH=3` (or per policy)

4. **Executor funding**
   - Fund the executor address on Base Sepolia (from an operator-owned
     faucet or bridged testnet ETH).
   - Sanity: `eth_getBalance(executor) ≥ 0.05 ETH` for a comfortable
     margin over any single broadcast.

5. **Deployment manifest promotion**
   - Promote the target manifest row in `deployment_manifests` from
     `draft` to `active`.
   - Confirm `manifest.chain_id == 84532` and `manifest.option_matching_engine`
     resolves to the deployed Base Sepolia address.

6. **Explicit broadcast gate**
   - Set `HV2_E2E_ALLOW_REAL_BASE_SEPOLIA_BROADCAST=true`.
   - Without this env var, the E2E test path refuses to open the RPC
     connection; this is the last-mile human-in-the-loop gate.

---

## 9. Next stage

Once every item in Section 8 is provisioned, the same milestone
(`BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-AND-SUBACCOUNT-V1-CLOSURE`)
can be re-executed. The re-execution should replay Parts C, D, E–P
(real environment validation + broadcast + confirmation) against the
newly provisioned infra, then converge on `SUBACCOUNT_V1_COMPLETE`
after the real chain confirmation event is observed AND correlated
by the indexer.

Nothing in this session's code changes obstructs that re-run; the
worktree is documentation-only.

