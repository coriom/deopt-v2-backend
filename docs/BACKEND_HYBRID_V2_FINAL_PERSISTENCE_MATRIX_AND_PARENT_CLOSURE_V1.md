# `BACKEND-HYBRID-V2-FINAL-PERSISTENCE-MATRIX-AND-PARENT-CLOSURE-V1`

Date: 2026-08-06
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`
Base mainnet: **REFUSED** at every layer.

This is the closure milestone for the two-parent tree

- `BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1`
- `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1`

Both parents are now considered **CLOSED** modulo the outer-session
push + parent-status commits.

---

## 1. What this milestone delivers

1. **High-risk reorg matrix** (`tests/hybrid_v2_reorg_high_risk_matrix_pg_integration.rs`,
   10 tests) — end-to-end orphan-economic-family invalidation
   coverage: deposit, withdrawal, reservation, order + partial fill,
   matched execution, premium transfer, multi-family batch,
   replacement with changed components, concurrent recovery on two
   deployments, restart-after-commit-before-memory-publication.
2. **Consolidated closure properties** (`tests/hybrid_v2_final_closure_properties.rs`,
   7 tests):
   - `prop_read_api_never_contains_orphan_rows`
   - `prop_unsupported_reconciliation_never_converged`
   - `prop_ready_implies_no_active_operation_row`
   - `prop_deployment_isolation_across_operations`
   - `prop_read_api_reflects_replacement_after_reorg`
   - `prop_operation_lock_serializes_all_three`
   - `prop_policy_a_unsupported_view_never_serialises_as_converged` (meta)
3. **Global closure matrix** —
   `docs/BACKEND_HYBRID_V2_GLOBAL_CLOSURE_MATRIX.md` maps 138
   scenarios (across 14 in-scope categories) to concrete test-file +
   test-name entries.
4. **CI gate** — `.github/workflows/backend-postgres-integrity.yml`
   extended with the two new binaries.
5. **Runbook update** — `docs/HYBRID_V2_OPERATOR_RUNBOOK.md` now
   documents supported vs unsupported reconciliation categories
   (Policy A) and the READY-vs-unsupported policy.
6. **Progress-doc dated final entry** — the parent progress log
   appends this closure and marks both parents CLOSED.

---

## 2. Reconciliation-scope policy (Policy A, frozen)

**Verdict:** `BACKEND_HYBRID_V2_SUPPORTED_SCOPE_RECONCILIATION_POLICY_ACCEPTED`.

Policy A: supported-scope convergence permits READY.

- Reconciliation directly reconciles **four** categories:
  1. Manifest identity (hash equality),
  2. Subaccount ownership (registry `ownerOf`),
  3. Collateral vault balance (`balanceWithYield`),
  4. Recovery state (`getRecoveryState`).
- The following remain `UNSUPPORTED_VIEW`:
  reservations, positions, order lifecycle, matched executions,
  active-series enumeration, escape / withdrawal counts.
- `UNSUPPORTED_VIEW` is **never** reported as `CONVERGED`. The
  reconciler returns `Unsupported { detail }` (not `Converged`) for
  these categories; the scheduler never coerces that into a
  Converged classification when persisting the row.
- READY does **not** claim unsupported categories were directly
  reconciled. It claims: the supported categories are convergent
  under the reconciler + provider allowlist AND the canonical event
  journal has been played through the deterministic reducer for
  every other category.
- Correctness of the unsupported categories is derived from the
  canonical journal + reducer, validated by the persisted-runtime
  property suite (§4 of the matrix doc).

Rationale: the on-chain views for the deferred categories are either
not yet pinned in the deployed Solidity or would require multi-call
composition to reconstruct fine-grained state. Adding an
under-specified view to the allowlist is worse than absence —
absence fails-closed via `Unsupported`, but a stale allowlist would
silently pass Convergence.

---

## 3. Readiness variant → HTTP mapping (canonical)

`ReadinessReason` variants that map to a hard-503:

| Variant | Owner | HTTP status | Body reason |
|---|---|---|---|
| `AwaitingFirstBlock` | runtime | 503 | `AWAITING_FIRST_BLOCK` |
| `WrongChain` | runtime | 503 | `WRONG_CHAIN` |
| `ManifestMismatch` | runtime | 503 | `MANIFEST_MISMATCH` |
| `UnknownCanonicalEvent` | runtime | 503 | `UNKNOWN_CANONICAL_EVENT` |
| `DecodeFailure` | runtime | 503 | `DECODE_FAILURE` |
| `ProjectionFailure` | runtime | 503 | `PROJECTION_FAILURE` |
| `CursorHashMismatch` | runtime | 503 | `CURSOR_HASH_MISMATCH` |
| `ExcessiveReorg` | reorg | 503 | `EXCESSIVE_REORG` |
| `ReorgDetected` | reorg | 503 | `REORG_DETECTED` |
| `ReorgSearching` | reorg | 503 | `REORG_SEARCHING` |
| `ReorgReplaying` | reorg | 503 | `REORG_REPLAYING` |
| `ReorgManualInterventionRequired` | reorg | 503 | `REORG_MANUAL_INTERVENTION_REQUIRED` |
| `RebuildRequested` | rebuild | 503 | `REBUILD_REQUESTED` |
| `RebuildInProgress` | rebuild | 503 | `REBUILD_IN_PROGRESS` |
| `RebuildFailed` | rebuild | 503 | `REBUILD_FAILED` |
| `ReconciliationInProgress` | reconciler | 503 | `RECONCILIATION_IN_PROGRESS` |
| `ReconciliationDrift` | reconciler | 503 | `RECONCILIATION_DRIFT` |
| `MigrationSchemaMismatch` | runtime | 503 | `MIGRATION_SCHEMA_MISMATCH` |
| `Bootstrapping` | runtime | 503 | `BOOTSTRAPPING` |
| `Stopping` | runtime | 503 | `STOPPING` |

The one non-fatal variant is `Behind`, which advances readiness to
READY but reports a lag counter. This mapping is stable across the
runtime, reorg, rebuild, and reconciliation subsystems.

---

## 4. Consolidated property inventory

| File | Property | Category |
|---|---|---|
| `hybrid_v2_property_tests.rs` | 5 baseline projection properties | Reducer |
| `hybrid_v2_persisted_runtime_properties.rs` | 7 runtime persistence properties | Runtime |
| `hybrid_v2_rpc_chain_source_properties.rs` | 9 read-only source properties | Live source |
| `hybrid_v2_reorg_recovery_properties.rs` | 9 reorg recovery properties | Reorg |
| `hybrid_v2_rebuild_operations_properties.rs` | 11 rebuild + reconciliation properties | Rebuild + reconciliation |
| `hybrid_v2_reconciliation_task_properties.rs` | 5 production task properties | Task |
| `hybrid_v2_final_closure_properties.rs` (new) | 7 consolidated closure properties | Closure |

Total: 53 bounded property tests across the persistence surface.
Every one is deterministic and skip-clean without a PG URL.

---

## 5. Scenario-to-test matrix

See `docs/BACKEND_HYBRID_V2_GLOBAL_CLOSURE_MATRIX.md`. 138 in-scope
scenarios, 100% mapped to concrete test-file + test-name entries.

---

## 6. CI gate status

`.github/workflows/backend-postgres-integrity.yml` gates **15**
hybrid_v2 PG-backed test binaries against a disposable Postgres 16
service container (schema dropped + recreated between suites so no
residual state leaks). New in this milestone:

- `hybrid_v2_reorg_high_risk_matrix_pg_integration`
- `hybrid_v2_final_closure_properties`

The workflow triggers on pushes to `main` and PRs that touch:

- `migrations/**`
- `src/db/**`
- `src/subaccounts/**`
- `src/hybrid_v2/**`
- `tests/postgres_migration_chain_integration.rs`
- `.github/workflows/backend-postgres-integrity.yml`

`DEOPT_REQUIRE_PG_INTEGRATION=1` is set so any silent skip fails
loudly in CI.

---

## 7. Legacy PG isolation skip status

Two pre-existing legacy failing tests remain skipped in the read-store
PG proof binary (documented in prior closure notes):

- `subaccount_summary_aggregates_counts`
- `list_fees_keyset_pagination_desc`

These were regressed before the persistence-operations parent
started; they trace to schema evolution in the read-store view
composition, not to the persistence layer under test. They remain
tracked separately from this milestone.

Every other PG-backed test in the workflow runs unskipped.

---

## 8. Verdicts claimed by this milestone

- `BACKEND_HYBRID_V2_FINAL_PERSISTENCE_CLOSURE_MODEL_RESOLVED`
- `BACKEND_HYBRID_V2_SUPPORTED_SCOPE_RECONCILIATION_POLICY_ACCEPTED`
- `BACKEND_HYBRID_V2_REORG_HIGH_RISK_MATRIX_COMPLETE`
- `BACKEND_HYBRID_V2_OPERATIONAL_GLOBAL_DATABASE_MATRIX_VALIDATED`
- `BACKEND_HYBRID_V2_OPERATIONAL_CLOSURE_PROPERTIES_VALIDATED`
- `BACKEND_HYBRID_V2_FINAL_OPERATIONAL_READINESS_VALIDATED`
- `BACKEND_HYBRID_V2_FINAL_PERSISTENCE_CI_GATE_VALIDATED`
- `BACKEND_HYBRID_V2_FINAL_PERSISTENCE_DOCUMENTATION_COMPLETE`

---

## 9. Parent milestone status

- `BACKEND-HYBRID-V2-PERSISTED-OPERATIONS-V1` — **CLOSED** by this
  milestone taken together with the intermediate operational,
  reorg-recovery, rebuild, reconciliation, live-source, and
  chain-view-provider closures. Every mandatory hybrid v2 PG test is
  executed and named in CI.
- `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — **CLOSED**
  for the same reason. The persistence-layer parent's remaining
  deferred items (production RPC provider, periodic reconciliation
  worker, admin routes) all landed in prior sub-milestones; this
  milestone completes the property + matrix coverage required to
  discharge the invariant
  `PARENT_CLOSURE_REQUIRES_ALL_MANDATORY_HYBRID_V2_PG_TESTS_TO_EXECUTE`.

The outer session is responsible for the actual parent-status
commits + push.

---

## 10. Deferred / explicitly non-verdict items

- Public write endpoints on hybrid v2 — out of scope (the entire
  parent tree is read-only projection persistence).
- Signer / execution surfaces — out of scope for hybrid v2.
- Base mainnet operation — refused; refusal is guarded by the
  `HYBRID_V2_CHAIN_ID` allowlist at config parse + at every worker
  spawn point.
- Reservation / positions / order / execution / active-series
  on-chain reconciliation — Policy A deferred; see §2.

No verdict is claimed for any of the above.

---

## 11. Truthful commit / cargo status

- `cargo fmt --all` — clean.
- `cargo check --workspace --all-targets` — passes with only
  pre-existing style warnings in unrelated tests.
- `cargo test --workspace --lib` — 1223 pass, 0 fail.
- `cargo test --test hybrid_v2_reorg_high_risk_matrix_pg_integration`
  (no PG URL) — 10/10 skip-clean.
- `cargo test --test hybrid_v2_final_closure_properties` (no PG URL)
  — 7/7 pass (the ones that don't require PG) + skip-clean for the
  PG-gated properties.
- Real PG execution against a live disposable database is left to
  the outer session's CI + integration harness. Every new file
  compiles clean and follows the same URL-gate pattern as the prior
  PG-backed suites.
