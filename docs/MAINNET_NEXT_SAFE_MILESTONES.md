# Mainnet next safe milestones

**Posture:** DOC ONLY. **No chain mutation, no `.env` edit, no
Safe-tx, no broadcast, no mainnet activation, no AWS resource
creation by THIS doc — it ENUMERATES the milestone sequence each of
which is separately authorised.**

**Companion:** `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`.

## 0. Hard rules (this doc)

```text
no mainnet tx                      ✅
no Sepolia tx                      ✅
no Safe tx                         ✅
no broadcast                       ✅
no AWS resource creation           ✅
no .env edit                       ✅
no secret printed                  ✅
no guessed address                 ✅
```

## 1. Milestone DAG

```
                            ┌─────────────────────────────────────────┐
                            │ MAINNET-AUDIT-MANIFEST-PREFLIGHT-PACK    │  (this milestone)
                            └────────────────────┬────────────────────┘
                                                 │
              ┌──────────────────────────────────┼──────────────────────────────────┐
              │                                  │                                  │
   ┌──────────▼──────────┐         ┌─────────────▼─────────────┐        ┌───────────▼────────────┐
   │ AWS-KMS-OPERATOR-   │         │ MAINNET-AUDIT-EXT-KICKOFF │        │ MAINNET-V2G-Y-OWNERSHIP-│
   │ SETUP-EXECUTION     │         │ (if not dispatched)       │        │ MIGRATION (deploy timelock│
   │ (operator-side AWS  │         └─────────────┬─────────────┘        │ + transfer-ownership pkts)│
   │  account + IAM +    │                       │                      └───────────┬────────────┘
   │  KMS key + CloudTrail)                      │                                  │
   └──────────┬──────────┘                       │                                  │
              │                                  │                                  │
   ┌──────────▼──────────┐                       │                                  │
   │ MAINNET-SIGNER-     │                       │                                  │
   │ REHEARSAL-PHASE-2-  │                       │                                  │
   │ EXECUTION Variant A │                       │                                  │
   │ (no-broadcast AWS   │                       │                                  │
   │  KMS rehearsal)     │                       │                                  │
   └──────────┬──────────┘                       │                                  │
              │                                  │                                  │
              │                                  │           ┌──────────────────────▼────────┐
              │                                  │           │ MAINNET-DEPLOYMENT-MANIFEST-FILL│
              │                                  │           │ (fill mainnet.template.json)   │
              │                                  │           └──────────────┬────────────────┘
              │                                  │                          │
              │           ┌──────────────────────▼──────────────────────────▼──┐
              │           │ MAINNET-TREASURY-SAFE-CREATION-PACKET (if needed)  │
              │           │ MAINNET-INSURANCE-OPERATOR-POLICY-PACKET (if needed)│
              │           └──────────────────────┬──────────────────────────────┘
              │                                  │
   ┌──────────▼──────────────────────────────────▼──────────┐
   │ MAINNET-READ-ONLY-PREFLIGHT                            │
   │ (run MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md against  │
   │  real mainnet — read-only commands only)               │
   └──────────────────────────┬──────────────────────────────┘
                              │
   ┌──────────────────────────▼──────────────────────────────┐
   │ FRONTEND-V2G-W3-SSR-PROXY                              │
   │ (admin hardening if not already done)                  │
   └──────────────────────────┬──────────────────────────────┘
                              │
   ┌──────────────────────────▼──────────────────────────────┐
   │ MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION Variant B   │
   │ (Sepolia signer canary — OPT-IN; later authorised)     │
   └──────────────────────────┬──────────────────────────────┘
                              │
   ┌──────────────────────────▼──────────────────────────────┐
   │ MAINNET-CANARY-PLAN                                    │
   │ (preparation per MAINNET_SIGNER_STAGING_REHEARSAL_PLAN §7)│
   └──────────────────────────┬──────────────────────────────┘
                              │
                              ▼
   ┌─────────────────────────────────────────────────────────┐
   │ MAINNET-CANARY-BROADCAST                                │
   │ (LAST; explicit operator authorisation required;        │
   │  separately runbook'd)                                  │
   └─────────────────────────────────────────────────────────┘
```

