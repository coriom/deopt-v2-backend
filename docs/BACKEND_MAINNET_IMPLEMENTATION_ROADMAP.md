# Backend mainnet implementation roadmap

**Posture:** READ-ONLY backend implementation roadmap. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet. No
source patch in this milestone.** Companion to
`PREBUILD_TO_BUILD_HANDOFF.md`. Catalogues the backend impl tracks
required between policy-closure and mainnet first-live-smoke, with
test strategy + done criteria per track.

**Date:** 2026-06-09

---

## 0. Hard stops (apply to every track)

```text
NO mainnet broadcast before `should_broadcast` exists.
NO mainnet broadcast before KMS/HSM/MPC signer path exists
   (or explicit waiver — see PREBUILD_TO_BUILD_HANDOFF.md §6.2).
NO mainnet broadcast before monitoring exists
   (or explicit waiver — see PREBUILD_TO_BUILD_HANDOFF.md §6.3).
NO mainnet broadcast while rebate profiles can create negative
   effective fees with rebateReserve=0 (Cluster 4 launch invariant).
```

---

## 1. Track summaries

### 1.1 `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`

| Aspect | Value |
|---|---|
| Source spec | `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` (33-line pseudocode) |
| Cluster 4 launch invariant | added — verifier sweep on profile signs + rebateReserve = 0 |
| Cluster 2 §6.6 | precheck layer (chainId / target / selector / max value 0 / max gas / nonce / rate / from-address recovery) lives in signer service; mirror in backend `should_broadcast` for early reject |
| Gap-list refs | C-4 / C-5 / W-3 / C-6 / C-7 / C-8 / C-15 |
| Touches | `src/options/service.rs`, new module e.g. `src/options/broadcast_policy.rs`, `src/options/execution.rs` |
| Returns | `(bool decision, enum reason)` per §8 pseudocode |
| Auditor anchor | Q-34 (launch invariant) + Q-31 (R-6 bright line) |

### 1.2 `MAINNET-BE-SIGNER-SERVICE-DESIGN` (read-only design)

| Aspect | Value |
|---|---|
| Source spec | Cluster 2 §2 Pattern C + §5 implementation impact |
| Output | `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` |
| Scope | mTLS server topology + §6.6 policy layer + KMS adapter interface + per-sign log + failover client + emergency disable + health endpoint |
| Vendor name | NOT recorded in tracked docs; offline binder only |
| Deployment shape | VPC-isolated; no public ingress |
| Auditor anchor | Q-26 / Q-27 (engagement package §7.6) + Cluster 2 redacted §1.4 sub-decisions |

### 1.3 `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER`

| Aspect | Value |
|---|---|
| Source spec | Cluster 2 §6 (BE custody rules) + `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md §13` (mainnet KMS lift) |
| Touches | `src/execution/signer.rs:26 from_private_key` retained + new `KmsRemoteSigner::from_service_endpoint`; `src/execution/config.rs:118` enforces refuse-env-keyed-on-mainnet; `src/config/env.rs` reads new keys; `src/options/service.rs:1166` + `:1213` swap to `RemoteSigner` trait; new module `src/execution/remote_signer.rs` |
| New env keys | `BACKEND_SIGNER_ENDPOINT`, mTLS cert paths |
| Startup guard | REFUSE `EXECUTOR_PRIVATE_KEY` on `chain_id=8453`; REQUIRE `BACKEND_SIGNER_ENDPOINT` for mainnet broadcast |
| Sepolia integration test | end-to-end Sepolia broadcast through new `RemoteSigner` path before mainnet activation |
| Auditor anchor | Q-26 (KMS non-extractable) + Q-27 (allowlist correctness) |

### 1.4 `BACKEND-MONITORING-ALERTS-WIRING`

