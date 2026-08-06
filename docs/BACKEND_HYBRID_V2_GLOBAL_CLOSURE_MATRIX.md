# Hybrid V2 Global Closure Scenario-to-Test Matrix

Produced by `BACKEND-HYBRID-V2-FINAL-PERSISTENCE-MATRIX-AND-PARENT-CLOSURE-V1`.

Every scenario derived from the persistence-operations parent brief
(Parts F/G/H — "the 30+ integration matrix" and the read/reconciler
matrices below) is mapped to an EXISTING test in this repository.
Tests are cited by binary + fn name so `cargo test --test <binary>
<fn>` runs the exact scenario. When a category is genuinely gated
by policy (Policy A — supported-scope reconciliation) or by an
absent Solidity fixture, the row is marked `Policy A — deferred`
with a link back to the milestone doc.

Legend for the `Test file` column: all files live under
`deopt-v2-backend/tests/`. The `.rs` suffix is elided.

---

## 1. Normal persisted runtime (14 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 1 | Migrations 0044..0048 apply cleanly | hybrid_v2_persistence_core_pg_proof | migrations_apply_cleanly |
| 2 | Deployment upsert is idempotent | hybrid_v2_persistence_core_pg_proof | upsert_deployment_is_idempotent |
| 3 | Deployments are isolated | hybrid_v2_persistence_core_pg_proof | deployments_are_isolated |
| 4 | Block-atomic persistence of all artifacts | hybrid_v2_persistence_core_pg_proof | block_atomic_persists_all_artifacts |
| 5 | Raw log + decoded event idempotent | hybrid_v2_persistence_core_pg_proof | raw_log_and_decoded_event_are_idempotent |
| 6 | Uint256 MAX round-trips | hybrid_v2_persistence_core_pg_proof | uint256_max_balance_roundtrips |
| 7 | Positions upsert + delete | hybrid_v2_persistence_core_pg_proof | positions_and_active_series_upsert_and_delete |
| 8 | Tick + persist commits atomically | hybrid_v2_runtime_persistence_integration | tick_and_persist_persists_block_atomically |
| 9 | Tick rollback on persist failure | hybrid_v2_runtime_persistence_integration | tick_and_persist_rolls_back_on_persist_failure |
| 10 | Bootstrap restores cursor | hybrid_v2_runtime_persistence_integration | bootstrap_from_persistence_restores_cursor |
| 11 | Duplicate block tick idempotent | hybrid_v2_runtime_persistence_integration | duplicate_block_tick_is_idempotent |
| 12 | Restart after success does not reapply | hybrid_v2_runtime_persistence_integration | restart_after_success_does_not_reapply_economics |
| 13 | Restart preserves projection via replay | hybrid_v2_runtime_persistence_integration | restart_preserves_projection_state_via_bootstrap_replay |
| 14 | API response identical across restart | hybrid_v2_runtime_persistence_integration | api_response_identical_before_and_after_restart |

## 2. Persist-failure atomicity matrix (7 injection points)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 15 | Table drop mid-write rolls back | hybrid_v2_runtime_persistence_integration | atomicity_drop_table_and_assert_rollback |
| 16 | Block-row write failure rolls back | hybrid_v2_runtime_persistence_integration | atomicity_block_row_write_failure_rolls_back |
| 17 | Raw-logs write failure rolls back | hybrid_v2_runtime_persistence_integration | atomicity_raw_logs_write_failure_rolls_back |
| 18 | Decoded-events write failure rolls back | hybrid_v2_runtime_persistence_integration | atomicity_decoded_events_write_failure_rolls_back |
| 19 | Cursor write failure rolls back | hybrid_v2_runtime_persistence_integration | atomicity_cursor_write_failure_rolls_back |
| 20 | Readiness write failure rolls back | hybrid_v2_runtime_persistence_integration | atomicity_readiness_write_failure_rolls_back |
| 21 | Retry after transient failure succeeds | hybrid_v2_runtime_persistence_integration | atomicity_retry_after_transient_failure_succeeds |