## 2. Milestone table

For each milestone: repo, owner, prerequisites, forbidden actions,
outputs, validation.

### M1. MAINNET-AUDIT-MANIFEST-PREFLIGHT-PACK (current)

| Field | Value |
|---|---|
| Repo | deopt-v2-backend |
| Owner | Backend + Security |
| Prerequisites | None. |
| Forbidden | no chain tx; no Safe tx; no AWS resource creation; no `.env` edit; no source code change. |
| Outputs | `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`, `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md`, `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md`, `MAINNET_GO_NO_GO_CRITERIA.md`, `MAINNET_NEXT_SAFE_MILESTONES.md`, `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` + RUN_STATE closure. |
| Validation | `git diff --check` clean. No source code modified. |

### M2. AWS-KMS-OPERATOR-SETUP-EXECUTION

| Field | Value |
|---|---|
| Repo | Operator side (NOT in this repo). |
| Owner | Operator + Security + Backend ops. |
| Prerequisites | M1 closed; `AWS_KMS_OPERATOR_SETUP_PACK.md` reviewed; operator commercial sign-off on Q-CD-5. |
| Forbidden | no `EXECUTOR_PRIVATE_KEY` ever; no `LocalDev` mode mainnet; no `Mock` mainnet; no AWS long-lived creds in production runtime. |
| Outputs | Real AWS account / IAM role / KMS key / CloudTrail trail provisioned per the templates. Operator binder updated. Public-safe `MAINNET_AWS_KMS_SETUP_RESULT.md` closure note (no real IDs / ARNs). |
| Validation | `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md §1-§5` all GREEN. |

### M3. MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION (Variant A — no-broadcast)

| Field | Value |
|---|---|
| Repo | Operator side. |
| Owner | Operator + Backend ops. |
| Prerequisites | M2 closed. |
| Forbidden | no chain tx; no broadcast on any network; no Safe tx; no `.env` edit in repo; no production address in tracked logs. |
| Outputs | Per `MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md §1` — preflight + GetPublicKey + Sign test prehash + offline DER decode + recovery. Public-safe `MAINNET_SIGNER_REHEARSAL_PHASE_2_RESULT.md` closure. |
| Validation | `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md §6 R1-R14` all GREEN. CloudTrail correlation verified. |

### M4. MAINNET-AUDIT-EXT-KICKOFF (parallel; M1 unblocks)

| Field | Value |
|---|---|
| Repo | deopt-v2-sol (engagement package owner). |
| Owner | Operator + Security + Sol. |
| Prerequisites | M1 closed; auditor selection captured offline. |
| Forbidden | no chain mutation; no `.env` edit; no premature finding disclosure. |
| Outputs | External auditor engaged; auditor working from `MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md`; findings captured per `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`. |
| Validation | Auditor confirmation of receipt; engagement timeline locked. |

### M5. MAINNET-V2G-Y-OWNERSHIP-MIGRATION (parallel; M1 unblocks)

| Field | Value |
|---|---|
| Repo | deopt-v2-sol. |
| Owner | Sol + Custody. |
| Prerequisites | M1 closed; mainnet OPS / GOV Safes already deployed (Cluster 1 result); commercial / legal sign-off captured. |
| Forbidden | no chain tx in THIS milestone — only deployment scripts + manifest prep. Actual deployment is a separate authorised operation. |
| Outputs | Deployment scripts ready; manifest filled with Timelock placeholder + role-binding plan + transfer-ownership Safe-tx packets prepared (read-only). |
| Validation | Per `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`. |

### M6. MAINNET-TREASURY-SAFE-CREATION-PACKET (if needed)

| Field | Value |
|---|---|
| Repo | Custody / Operator. |
| Owner | Custody + Operator. |
| Prerequisites | M1 closed; operator policy decides whether Treasury Safe is required at launch. |
| Forbidden | no chain tx by THIS milestone — packet preparation only. |
| Outputs | Public-safe Treasury Safe creation packet (address recorded after operator-authorised Safe creation). |
| Validation | Operator binder updated. |