| Aspect | Value |
|---|---|
| Source spec | `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` (V1) + Cluster 3 §4.3 BE funding ladder + Cluster 4 §10 InsuranceFund + rebate-DEFER alerts |
| Touches | metric collector wire-up; alert routing config (PagerDuty + Discord); runbook docs; Grafana dashboard JSON |
| Metrics | per §2 of monitoring spec — signer + broadcast + economics + lifecycle + subsidy + chain controls (R5 drift gauge); new Cluster 3 + 4 ratios |
| Alerts | per §3 / §4 + new Cluster 3 (BE_BAL_LOW tightened ladder; BE_DRAIN_PENDING) + new Cluster 4 (INSURANCE_BELOW_TARGET / INSURANCE_NEAR_DEPLETION / REBATE_RESERVE_NONZERO_AT_LAUNCH / EFFECTIVE_NEGATIVE_PPM_AT_LAUNCH / PFV_FEE_BALANCE_GROWTH_STALL) |
| Runbook | `RUNBOOK_BACKEND_EXECUTOR.md` (one-pager per alert with diagnostic queries) — MON-6 |
| Synthetic-fire test | quarterly drill — MON-8 |
| Auditor anchor | Q-37 (Cluster 4 operator allowlist) + monitoring closure for minimum-pass §10 |

### 1.4.1 `EXECUTOR-HEALTH-ENDPOINT-V2` (operator-side JSON summary)

| Aspect | Value |
|---|---|
| Source spec | this doc §1.4 + `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` + `BACKEND_VAULT_OBSERVABILITY_USE_TYPED_CONFIG_RESULT.md` next-milestone recommendation |
| Output | `GET /executor/health/v2` — non-sensitive JSON envelope for admin UI / frontend / operator consumers that cannot scrape Prometheus |
| Touches | new `src/api/executor_health_v2.rs`; `src/api/routes.rs` (route + handler + 4 integration tests) |
| Schema | groups: service / execution_flags / signer / policy_gate / live_provider_config / chain_state_last_seen / economics_last_seen / r5 / recent_policy_decisions / recent_signer_events / observability / warnings / hard_stops / not_tracked_yet / overall_status / reasons |
| Status logic | green / yellow / red; red is reserved for custody-policy hard stops (mainnet local-signer attempt, mainnet env-key seated, OME paused, BE not executor, R5 drift) |
| Secret-safety | redaction unit + integration tests pin the contract that no private key, RPC URL, signer endpoint, or admin token can appear |
| Auditor anchor | non-launch-critical; closes the operator-UX gap from §1.4 (Prometheus-only consumers had no admin/frontend JSON path) |
| Posture | SHIPPED 2026-06-10 — 926 / 926 backend tests green; see `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` |

### 1.5 `OPTION-EXECUTION-TX-VISIBILITY-FIX`

| Aspect | Value |
|---|---|
| Bug | `/executor/transactions/<intent>` returns `[]` for option intents despite `option_execution_transactions` DB table carrying the row |
| Gap-list ref | C-12 |
| Touches | `src/api/routes.rs` route handler; refer to `option_execution_transactions` for option intents (existing legacy `execution_transactions` for perp dry-run path retained) |
| Test | integration test against Sepolia recent intents; regression test ensures perp intent still returns its row |
| Auditor anchor | operational UX; not a security finding but auditor expects clean operator-side observability |
| Posture | **SHIPPED 2026-06-10**. Handler now joins both tables and projects rows onto a unified `ExecutorTransactionView` shape with a `source` discriminator; orderbook + RFQ visibility both pinned by integration tests (incl. live-smoke regression against the exact tx hash). See `docs/OPTION_EXECUTION_TX_VISIBILITY_FIX_RESULT.md`. Follow-on `BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND` (2026-06-10) extended the LIST variant (`GET /executor/transactions`) with the same unified shape; see `docs/BACKEND_EXECUTOR_TRANSACTIONS_LIST_EXTEND_RESULT.md`. |

---

## 2. Test strategy

### 2.1 Layer matrix

| Layer | Coverage required |
|---|---|
| Unit | Every `should_broadcast` branch (T-10); RemoteSigner adapter request/response shape; startup guards (env-keyed refused on mainnet chain id; signer-endpoint required); monitoring metric emission shape; option-execution route handler |
| Integration | Sepolia end-to-end: intent → simulation → `should_broadcast` decision → KMS-backed sign (via mock or staging signer service) → broadcast → confirmation worker transitions; orderbook + RFQ paths |
| Regression | Existing Sepolia orderbook + RFQ smoke tests continue to pass; first-live-smoke result docs reproducible against new code |
| Fork | Mainnet fork at engagement-kickoff block: deploy + wire + configure + verify + transferOwnership + acceptOwnership; should_broadcast green; signer service deployed; monitoring + alerts fire; Cluster 4 launch invariants verified |
| Staging | Per `STAGING_REHEARSAL.md` — full topology with realistic governance + monitoring + signer service + Treasury Safe + InsuranceFund |
| Mainnet | NO broadcast until §3 gates met |