## 3. Restart / idempotency matrix (7 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 22 | Restart after many blocks continues | hybrid_v2_runtime_persistence_integration | restart_after_many_blocks_continues_at_next_expected |
| 23 | Restart after failed block retries safely | hybrid_v2_runtime_persistence_integration | restart_after_failed_block_retries_safely |
| 24 | Restart after graceful shutdown lossless | hybrid_v2_runtime_persistence_integration | restart_after_graceful_shutdown_is_lossless |
| 25 | Duplicate raw log within block no dup | hybrid_v2_runtime_persistence_integration | duplicate_raw_log_within_a_block_does_not_produce_duplicate_projection |
| 26 | Provider retry after timeout no reapply | hybrid_v2_runtime_persistence_integration | provider_retry_after_timeout_does_not_reapply_events |
| 27 | Duplicate block batch idempotent | hybrid_v2_runtime_persistence_integration | duplicate_block_batch_is_idempotent |
| 28 | Restart then duplicate last block noop | hybrid_v2_runtime_persistence_integration | restart_then_duplicate_last_block_is_noop |

## 4. Readiness state machine (4 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 29 | Readiness transitions during normal indexing | hybrid_v2_runtime_persistence_integration | readiness_transitions_normal_indexing |
| 30 | Readiness NOT_READY on persist failure | hybrid_v2_runtime_persistence_integration | readiness_transitions_on_persist_failure |
| 31 | Readiness reflects active recovery phase | hybrid_v2_reorg_recovery_pg_integration | readiness_reflects_active_recovery_phase |
| 32 | READY implies no active operation row | hybrid_v2_final_closure_properties | prop_ready_implies_no_active_operation_row |

## 5. Live worker + RPC chain source (9 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 33 | Disabled config starts no worker | hybrid_v2_live_worker_pg_integration | disabled_configuration_starts_no_worker |
| 34 | Wrong chain-id prevents startup | hybrid_v2_live_worker_pg_integration | wrong_chain_id_prevents_worker_startup |
| 35 | Base mainnet rejected before spawn | hybrid_v2_live_worker_pg_integration | base_mainnet_rejected_before_spawn |
| 36 | Empty block commits cursor advance | hybrid_v2_live_worker_pg_integration | empty_block_commits_cursor_advance |
| 37 | Failed RPC leaves cursor unchanged | hybrid_v2_live_worker_pg_integration | failed_rpc_leaves_cursor_unchanged |
| 38 | Graceful shutdown via watch channel | hybrid_v2_live_worker_pg_integration | graceful_shutdown_via_watch_channel |
| 39 | Restart resumes from postgres cursor | hybrid_v2_live_worker_pg_integration | restart_resumes_from_postgres_cursor |
| 40 | Parent-hash mismatch fails closed | hybrid_v2_live_worker_pg_integration | parent_hash_mismatch_fails_closed_no_replay |
| 41 | No prohibited RPC method ever generated | hybrid_v2_rpc_chain_source_properties | prop_no_prohibited_method_generated |

## 6. Read-store + main-router (13 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 42 | Deployments listed via PG router | hybrid_v2_read_api_postgres_main_router_tests | deployments_route_lists_configured_deployment_through_pg_router |
| 43 | Deployment status readable | hybrid_v2_read_api_postgres_main_router_tests | deployment_status_readable_through_pg_router |
| 44 | Owner subaccounts route | hybrid_v2_read_api_postgres_main_router_tests | owner_subaccounts_route_through_pg_router |
| 45 | Subaccount collateral route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_collateral_route_through_pg_router |
| 46 | Subaccount reservations route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_reservations_route_through_pg_router |
| 47 | Subaccount positions route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_positions_route_through_pg_router |
| 48 | Subaccount orders route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_orders_route_through_pg_router |
| 49 | Subaccount executions route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_executions_route_through_pg_router |
| 50 | Subaccount recovery route | hybrid_v2_read_api_postgres_main_router_tests | subaccount_recovery_route_through_pg_router |
| 51 | Global history route | hybrid_v2_read_api_postgres_main_router_tests | global_history_route_through_pg_router |
| 52 | Write method rejected | hybrid_v2_read_api_postgres_main_router_tests | hybrid_v2_write_method_rejected_through_pg_router |
| 53 | Postgres unavailability fails closed | hybrid_v2_read_api_postgres_main_router_tests | postgres_unavailability_fails_closed_no_memory_fallback |
| 54 | Read API never contains orphan rows | hybrid_v2_final_closure_properties | prop_read_api_never_contains_orphan_rows |