### M7. MAINNET-INSURANCE-OPERATOR-POLICY-PACKET (if needed)

| Field | Value |
|---|---|
| Repo | Operator policy doc. |
| Owner | Operator + Security. |
| Prerequisites | M1 closed; Cluster 3 Insurance Fund policy reviewed. |
| Forbidden | no chain tx; no policy enactment by THIS doc. |
| Outputs | Public-safe operator policy doc; private roster in offline binder. |
| Validation | Operator confirms publication path. |

### M8. MAINNET-DEPLOYMENT-MANIFEST-FILL

| Field | Value |
|---|---|
| Repo | deopt-v2-sol. |
| Owner | Sol + Operator. |
| Prerequisites | M4 closed (audit findings resolved); M5 closed (Timelock + ownership migration ready); M6 / M7 closed if applicable. |
| Forbidden | no mainnet deployment in THIS milestone — only manifest fill from operator binder values. |
| Outputs | `deopt-v2-sol/deployments/mainnet.template.json` → `mainnet.json` with all 76 slots resolved. |
| Validation | Per `MAINNET_MANIFEST_TODO_INVENTORY.md` per-slot verifier. |

### M9. MAINNET-READ-ONLY-PREFLIGHT

| Field | Value |
|---|---|
| Repo | All three (verification step). |
| Owner | Operator + Backend ops + Security. |
| Prerequisites | M2-M8 closed; mainnet contracts deployed via the deployment scripts in a SEPARATELY-runbook'd operation. |
| Forbidden | no chain tx; no Safe tx; no mutation; READ-ONLY only. |
| Outputs | `MAINNET_READ_ONLY_PREFLIGHT_RESULT.md` capturing the §1-§7 checklist results. |
| Validation | All checks GREEN per `MAINNET_GO_NO_GO_CRITERIA.md §2`. |

### M10. FRONTEND-V2G-W3-SSR-PROXY (parallel)

| Field | Value |
|---|---|
| Repo | deopt-v2-frontend. |
| Owner | Frontend + Security. |
| Prerequisites | M1 closed. |
| Forbidden | no production deployment; no admin token output. |
| Outputs | SSR proxy implementation + tests + closure doc. |
| Validation | Per `ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md` next-iteration spec. |

### M11. MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION (Variant B — Sepolia canary; OPT-IN)

| Field | Value |
|---|---|
| Repo | Operator side. |
| Owner | Operator + Backend ops + Security. |
| Prerequisites | M3 (Variant A) closed; M9 closed; OPS/GOV/Treasury custody confirmed; explicit authorisation captured per `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`. |
| Forbidden | no mainnet broadcast; no more than ONE Sepolia broadcast; no Safe tx; no `.env` edit in repo. |
| Outputs | Per `MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md §2` — single Sepolia tx hash; CloudTrail correlation; backend `/executor/transactions/<intent_id>` returns row. Public-safe `MAINNET_SIGNER_REHEARSAL_PHASE_3_SEPOLIA_CANARY_RESULT.md` closure (Phase 3 in staging plan terms). |
| Validation | Per Variant B end-to-end. |

### M12. MAINNET-CANARY-PLAN

| Field | Value |
|---|---|
| Repo | Operator runbook. |
| Owner | Operator + Backend ops + Security + Custody. |
| Prerequisites | M11 closed; M9 GREEN; all GO criteria from `MAINNET_GO_NO_GO_CRITERIA.md §2` met. |
| Forbidden | no mainnet tx in THIS milestone — preparation only. |
| Outputs | Mainnet canary plan doc capturing: `setExecutor` Safe-tx packet (read-only), gas funding plan, OPS Safe signer scheduling, rollback plan, monitoring channel staffing. |
| Validation | Per `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §7 Phase 7 acceptance`. |

### M13. MAINNET-CANARY-BROADCAST (LAST; OPT-IN; separately authorised)

