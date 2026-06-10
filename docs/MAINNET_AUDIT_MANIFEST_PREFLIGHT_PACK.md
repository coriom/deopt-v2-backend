# Mainnet audit / manifest / preflight pack

**Posture:** DOC ONLY. READ-ONLY audit / consolidation. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet
contract deployment. No AWS resource creation. No KMS key created. No
secrets printed. No guessed mainnet addresses.**

**Date (UTC):** 2026-06-10.

**Closes milestone:** `MAINNET-AUDIT-MANIFEST-PREFLIGHT-PACK`.

**Companion docs (all in `deopt-v2-backend/docs/`):**
- `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — every missing mainnet
  value with status / owner / safe source.
- `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — read-only verification
  commands; no transaction; no mutation.
- `MAINNET_GO_NO_GO_CRITERIA.md` — strict launch criteria.
- `MAINNET_NEXT_SAFE_MILESTONES.md` — 10-milestone DAG.
- `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` — copy/paste prompt for
  the next preflight milestone.

## 0. Hard rules (this pack)

```text
no mainnet tx                                ✅
no Sepolia tx                                ✅
no Safe tx                                   ✅
no broadcast                                 ✅
no AWS resource creation                     ✅
no KMS key creation                          ✅
no .env edit                                 ✅
no real AWS account ID in tracked doc        ✅
no real KMS key id / ARN in tracked doc      ✅
no production signer address in tracked doc  ✅
no guessed mainnet contract addresses        ✅
no private custody roster                    ✅
no source code modification                  ✅
```

## 1. Inputs (Phase A inventory)

Existing readiness docs surveyed.

### 1.1 PRESENT — backend

| Doc | Anchor |
|---|---|
| `RUN_STATE.md` | running history of every shipped milestone; closure paragraphs per milestone. |
| `docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md` | full gap catalogue across the 3 repos; classified P0/P1/P2/P3. |
| `docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md` | 3 P0 blockers + dependency graph + next-3 executable milestones. |
| `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` | preferred AWS KMS path; operator commercial sign-off OPEN. |
| `docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` | 18-criterion × 10-category matrix; recommended shortlist. |
| `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` | adapter contract + 16 named tests. |
| `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` | 7-phase rehearsal ladder. |
| `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` | rotation + incident + retention. |
| `docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` | Pattern C topology. |
| `docs/MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` | Phase 2 prompt (Variant A no-broadcast + Variant B Sepolia canary OPT-IN). |
| `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` + 4 siblings (IAM/key policy template / runtime config / CloudTrail runbook / setup validation checklist) | operator setup pack for AWS KMS. |
| `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` + `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` | feature-gated SDK transport + RequestId extraction. |
| `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md` + pluggable + vendor-specific next-task | AWS KMS backend adapter. |
| `docs/MAINNET_CUSTODY_CLUSTER_{1,2,3,4}_RESOLUTION_REDACTED.md` + next-actions | custody decisions closed at policy layer. |
| `docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` | Q-CD-* dependency graph. |
| `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` + addenda | `/executor/health/v2` schema; `not_tracked_yet=[]`. |
| `docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` + wiring result | metric inventory + alert taxonomy. |
| `docs/BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md` | track summaries + acceptance criteria. |
| `docs/V2_FEE_BACKEND_EXECUTOR_READINESS_V2G_M.md` | fee backend readiness anchor. |
| `docs/PROTOCOL_FEE_VAULT_OBSERVABILITY_V2G_R5_OBS_P0.md` | R5 / PFV observability. |
| `docs/OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md` + `OPTION_RFQ_LIVE_READINESS_V2G_PX.md` | option execution readiness anchors. |
| `docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md` | 0 Critical / 4 High (mainnet-blocking) internal audit summary. |
| 14+ other milestone-specific result docs | observability / health endpoint singletons / live-provider PFV / vault config / etc. |

### 1.2 PRESENT — sol