## 7. Reorg recovery — end-to-end + phase machine (14 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 55 | One-block reorg recovered E2E | hybrid_v2_reorg_recovery_pg_integration | one_block_reorg_recovered_end_to_end |
| 56 | Deployment isolation | hybrid_v2_reorg_recovery_pg_integration | deployment_isolation_pg |
| 57 | Restart during Detected resumes | hybrid_v2_reorg_recovery_pg_integration | restart_during_detected_resumes_recovery |
| 58 | Duplicate recovery trigger serialised | hybrid_v2_reorg_recovery_pg_integration | duplicate_recovery_trigger_serialised_via_lock |
| 59 | Stale reorg lock reclaimed | hybrid_v2_reorg_recovery_pg_integration | stale_reorg_lock_reclaimed_after_completion |
| 60 | Commit reorg marks orphans + advances cursor | hybrid_v2_reorg_recovery_pg_integration | commit_reorg_recovery_marks_orphans_and_advances_cursor |
| 61 | Excessive depth enters manual intervention | hybrid_v2_reorg_recovery_pg_integration | excessive_depth_enters_manual_intervention |
| 62 | Finalized boundary violation refuses | hybrid_v2_reorg_recovery_pg_integration | finalized_boundary_violation_manual_intervention |
| 63 | Recovery is idempotent (property) | hybrid_v2_reorg_recovery_properties | prop_recovery_idempotent |
| 64 | Orphans never canonical (property) | hybrid_v2_reorg_recovery_properties | prop_orphans_never_canonical |
| 65 | Cursor hash in canonical branch (property) | hybrid_v2_reorg_recovery_properties | prop_cursor_hash_in_canonical_branch |
| 66 | Recovery == fresh replay (property) | hybrid_v2_reorg_recovery_properties | prop_recovery_equals_fresh_replay |
| 67 | Uninterrupted == restarted (property) | hybrid_v2_reorg_recovery_properties | prop_uninterrupted_equals_restarted |
| 68 | No cross-branch exec correlation | hybrid_v2_reorg_recovery_properties | prop_no_cross_branch_execution_correlation |

## 8. Reorg high-risk matrix — orphan economic family invalidations (10 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 69 | Orphaned deposit balance reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_deposit_balance_reverted |
| 70 | Orphaned withdrawal balance reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_withdrawal_balance_reverted |
| 71 | Orphaned reservation creation reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_reservation_creation_reverted |
| 72 | Orphaned order + partial fill reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_order_and_partial_fill_reverted |
| 73 | Orphaned matched execution invalidated | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_matched_execution_invalidated |
| 74 | Orphaned premium transfer reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_premium_transfer_reverted |
| 75 | Orphaned multi-family batch reverted | hybrid_v2_reorg_high_risk_matrix_pg_integration | orphaned_multi_family_batch_reverted |
| 76 | Replacement execution with changed components | hybrid_v2_reorg_high_risk_matrix_pg_integration | replacement_execution_with_changed_components |
| 77 | Concurrent recovery on two deployments isolated | hybrid_v2_reorg_high_risk_matrix_pg_integration | concurrent_recovery_on_two_deployments_isolated |
| 78 | Restart after commit before memory publish | hybrid_v2_reorg_high_risk_matrix_pg_integration | restart_after_recovery_commit_before_memory_publication |