| Field | Value |
|---|---|
| Repo | Operator runbook execution. |
| Owner | Operator + Backend ops + Security + Custody. |
| Prerequisites | M12 closed; ALL GREEN per `MAINNET_GO_NO_GO_CRITERIA.md §2`; explicit operator + Security + Custody authorisation captured per separately-runbook'd protocol. |
| Forbidden | no second canary in this milestone; if rollback triggered, halt; no `.env` edit in repo. |
| Outputs | Mainnet tx hash recorded publicly; `/executor/transactions/<intent_id>` returns row; confirmation worker mined_success observed. Public-safe `MAINNET_CANARY_BROADCAST_RESULT.md` closure. |
| Validation | All GO criteria GREEN before; no rollback trigger fires; mined_success observed within window. |

## 3. Parallelism

| Track | Can run in parallel with |
|---|---|
| M2 (AWS setup) | M4 (audit kickoff), M5 (governance migration prep), M6/M7 (custody packets), M10 (frontend) |
| M3 (Phase 2 Variant A) | M4, M5, M6, M7, M8, M10 |
| M4 (audit kickoff) | every other milestone |
| M9 (read-only preflight) | requires M2-M8 complete; no parallel work other than M10 |
| M11 (Phase 2 Variant B) | requires M3, M9 complete; serial after that |
| M12, M13 (canary plan + broadcast) | serial; final two |

## 4. Critical path

The shortest path from current state to mainnet canary broadcast:

1. **M2** AWS setup (operator-side; 1-2 weeks).
2. **M3** Phase 2 Variant A no-broadcast rehearsal (1-2 days).
3. **M4** Audit kickoff (external auditor; 4-8 weeks engagement; runs in parallel to M2 / M3).
4. **M5** Governance migration ready (1 week; runs in parallel to M2 / M3 / M4).
5. **M6 / M7** Custody packets if needed (1 week each).
6. **M8** Manifest fill (after M4 audit findings resolved; 1 day).
7. **M9** Read-only preflight (after mainnet contracts deployed in separately-runbook'd operation; 1 day).
8. **M10** Frontend SSR proxy (1-2 weeks; can run in parallel to M2-M5).
9. **M11** Phase 2 Variant B Sepolia canary (1-2 days).
10. **M12** Canary plan (1 week).
11. **M13** Mainnet canary broadcast (separately authorised; 1 day operation window).

External audit (M4) is the **dominant critical-path item** (4-8
weeks). Every other operator-side track can complete within 1-2
weeks each but cannot reach M9 until audit findings are resolved.

## 5. Risk + dependency callouts

| Risk | Mitigation |
|---|---|
| External auditor turnaround delay | Operator dispatches M4 immediately; runs everything else in parallel; audit becomes blocking AT M8 not before. |
| Operator commercial Q-CD-5 sign-off delay | Backend implementation track is already complete; M2-M3 can begin once sign-off lands. |
| KMS key region availability | EU regions enumerated in `MAINNET_KMS_VENDOR_SELECTION_DECISION.md §3.1`; operator picks one. |
| Treasury Safe creation order | If launch-day Treasury Safe NOT REQUIRED, M6 deferred to post-launch. |
| Frontend SSR proxy delay | M10 can complete in parallel; not on critical path for mainnet broadcast. |
| Internal audit High findings not resolved | Hardened in M9 acceptance criteria; cannot reach M13 until resolved. |

## 6. Cross-links

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — missing values.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — verification commands.
* `MAINNET_GO_NO_GO_CRITERIA.md` — GO criteria for each phase.
* `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` — next-task prompt for M9.
* `AWS_KMS_OPERATOR_SETUP_PACK.md` — operator setup pack for M2.
* `MAINNET_SIGNER_REHEARSAL_PHASE_2_NEXT_TASK.md` — Phase 2 prompt
  for M3 + M11.
* `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` (in deopt-v2-sol) — audit
  package for M4.
* `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` (in deopt-v2-sol) —
  governance migration for M5.
* `MAINNET_MANIFEST_TODO_INVENTORY.md` (in deopt-v2-sol) — manifest
  TODOs for M8.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — phase ladder anchors
  for M3 / M11 / M12 / M13.