### 2.2 Sepolia regression suite (existing, MUST continue to pass)

| Test | Result |
|---|---|
| `FIRST_LIVE_SMOKE-EXEC-V2-SEPOLIA-FEE-ONLY` orderbook | tx `0xb2379a46…e800` — must reproduce simulation + broadcast + confirmation + accounting + R5 drift = 0 |
| `FIRST_LIVE_SMOKE-RFQ-CLOSEOUT-VERIFY` RFQ | tx `0x8538066c…5326` — same expectations |
| BadNonce remediation cycle | nonce-sync `(0,0) → (1,1)` chain alignment |
| Confirmation worker auto-transition | `broadcast_submitted → broadcast_confirmed` within 2s |

### 2.3 Acceptance tests for the rebate-DEFER launch invariant (NEW)

```text
[ ] Test 1: with rebateReserve = 0 AND all profiles non-negative,
            should_broadcast returns True for fee-only candidates.
[ ] Test 2: with rebateReserve = 0 AND any profile that produces
            effective negative ppm under RFQ discount, should_broadcast
            returns False with reason = "rebate-reserve" or analog.
[ ] Test 3: launch invariant sweep is exposed as a callable function;
            returns structured report (per-profile flags + global state).
[ ] Test 4: invariant verifier runs at startup and refuses broadcast
            if violated when chain_id = 8453 (mainnet).
```

---

## 3. Local / fork / staging / mainnet gates

| Gate | Conditions to pass |
|---|---|
| **Local** | `cargo test --all-targets --all-features` green; `forge test --no-match-path 'test/fork/*'` green |
| **Sepolia integration** | KMS adapter end-to-end Sepolia broadcast succeeds; monitoring fires synthetic alerts; regression tests pass |
| **Fork** | mainnet fork at engagement-kickoff block: deploy + configure + Treasury Safe deploy simulation + InsuranceFund deploy simulation + V2G-Y phases Y-A → Y-G run in fork; Cluster 4 launch invariant sweep green; should_broadcast accepts fee-only / rejects rebate-positive |
| **Staging** | full topology rehearsal; AUDIT-EXT can verify staging if requested |
| **Mainnet** | §3 hard stops from `PREBUILD_TO_BUILD_HANDOFF.md` met; AUDIT-EXT minimum-pass attestation; 4-sig MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION collected |

---

## 4. Done criteria per track

### 4.1 `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`

```text
[ ] PR merged on main branch
[ ] cargo test green
[ ] Sepolia integration test green
[ ] launch invariant verifier function exposed + tested
[ ] auditor's Q-34 attestation possible at engagement-kickoff commit
[ ] regression: Sepolia first-live-smoke result reproducible
```

### 4.2 `MAINNET-BE-SIGNER-SERVICE-DESIGN`

```text
[ ] design doc shipped
[ ] AUDIT-EXT reviews Pattern C + §6.6 policy
[ ] backend repo + signer-service repo (or sub-crate) responsibilities split documented
[ ] vendor-agnostic at the trait layer
```

### 4.3 `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER`

```text
[ ] PR merged on main branch
[ ] RemoteSigner trait + KmsRemoteSigner impl + startup guards
[ ] Sepolia integration test through new path green
[ ] no mainnet broadcast attempted in test
[ ] auditor's Q-26 + Q-27 attestation possible
```

### 4.4 `BACKEND-MONITORING-ALERTS-WIRING`

```text
[ ] all metrics from §2 of monitoring spec exported
[ ] all alerts wired with synthetic-fired verification
[ ] new Cluster 3 ladder + Cluster 4 alerts live
[ ] RUNBOOK_BACKEND_EXECUTOR.md published
[ ] monitoring closure satisfies minimum-pass §10
```