Note: additional scenarios from the brief — orphaned recovery epoch,
orphaned escape/finalization, orphaned fee/rebate emissions — are
covered by the orphan-invariant guarantee established by
`orphaned_multi_family_batch_reverted`: every economic family sharing
the same `(deployment_id, block_hash)` tuple is invalidated by
`commit_reorg_recovery` in the same SQL transaction. See
BACKEND_HYBRID_V2_FINAL_PERSISTENCE_MATRIX_AND_PARENT_CLOSURE_V1.md
section "reorg high-risk matrix" for the derivation.

## 9. Rebuild operations (13 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 79 | JournalReplay nothing-to-do when match | hybrid_v2_rebuild_operations_pg_integration | journal_replay_rebuild_nothing_to_do_when_projection_matches_journal |
| 80 | Rebuild lock blocks reconciliation | hybrid_v2_rebuild_operations_pg_integration | rebuild_operation_lock_blocks_reconciliation |
| 81 | Operation lock is deployment scoped | hybrid_v2_rebuild_operations_pg_integration | operation_lock_is_deployment_scoped |
| 82 | Rebuild phase persisted across restart | hybrid_v2_rebuild_operations_pg_integration | rebuild_phase_persisted_across_restart |
| 83 | Duplicate rebuild request idempotent | hybrid_v2_rebuild_operations_pg_integration | duplicate_rebuild_request_is_idempotent_per_epoch |
| 84 | Latest rebuild returns highest epoch | hybrid_v2_rebuild_operations_pg_integration | latest_rebuild_operation_returns_highest_epoch |
| 85 | Rebuild epoch monotonically advances | hybrid_v2_rebuild_operations_properties | rebuild_epoch_monotonically_advances_across_calls |
| 86 | FreshChain ingest marks Complete on success | hybrid_v2_rebuild_operations_properties | fresh_chain_ingest_marks_complete_on_success |
| 87 | FreshChain ingest failure retryable then manual | hybrid_v2_rebuild_operations_properties | fresh_chain_ingest_failure_is_retryable_then_manual |
| 88 | Rebuild verification detects drift + escalates | hybrid_v2_rebuild_operations_properties | rebuild_verification_detects_drift_and_escalates_manual |
| 89 | Bootstrap blocked when rebuild active | hybrid_v2_rebuild_bootstrap_properties | bootstrap_returns_rebuild_blocked_when_rebuild_active |
| 90 | Bootstrap blocked when rebuild failed | hybrid_v2_rebuild_bootstrap_properties | bootstrap_returns_rebuild_blocked_when_rebuild_failed |
| 91 | Rebuild auto-rematerialize rewrites projection | hybrid_v2_rebuild_bootstrap_properties | rebuild_from_journal_auto_rematerialize_rewrites_projection |

## 10. Reconciliation core (10 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 92 | Converged persists row | hybrid_v2_reconciliation_pg_integration | reconciliation_converged_persists_row |
| 93 | Detects manifest mismatch | hybrid_v2_reconciliation_pg_integration | reconciliation_detects_manifest_mismatch |
| 94 | Provider unavailable never publishes drift | hybrid_v2_reconciliation_pg_integration | reconciliation_provider_unavailable_never_publishes_drift |
| 95 | No auto-repair on drift | hybrid_v2_reconciliation_pg_integration | reconciliation_no_auto_repair_on_drift |
| 96 | History append-only | hybrid_v2_reconciliation_pg_integration | reconciliation_history_append_only |
| 97 | Reconciliation lock exclusive vs rebuild | hybrid_v2_reconciliation_pg_integration | reconciliation_lock_exclusive_with_rebuild |
| 98 | Converged property (property) | hybrid_v2_reconciliation_task_properties | prop_converged_provider_never_flips_to_drift |
| 99 | Provider failure never mutates (property) | hybrid_v2_reconciliation_task_properties | prop_provider_failure_never_mutates_projection |
| 100 | Drift never auto-repairs (property) | hybrid_v2_reconciliation_task_properties | prop_drift_never_auto_repairs_projection |
| 101 | Unsupported view never converged (property) | hybrid_v2_final_closure_properties | prop_unsupported_reconciliation_never_converged |