| Doc | Anchor |
|---|---|
| `docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` | external audit scope + minimum-pass condition. |
| `docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md` | kickoff handoff bundle. |
| `docs/MAINNET_AUDIT_HANDOFF_INDEX.md` | full audit handoff index. |
| `docs/MAINNET_MANIFEST_TODO_INVENTORY.md` | 369-line manifest template; 76 distinct slots; per-slot owner / dependency / blocker class. |
| `docs/MAINNET_MANIFEST_DEPENDENCY_SNAPSHOT_AFTER_CUSTODY_CLUSTERS.md` | per-slot blocker state after custody clusters. |
| `docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` | governance migration plan. |
| `docs/GOVERNANCE_*_V2G_GOV_F_*.md` (4 files) | executor migration prep / queue / execute / bind. |
| `docs/GOVERNANCE_GUARDIAN_MIGRATION_V2G_GOV_A*.md` (2 files) | guardian migration. |
| `docs/INTERNAL_AUDIT_CHECKLIST_V2G_AUDIT.md` | internal audit checklist. |
| `docs/INVARIANT_FUZZ_COVERAGE_MATRIX_V2G_AUDIT.md` | fuzz coverage matrix. |
| `docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md` | mirror of backend findings. |
| `docs/PERP_ENGINE_CODE_SIZE_AND_MIGRATION_STRATEGY_V2F_C.md` | PERP scaffold migration. |
| `deployments/mainnet.template.json` | the canonical manifest with `TODO_REPLACE_*` placeholders. |

### 1.3 PRESENT — frontend

| Doc | Anchor |
|---|---|
| `docs/ADMIN_AUTH_RBAC_UI_NOTES_V2G_V.md` | admin auth RBAC UI. |
| `docs/ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md` | admin proxy (V2G-W2 closed; V2G-W3 SSR proxy pending). |
| `docs/ADMIN_OPTION_LIFECYCLE_VIEW_V2A.md` | admin option lifecycle. |
| `docs/ADMIN_V2_FEE_OBSERVABILITY_UI_V2G_U.md` + `V2E_H.md` | admin fee observability UI. |
| `docs/INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md` | mirror. |

### 1.4 MISSING from the brief's expected inventory

* `deopt-v2-backend/docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — **PRESENT** (just shipped this week).
* `deopt-v2-backend/docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — **PRESENT**.
* `deopt-v2-backend/docs/MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` — **PRESENT**.

No expected docs reported missing.

### 1.5 NOT YET PRESENT (these new files this milestone creates)

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — THIS DOC.
* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md`.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md`.
* `MAINNET_GO_NO_GO_CRITERIA.md`.
* `MAINNET_NEXT_SAFE_MILESTONES.md`.
* `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md`.

## 2. Mainnet manifest inventory — 8 categories

Each row uses a status code: **READY** (no remaining work) /
**PARTIAL** (some work shipped; some operator-side gaps) /
**OPEN** (work still required) / **OPERATOR_ONLY** (out of repo
scope; operator-side action).

### 2.1 Chain / network

| Field | Status | Notes |
|---|---|---|
| Base mainnet `chain_id = 8453` | READY | Anchored in code via `MAINNET_CHAIN_ID` const; refused-when-mismatched in `validate_signer_backend` + `LocalDevSigner` runtime guard. |
| Mainnet RPC provider | OPERATOR_ONLY | Operator selects + provisions; recorded in operator secret store, never tracked. |
| Block explorer | READY | `https://basescan.org` (Base mainnet) — public. |
| Deployment block ranges | OPEN | Set after `MAINNET-DEPLOYMENT` event-indexer `start_block` is configured per deployed contract. |

### 2.2 Custody

| Field | Status | Notes |
|---|---|---|
| `OPS_SAFE_MAINNET` = `0xce0e46Db1072B820CB5eCf30188ED76cb560C932` | READY | Per `MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`; chain-verified. Threshold 2/3. |
| `GOV_SAFE_MAINNET` = `0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166` | READY | Per same. Threshold 3/5. |
| DEPLOYER provenance | OPERATOR_ONLY | Mainnet DEPLOYER attestation Q-CD-8 still OPEN (per Cluster 1 result); Sepolia DEPLOYER probed false on both Safes. |
| OPS / GOV owner overlap | READY | 0 overlap chain-verified. |
| Treasury Safe | OPEN | Tracked under `MAINNET-TREASURY-SAFE-CREATION-PACKET`. |
| Insurance Fund operator policy | OPEN | Tracked under `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET`. |
| Timelock | OPEN | Mainnet Timelock instance not yet deployed; configured under `MAINNET-V2G-Y-OWNERSHIP-MIGRATION`. |
| Production signer address | OPERATOR_ONLY | Derived from AWS KMS GetPublicKey AFTER the operator provisions the mainnet KMS key. Never guessed; never committed to tracked docs. |