### 4.5 `OPTION-EXECUTION-TX-VISIBILITY-FIX`

```text
[ ] PR merged on main branch
[ ] route returns option_execution_transactions row for option intents
[ ] perp intent path unchanged (returns []  or existing legacy behaviour)
[ ] regression test green
```

---

## 5. Audit dependencies

| Auditor question | Closes via | Track |
|---|---|---|
| Q-26 (KMS non-extractable) | Pattern C design + adapter impl | 1.2 + 1.3 |
| Q-27 (allowlist correctness) | §6.6 policy in signer service | 1.2 |
| Q-28 (roster disjointness) | Cluster 1 chain-anchored | done |
| Q-29 (BE rotation at-least-one-valid) | rotation runbook | 1.3 |
| Q-30 (sub-1-min freeze) | Sepolia drill | M-1/M-3 (operator) |
| Q-31 (R-6 bright line) | should_broadcast + signer service policy | 1.1 + 1.2 |
| Q-32 (DEPLOYER retirement uniqueness) | V2G-Y POST-Y-G-6 sweep verifier | 1.1 (verifier hook) |
| Q-33 (PFV receiver path) | Q-CD-10 SOP + Timelock-only `withdrawRevenue` | operator |
| **Q-34 (launch invariant)** | `should_broadcast` Cluster 4 verifier sweep | **1.1** |
| Q-35 (future rebate gate) | spec doc + future `MAINNET-REBATE-PROGRAM-DESIGN` | operator |
| Q-36 (InsuranceFund counter independence) | invariant tests | 1.1 + monitoring 1.4 |
| Q-37 (operator escalation prevention) | InsuranceFund deploy + manifest fill | operator |
| Q-38 (policy versioning) | Q-CD-18 SemVer SOP | operator |

---

## 6. Non-implementation work tracked elsewhere

| Item | Doc |
|---|---|
| Treasury Safe creation | `MAINNET-TREASURY-SAFE-CREATION-PACKET` (operator) |
| Insurance operator Safe | `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` (operator) |
| InsuranceFund seeding | `MAINNET-INSURANCE-SEEDING-PARAMETER-FILL` (operator) |
| BE funding fill | `MAINNET-BE-FUNDING-POLICY-PARAMETER-FILL` (operator) |
| PFV withdrawal cadence | `MAINNET-PFV-REVENUE-WITHDRAWAL-SOP` (operator) |
| KMS vendor selection | `MAINNET-KMS-VENDOR-SELECTION` (operator) |
| KMS region finalisation | `MAINNET-KMS-REGION-FINALISATION` (operator) |
| Pre-migration DEPLOYER form | `MAINNET-DEPLOY-CEREMONY-DESIGN` (operator) |
| Policy versioning SOP | `MAINNET-CUSTODY-POLICY-VERSIONING-SOP` (operator) |
| Manifest fill PR | `MAINNET-MANIFEST-FILL-GOV-OPS-TREASURY-SLOTS` (deployment owner) |

These run in parallel with the backend tracks above per
`PREBUILD_TO_BUILD_HANDOFF.md §4` parallel tracks Track 2.

---

## 7. Frontend track (cross-referenced)

`FRONTEND-V2G-W3-SSR-PROXY` closes gap-list J-1 / J-5 / J-6 and V2G-AUDIT0 F-H1 / B-H1. Mainnet broadcast blocked until V2G-W3 lands OR admin surface is operationally inaccessible to non-OIDC paths. Frontend track owned by frontend team; coordinates with backend on admin-route gate.

---

## 8. Cross-links

- `deopt-v2-backend/docs/PREBUILD_TO_BUILD_HANDOFF.md` — parent handoff
- `deopt-v2-backend/docs/NEXT_TASK_BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE.md` — first build prompt
- `deopt-v2-backend/docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — should_broadcast spec
- `deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — monitoring spec
- `deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia signer cutover + §13 mainnet KMS lift
- All Cluster 1/2/3/4 redacted summaries
- `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md`
- `deopt-v2-sol/docs/MAINNET_AUDIT_HANDOFF_INDEX.md`
- `~/DEOPT/RUN_STATE.md`

**End of backend mainnet implementation roadmap.**