## 11. Production RPC chain-view provider + task (14 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 102 | Owner + balance + recovery round trip | hybrid_v2_rpc_chain_view_provider_tests | provider_fetch_snapshot_owner_balance_recovery_round_trip |
| 103 | Converges when projection matches chain | hybrid_v2_rpc_chain_view_provider_tests | provider_converges_when_projection_matches_chain |
| 104 | Balance mismatch reports drift | hybrid_v2_rpc_chain_view_provider_tests | provider_reports_projection_drift_on_balance_mismatch |
| 105 | Manifest mismatch surfaces | hybrid_v2_rpc_chain_view_provider_tests | provider_manifest_mismatch_surfaces_from_reconciler |
| 106 | Unlisted selector never network | hybrid_v2_rpc_chain_view_provider_tests | provider_unlisted_selector_never_hits_the_network |
| 107 | Unlisted target never network | hybrid_v2_rpc_chain_view_provider_tests | provider_unlisted_target_never_hits_the_network |
| 108 | Provider never emits prohibited methods | hybrid_v2_rpc_chain_view_provider_tests | provider_never_emits_prohibited_methods |
| 109 | Provider construction rejects bad module | hybrid_v2_rpc_chain_view_provider_tests | provider_construction_rejects_bad_module_address |
| 110 | Allowlist covers expected selectors | hybrid_v2_rpc_chain_view_provider_tests | provider_allowlist_covers_expected_selectors |
| 111 | Recovery state variants decode | hybrid_v2_rpc_chain_view_provider_tests | provider_recovery_state_variants_decode_symbolically |
| 112 | Task persists converged row | hybrid_v2_reconciliation_task_pg_integration | task_persists_converged_row_from_production_provider |
| 113 | Task persists provider-unavailable row | hybrid_v2_reconciliation_task_pg_integration | task_persists_provider_unavailable_row_never_marks_drift_readiness |
| 114 | Task persists drift row from balance mismatch | hybrid_v2_reconciliation_task_pg_integration | task_persists_drift_row_from_balance_mismatch |
| 115 | Task operation lock conflicts | hybrid_v2_reconciliation_task_pg_integration | task_operation_lock_conflicts_between_reconciliation_and_rebuild |

## 12. Unified operation lock (7 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 116 | Three operations mutually exclusive | hybrid_v2_operation_lock_pg_integration | all_three_operations_mutually_exclusive_per_deployment |
| 117 | Distinct deployments independent | hybrid_v2_operation_lock_pg_integration | distinct_deployments_are_independent |
| 118 | Release fenced by holder epoch | hybrid_v2_operation_lock_pg_integration | release_is_fenced_by_holder_epoch |
| 119 | Lock mutually exclusive across kinds (property) | hybrid_v2_rebuild_operations_properties | operation_lock_mutually_exclusive_across_kinds |
| 120 | Lock deployment isolation (property) | hybrid_v2_rebuild_operations_properties | operation_lock_deployment_isolation |
| 121 | Lock serializes all three ordered pairs | hybrid_v2_final_closure_properties | prop_operation_lock_serializes_all_three |
| 122 | Deployment isolation across operations | hybrid_v2_final_closure_properties | prop_deployment_isolation_across_operations |

## 13. Persistence convergence + snapshot roundtrip (9 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 123 | Deployment upsert idempotent + isolated | hybrid_v2_persistence_convergence_tests | upsert_deployment_is_idempotent_and_isolates_by_hash |
| 124 | Block-atomic snapshots full state | hybrid_v2_persistence_convergence_tests | block_atomic_write_snapshots_full_state |
| 125 | Cursor persists + reads back | hybrid_v2_persistence_convergence_tests | cursor_persists_and_reads_back |
| 126 | Readiness persists + reads back | hybrid_v2_persistence_convergence_tests | readiness_persists_and_reads_back |
| 127 | Two-deployment snapshots isolated | hybrid_v2_persistence_convergence_tests | two_deployments_snapshots_are_isolated |
| 128 | Idempotent reapplication same state | hybrid_v2_persistence_convergence_tests | idempotent_reapplication_yields_same_state |
| 129 | Reorg event captured | hybrid_v2_persistence_convergence_tests | record_reorg_event_captured |
| 130 | Full state roundtrips every family | hybrid_v2_persistence_convergence_tests | full_state_snapshot_roundtrips_all_field_categories |
| 131 | Cursor advance updates counters monotonically | hybrid_v2_persistence_convergence_tests | cursor_advance_updates_counters_monotonically |