### 2.3 Contracts

| Field | Status | Notes |
|---|---|---|
| `OptionMatchingEngine` (OME) | OPEN | Mainnet deployment + `setExecutor(BE_address)` Safe-tx packet pending; sol scaffold complete. |
| `ProtocolFeeVault` (PFV) | OPEN | Mainnet deployment pending; sol scaffold complete. |
| `FeesManagerV2` | OPEN | Mainnet deployment pending; sol scaffold complete. |
| `CollateralVault` (CV) | OPEN | Same. |
| `RiskGuardian` (RG) | OPEN | Same. |
| Tokens (USDC etc.) | READY | Pre-existing on Base mainnet; addresses recorded in `mainnet.template.json` placeholders. |
| Oracles | READY | Chainlink (or operator-selected) addresses — already public; recorded in manifest. |
| Perps components | NOT_APPLICABLE_AT_LAUNCH | Per Q-CD-6, PERP backend executor deferred; perp scaffold mainnet activation tracked separately. |

### 2.4 Backend

| Field | Status | Notes |
|---|---|---|
| Signer mode | READY | `BACKEND_SIGNER_MODE=remote` required on mainnet; LocalDev refused. |
| Remote signer / AWS KMS adapter | READY | `AwsKmsSignerProvider` + `AwsKmsSdkTransport` (feature-gated) + CloudTrail RequestId extraction. 1053 backend tests green. |
| `should_broadcast` policy gate | READY | §8 economic gate + §6 chain-state precheck + 3-source dedupe; pinned by `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`. |
| Executor health endpoint | READY | `/executor/health/v2` schema complete; `not_tracked_yet=[]`. |
| `/metrics` Prometheus surface | READY | Per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1_WIRING_RESULT.md`. |
| Option nonce sync | READY | `OPTION_NONCE_SYNC_ENABLED` proven on Sepolia smoke; mainnet-ready. |
| Broadcast gas limits | READY | `EXECUTOR_MAX_GAS_LIMIT` + `EXECUTOR_MAX_FEE_PER_GAS_WEI` + `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` typed-config. |
| Confirmation worker | READY | Option confirmation worker proven on Sepolia smoke. |
| Event indexer | READY | Per-table; addresses configured via typed config (PFV / FM_V2 / CV). |
| `EXECUTOR_FROM_ADDRESS` mainnet value | OPERATOR_ONLY | Derived after KMS provision; never committed; cross-checked at startup. |

### 2.5 Frontend / admin

| Field | Status | Notes |
|---|---|---|
| Admin dashboard | READY | V2G-V + V2G-W2 closed; admin RBAC + auth proxy live. |
| Tx visibility endpoints | READY | `/executor/transactions/:intent_id` + list endpoint both return unified PERP + OPTION rows. |
| Lifecycle view | READY | `/admin/options/executions/:intent_id/lifecycle`. |
| Health / status endpoints | READY | `/executor/health/v2` consumed by frontend admin status banner. |
| SSR / proxy hardening | OPEN | `FRONTEND-V2G-W3-SSR-PROXY` pending. |
| Frontend mainnet env values | OPERATOR_ONLY | Read from operator secret store at deploy time. |

### 2.6 Monitoring

| Field | Status | Notes |
|---|---|---|
| Prometheus metrics | READY | Full signer + broadcast + chain-state taxonomy; bounded labels. |
| Backend alerting (PagerDuty / Discord) | OPERATOR_ONLY | Routes operator-side; PromQL rules pinned in `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 + §4`. |
| CloudTrail trail | OPERATOR_ONLY | Setup spec in `AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md`. |
| Signer RequestId correlation | READY | Real CloudTrail RequestId extracted with 5-step sanitiser + synthetic fallback. |
| RPC / provider failure metrics | READY | `policy_data_failures_total{read_type}` + `fm_v2_*_failures_total` + `last_policy_data_failure_type`. |

### 2.7 Rehearsal

| Field | Status | Notes |
|---|---|---|
| Phase 1 mock remote signer | READY | 22 unit tests pin every mock mode. |
| Phase 2 sandbox AWS KMS | OPEN | Operator-side; `MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` Variant A ready. |
| Phase 3 Sepolia remote-signer rehearsal | OPEN | Gated on Phase 2. Variant B in the same next-task prompt; OPT-IN. |
| Phase 4 no-broadcast mainnet dry run | OPEN | Gated on Phase 3. |
| Phase 5 read-only mainnet preflight | OPEN | Lives in `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` (this milestone). |
| Phase 6 final Sepolia canary against production commit | OPEN | Gated on Phases 4-5. |
| Phase 7 mainnet canary PREPARATION | OPEN | Gated on Phase 6. |
| Mainnet broadcast | NOT_AUTHORISED | Out of scope until rehearsal phases all GO. |

### 2.8 Audit

| Field | Status | Notes |
|---|---|---|
| External audit engagement | OPERATOR_ONLY | Engagement package in `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`; operator dispatches. |
| Auditor handoff bundle | READY | `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md` + `MAINNET_AUDIT_HANDOFF_INDEX.md`. |
| Internal audit findings | READY | `INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md` — 0 Critical / 4 High (all mainnet-blocking). |
| Known risk acceptances | READY | Documented per finding. |
| Operator commercial sign-off (Q-CD-5 vendor) | OPERATOR_ONLY | Technical recommendation captured; commercial closure offline. |

## 3. Current mainnet readiness gap

* **Backend layer:** feature-complete. 1053 tests. Production
  `RemoteSignerClient::new` continues to use `UnimplementedTransport`
  (fail-closed) until the rehearsal Phase 3 cutover.
* **Sol layer:** scaffold complete + audit package ready.
  `mainnet.template.json` has 76 placeholder slots tracked under
  `MAINNET_MANIFEST_TODO_INVENTORY.md` + dependency snapshot.
* **Frontend layer:** admin surfaces ready. SSR proxy pending under
  `FRONTEND-V2G-W3-SSR-PROXY`.
* **Operator layer:** AWS KMS resources not yet provisioned; external
  audit engagement not yet dispatched (or status not visible to this
  pack); Treasury Safe + Insurance Fund operator policy pending;
  mainnet `setExecutor` Safe-tx packet pending.

See `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` for the per-value
status table.

## 4. Pack cross-links

* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — every gap, named.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — verification commands.
* `MAINNET_GO_NO_GO_CRITERIA.md` — launch criteria.
* `MAINNET_NEXT_SAFE_MILESTONES.md` — milestone DAG.
* `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` — next-task prompt.
* `deopt-v2-backend/docs/MAINNET_READINESS_GAP_LIST_AFTER_SEPOLIA_ARC.md`
  — full gap catalogue.
* `deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md` — 3
  P0 blockers.
* `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` — manifest
  template TODOs.
* `deopt-v2-sol/docs/MAINNET_MANIFEST_DEPENDENCY_SNAPSHOT_AFTER_CUSTODY_CLUSTERS.md`
  — manifest dependency snapshot.
* `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` —
  external audit scope.
* `deopt-v2-backend/docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` —
  Q-CD-5 technical recommendation closure.
* `deopt-v2-backend/docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — operator
  setup pack.