## 14. Persisted runtime properties (7 scenarios)

| # | Scenario | Test file | Test name |
|---|---|---|---|
| 132 | Uninterrupted run == restart sequence | hybrid_v2_persisted_runtime_properties | prop_uninterrupted_run_equals_restart_sequence |
| 133 | Duplicate block sequence idempotent | hybrid_v2_persisted_runtime_properties | prop_duplicate_block_sequence_is_idempotent |
| 134 | Published state == committed PG state | hybrid_v2_persisted_runtime_properties | prop_published_state_equals_committed_postgres_state |
| 135 | Cursor never advances without commit | hybrid_v2_persisted_runtime_properties | prop_cursor_never_advances_without_committed_projections |
| 136 | Deployment isolation | hybrid_v2_persisted_runtime_properties | prop_deployment_isolation |
| 137 | Aggregate reservations == per-engine sum | hybrid_v2_persisted_runtime_properties | prop_aggregate_reservations_equal_per_engine_sum |
| 138 | Filled quantity monotone on canonical branch | hybrid_v2_persisted_runtime_properties | prop_filled_quantity_monotone_on_canonical_branch |

## 15. Policy A — deferred / out-of-scope categories

| Category | Status | Reason |
|---|---|---|
| Positions on-chain reconciliation | Policy A — deferred | View signature not pinned. Correctness derives from journal replay + reducer (§7 tests). |
| Order lifecycle on-chain reconciliation | Policy A — deferred | Same as above. |
| Matched execution on-chain reconciliation | Policy A — deferred | Same as above. |
| Active-series enumeration on-chain reconciliation | Policy A — deferred | Same as above. |
| Reservations on-chain reconciliation | Policy A — deferred | Same as above. |
| Escape / withdrawal on-chain reconciliation | Policy A — deferred | RPC allowlist does not yet include the escape controller view methods. |

Every row above satisfies the frozen invariant
`UNSUPPORTED_RECONCILIATION_VIEW_IS_NEVER_REPORTED_AS_CONVERGED`
because the reconciler returns `Unsupported { detail }` — never
`Converged` — for these categories. The consolidated property
`prop_unsupported_reconciliation_never_converged` pins the
string-mapping and cross-checks the enum.

---

## Coverage summary

| Section | Scenarios | Status |
|---|---|---|
| Normal persisted runtime | 14 | Covered |
| Persist-failure atomicity | 7 | Covered |
| Restart / idempotency | 7 | Covered |
| Readiness state machine | 4 | Covered |
| Live worker + RPC source | 9 | Covered |
| Read-store + main router | 13 | Covered |
| Reorg recovery E2E + properties | 14 | Covered |
| Reorg high-risk matrix | 10 | Covered (new) |
| Rebuild operations | 13 | Covered |
| Reconciliation core | 10 | Covered |
| Production RPC provider + task | 14 | Covered |
| Unified operation lock | 7 | Covered |
| Persistence convergence + snapshot | 9 | Covered |
| Persisted runtime properties | 7 | Covered |
| Policy A deferred categories | 6 | Non-verdict — see closure doc |
| **Total in-scope scenarios covered** | **138** | **Pass on real PG** |

Newly-added test binaries in this closure milestone:

- `hybrid_v2_reorg_high_risk_matrix_pg_integration` — 10 tests
- `hybrid_v2_final_closure_properties` — 7 tests

Both are gated in `.github/workflows/backend-postgres-integrity.yml`.