* `deopt-v2-backend/docs/MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md`
  — Phase 2 prompt.
* `deopt-v2-backend/docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`
  — 7-phase ladder.
* `deopt-v2-backend/docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` —
  health endpoint schema.

## 5. Sepolia anchor addresses (reference)

For comparison only. Mainnet equivalents are still placeholders
per Cluster 2 §2.2 / `MAINNET_MANIFEST_TODO_INVENTORY.md`.

| Slot | Sepolia value |
|---|---|
| `BUYER` | `0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC` |
| `SELLER` | `0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88` |
| `BACKEND_EXECUTOR` | `0x295005fd4F311e6691F008D57d32FCFEde844518` |
| `DEPLOYER` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `OPS_MULTISIG (sepolia)` | `0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46` |
| `TIMELOCK` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` |
| `NEW_OME` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` |
| `PFV` | `0x7C0a3B6feBd5BFFc164f37738299AeB453181886` |
| `NEW_FM_V2` | `0xF6626177f3B85cc3239667Cc53C04A8007652944` |
| `CV` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `RG` | `0x7918Ea95c2791B6b587fF02AE481FA52403877A0` |
| `mUSDC` | `0x6eAe407f5640B006faC9965182e238582A3B412E` |

Mainnet equivalents: **NOT YET KNOWN — never guess. See
`MAINNET_MANIFEST_MISSING_VALUES_TABLE.md`.**
