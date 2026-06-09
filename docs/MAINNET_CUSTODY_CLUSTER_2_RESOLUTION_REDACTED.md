# Mainnet custody — Cluster 2 resolution (REDACTED public summary)

**Posture:** READ-ONLY redacted public summary. **No chain mutation.
No `.env` edit. No Safe-tx. No broadcast. No mainnet broadcast.
No KMS key created. No IAM role provisioned. No vendor account
created.** Public companion to the private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION.private.md`
(mode 600, outside all repo trees).

**Date validated (UTC):** 2026-06-09

This document contains **NO** vendor credentials, API keys, KMS
account IDs, IAM ARNs, network FQDNs, IP ranges, VPC IDs, mainnet
BACKEND_EXECUTOR EOA addresses, private keys, seed phrases, mnemonics,
recovery phrases, RPC secrets, admin tokens, or DATABASE_URL values.
Only architectural pattern labels, policy descriptions, status
classifications, and the sha256 anchor for the private artefact.

---

## 0. Cluster 2 closure status (architecture + policy decided; vendor + region detail pending)

| Q-CD | Status | Decision label |
|---|---|---|
| **Q-CD-5** KMS / HSM architecture pattern | **ARCH-PATTERN-DECIDED** | **Pattern C** — dedicated backend signer microservice backed by HSM/MPC or KMS, with strict signing policy layer. Vendor choice (specific KMS/HSM/MPC provider behind the signer service) is **PROVIDER-CHOICE-PENDING**. |
| **Q-CD-6** Option vs Perp BE topology | **OPERATOR-DECIDED** | **Distinct EOAs.** OPTION_BACKEND_EXECUTOR mainnet provisioned at launch (KMS-backed via Pattern C). PERP_BACKEND_EXECUTOR mainnet **provisioning deferred** until perp scaffold real-broadcast path is implemented in source. |
| **Q-CD-14** KMS / HSM region pair | **RECOMMENDED-PENDING-PROVIDER** | Primary **EU** region + secondary **EU / nearby compatible** region. Exact regions follow vendor selection. |
| **Q-CD-15** Key-deletion approval lock | **POLICY-DECIDED-PROVIDER-DETAIL-PENDING** | Disable ≠ delete. Permanent deletion requires ≥ 2 independent operator approvals + governance sign-off + ≥ 7-day waiting period if vendor-supported. Emergency IAM revoke is faster than delete. |

---

## 1. Q-CD-5 — architecture pattern (Pattern C)

### 1.1 Patterns compared

| Pattern | Outcome |
|---|---|
| A — Cloud KMS direct from backend | ACCEPTABLE per custody policy §7.1; **superseded by Pattern C for stronger trust-boundary** |
| B — HSM / MPC vendor direct from backend | ACCEPTABLE per custody policy §7.1; same trust-boundary trade-off as A |
| **C — Dedicated backend signer microservice backed by HSM/MPC or KMS, with strict signing policy layer** | **RECOMMENDED DEFAULT** |
| D — Raw `EXECUTOR_PRIVATE_KEY` env (Sepolia rehearsal shape) | **UNACCEPTABLE for mainnet** per custody policy §7.2 (W-5 from gap list); explicit auto-fail |

### 1.2 Why Pattern C

- Backend app **never** sees the private key — even in memory; even momentarily.
- Signer service is the **single chokepoint** that enforces the §6.6 transaction policy (chainId allowlist `{8453}`, target allowlist `{NEW_OME_MAINNET}`, selector allowlist `{executeTrade, executeRfqTrade}`, max value `0`, max gas, nonce match, rate limit, post-sign from-address verification).
- Two-layer audit trail: signer service per-sign log + downstream KMS/HSM provider log.
- **Vendor swap is easier**: signer service abstracts the underlying KMS/HSM/MPC vendor; the backend never knows which is underneath.
- **Defence-in-depth** (custody policy P-11): backend compromise alone is insufficient to mis-sign — attacker must also defeat the signer-service policy layer.
- **Audit clarity**: AUDIT-EXT can review one signer-service surface with clear authn (mTLS) and a finite policy enum, rather than reviewing diverse backend-side KMS calls.

**Trade-off accepted:** higher operational complexity (extra microservice) + small two-hop latency cost. For an OME path signing at most tens of tx per minute, two-hop latency is well inside the existing dedupe + simulation budget.

### 1.3 Implementation impact (high-level)

- New `RemoteSigner` trait or equivalent indirection in `src/execution/signer.rs`. Existing `ExecutorSigner::from_private_key` retained for Sepolia / tests only. New `KmsRemoteSigner::from_service_endpoint(endpoint)` impl for mainnet.
- New env keys: `BACKEND_SIGNER_ENDPOINT`, plus mTLS cert paths. **`EXECUTOR_PRIVATE_KEY` env REFUSED on mainnet** (`chain_id = 8453`) at startup.
- Separate signer-microservice codebase / crate (new repo or new sub-crate). Responsibilities: mTLS server, §6.6 policy layer, vendor adapter, structured per-sign log, failover client, emergency disable endpoint, health endpoint.

### 1.4 Sub-decisions deferred (NOT in this resolution)

| Sub-decision | Owner | When resolved |
|---|---|---|
| Specific KMS / HSM / MPC vendor (cloud KMS vendor / HSM vendor / MPC provider name) | Operator + Security + Backend | `MAINNET-KMS-VENDOR-SELECTION` (separate milestone; recorded in offline binder) |
| Signer-service language / framework | Backend + Security | `MAINNET-BE-SIGNER-SERVICE-DESIGN` |
| Signer-service transport (mTLS HTTPS vs mTLS gRPC) | Backend | `MAINNET-BE-SIGNER-SERVICE-DESIGN` |
| Exact KMS region pair | Operator + DevOps | follows vendor — `MAINNET-KMS-REGION-FINALISATION` |

---

## 2. Q-CD-6 — Option vs Perp BE topology

### 2.1 Decision: distinct EOAs

- **OPTION_BACKEND_EXECUTOR (mainnet)** — distinct EOA, KMS/HSM-backed via Pattern C signer service. **Provisioned at launch.**
- **PERP_BACKEND_EXECUTOR (mainnet)** — distinct EOA placeholder. **Provisioning deferred** until perp scaffold real-broadcast path is implemented in source.

### 2.2 Reasoning

| Reason | Detail |
|---|---|
| Option path is proven | Sepolia rehearsal closed both first-live orderbook smoke and first-live RFQ smoke; R5 drift preserved at 0 across both. |
| Perp path scaffolded only | Source-verified hard-stop at `src/execution/executor.rs:54-58` (`"real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true"`). Provisioning a perp BE today gives it nothing to sign. |
| Blast-radius separation | Per custody policy R-3 + R-7. Compromise of one EOA does not implicate the other. |
| Audit clarity | One EOA per executable surface is the simplest case for AUDIT-EXT to attest. |
| Manifest separation | `mainnet.template.json` already has separate `matchingExecutors.options[0].executor` (line 114) and `matchingExecutors.perps[0].executor` (line 120) slots. |

### 2.3 Launch-scope implication

If perps remain out of mainnet launch scope, the `matchingExecutors.perps[0].executor` slot is filled either with `address(0)` (verify engine accepts a zero executor as "disabled" first) OR left as `TODO_REPLACE_*` with explicit "perp surface not in launch scope" annotation. A future `MAINNET-PERP-BE-PROVISION` milestone provisions the perp EOA after the perp scaffold implements real broadcast.

---

## 3. Q-CD-14 — Region pair (RECOMMENDED-PENDING-PROVIDER)

### 3.1 Structural recommendation

| Region role | Recommendation |
|---|---|
| Primary | EU region (data residency + likely org base) |
| Secondary | EU or nearby compatible region (legal / compliance compatibility, distinct AZ/region) |
| Failover RPO | ≤ 5 min for audit log |
| Failover RTO | ≤ 15 min for signing pipeline; otherwise OPS_MULTISIG pauses NEW_OME |
| DR runbook | required pre-mainnet; rehearsed on staging before first-live-smoke |

### 3.2 Region requirements (independent of vendor)

```text
[ ] low latency to Base RPC provider
[ ] clear data residency
[ ] audit log retention ≥ 1y hot / ≥ 7y cold
[ ] IAM separation between regions (primary compromise must not grant secondary)
[ ] no single-person delete access
[ ] independent network paths
[ ] regional incident channel subscribed
```

### 3.3 Why PENDING-PROVIDER

Exact region names are vendor-specific. The structural recommendation
+ 7 requirements above are locked; exact regions are filled into the
offline ops runbook after vendor selection.

---

## 4. Q-CD-15 — Key-deletion approval lock (POLICY-DECIDED-PROVIDER-DETAIL-PENDING)

### 4.1 Policy summary

| Item | Policy |
|---|---|
| **Permanent key deletion** | Disabled by default if vendor supports it; otherwise ≥ 7-day waiting period + ≥ 2 independent operator approvals + governance sign-off (GOV_SAFE_MAINNET attestation) |
| **Key disable (reversible)** | ≥ 1 operator approval; emergency disable allowed with ≥ 1 SRE/operator approval |
| **Key rotation** | New key provisioned in parallel; new EOA derived; on-chain `setExecutor(new, true)` via Timelock; `setExecutor(old, false)` only AFTER new is live (preserves at-least-one-valid-executor invariant — analogous to V2G-GOV-F-X add-then-remove order) |
| **Emergency IAM revoke** | Faster than disable; revoke `kms:Sign` permission on the BE key; effective within IAM propagation window |
| **Audit logging** | Every disable / rotate / delete / revoke emits custody-event log entry; retention ≥ 1y hot / ≥ 7y cold |
| **Disable ≠ delete** | Disable is reversible; delete is permanent. Custody runbook MUST state which is being performed. |

### 4.2 Break-glass

| Trigger | Action |
|---|---|
| BE compromise suspected | (1) emergency IAM revoke of backend service role; (2) OPS_MULTISIG `NEW_OME.pause()` belt-and-braces; (3) start rotation per §4.1 |
| KMS region outage | failover to secondary region; if backup region also down → OPS_MULTISIG pauses NEW_OME |
| Signer service compromise | revoke service IAM credentials; deploy clean signer service from infra-as-code; rotate underlying key if compromise reached key handle |

### 4.3 Why PROVIDER-DETAIL-PENDING

The procedure shape is locked above; the exact vendor API names, IAM
policy JSON, and multi-actor approval configuration are vendor-specific
and follow Q-CD-5 vendor selection.

---

## 5. Implementation implications

### 5.1 Backend code (deferred to a separate implementation milestone)

| File | Change |
|---|---|
| `src/execution/signer.rs` | New `RemoteSigner` trait; existing `ExecutorSigner::from_private_key` retained for Sepolia / tests; new `KmsRemoteSigner::from_service_endpoint(endpoint)` impl for mainnet. |
| `src/execution/config.rs` | New env keys: `BACKEND_SIGNER_ENDPOINT`, mTLS cert paths. Startup REFUSES `EXECUTOR_PRIVATE_KEY` on mainnet (`chain_id = 8453`). |
| `src/config/env.rs` | Wires new keys; preserves Sepolia env-key path for `chain_id = 84532` only. |
| `src/options/service.rs` (around line 1166, 1213) | Swap `ExecutorSigner` call sites for `RemoteSigner` trait. |
| new module `src/execution/remote_signer.rs` | mTLS client; request/response shape; structured error mapping. |
| new tests | env-keyed signer REFUSED on mainnet chain id; mainnet path requires signer endpoint; mTLS handshake; recovered from-address must equal BE EOA. |

### 5.2 Signer microservice (new repo / crate)

Out-of-tree for backend. Lives in a separate repo OR a separate crate. Responsibilities:
- mTLS server accepting sign requests from backend.
- §6.6 transaction policy precheck (chainId / target / selector / max value 0 / max gas / nonce / rate limit / post-sign from-address verification).
- Adapter for chosen KMS / HSM / MPC provider.
- Structured per-sign log with request_id propagated.
- Failover client to secondary region.
- Emergency disable / pause endpoint (IAM-gated).
- Health endpoint.

### 5.3 Manifest implications

- `matchingExecutors.options[0].executor` (line 114) — fillable once mainnet BE EOA is derived from KMS public key (post Q-CD-5 vendor + Q-CD-14 region resolution).
- `matchingExecutors.perps[0].executor` (line 120) — deferred per Q-CD-6.
- New manifest schema slots recommended (per custody policy §13.3 + Cluster 1 schema gaps):
  - `governanceRoles.kmsKeyHandles.optionBackendExecutor` — KMS handle ARN (not secret).
  - `governanceRoles.kmsKeyHandles.optionBackendExecutorNext` — warm spare handle.

No mainnet BE EOA address is committed in this Cluster 2 resolution.
EOA derivation is the next milestone (post vendor selection).

### 5.4 V2G-Y implications

V2G-Y phase Y-F (NEW_OME executor migration to mainnet BE) requires:

| Y-F requirement | Status after Cluster 2 |
|---|---|
| BE_MAINNET EOA derived | NOT YET (KMS key not provisioned) |
| KMS key generated inside KMS | NOT YET (vendor not selected) |
| Signer microservice deployed | NOT YET (design not complete) |
| Backend `RemoteSigner` trait merged | NOT YET (gap-list D-1 implementation) |
| Sepolia integration test green | NOT YET |
| `MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION` 4-signature attestation | NOT YET |

Cluster 1 already unblocked Y-A and Y-G-1..6. Cluster 2 unblocks the
**planning** of Y-F; **execution** of Y-F still requires the
implementation milestones below.

### 5.5 Audit implications

Auditor (per `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §4.9 / §4.10 / §7 Q-26..Q-30`) reviews:
- Signer microservice trust boundary: does the §6.6 policy layer correctly reject every disallowed tx shape? (Q-27)
- BE private key non-extractable from KMS by any backend service IAM role (Q-26).
- IAM policy minimality (signer service has `kms:Sign` only on the BE key).
- mTLS authn between backend and signer service; bypass-attempt resistance.
- Region failover semantics + key-deletion-lock policy (Q-CD-15).
- Pattern C selection rationale vs Patterns A / B / D.

Note: in earlier custody-policy text, Pattern A was recorded as "fastest path" recommendation. Cluster 2 explicitly **refines to Pattern C** for the security posture. Audit handoff bundle ships this redacted summary as the source of truth on the architecture decision.

---

## 6. What Cluster 2 unblocks

### 6.1 Implementation milestones now scopable

| Milestone | Description | Owner |
|---|---|---|
| `MAINNET-BE-SIGNER-SERVICE-DESIGN` | Read-only design of the signer microservice (Pattern C): API shape, mTLS topology, policy layer, KMS/HSM adapter interface | Backend + Security |
| `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` | `RemoteSigner` trait in backend; KMS/HSM adapter; mainnet env-key refusal | Backend |
| `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE` | Implement `should_broadcast` per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` (gap-list C-4) | Backend + Risk |
| `BACKEND-SIGNER-AUDIT-LOGS-AND-ALERTS` | Per-sign structured log; `kms_request_id` correlation; sign-rate alerts; IAM-revoke alerts | Backend + SRE |
| `MAINNET-KMS-VENDOR-SELECTION` | Sub-decision under Q-CD-5: choose specific KMS/HSM/MPC vendor | Operator + Security + Backend |
| `MAINNET-KMS-REGION-FINALISATION` | Sub-decision under Q-CD-14: pick exact primary + secondary regions | Operator + Security + DevOps |

### 6.2 Items now policy-clear

- Backend startup guard refusing `EXECUTOR_PRIVATE_KEY` on mainnet chain id is **policy-clear**; implementation pending.
- Mainnet BE rotation procedure is **policy-clear** (per §4.1); vendor-specific procedure follows vendor selection.
- Break-glass procedures (§4.2) are **policy-clear**; vendor-specific IAM revoke calls follow vendor selection.

---

## 7. Remaining open decisions (post-Cluster-2)

- **Q-CD-5 vendor name** — sub-decision; operator + security; recorded in offline binder.
- **Q-CD-14 exact regions** — sub-decision; operator + DevOps; follows vendor.
- **Q-CD-15 vendor-specific workflow JSON** — sub-detail; recorded in offline ops runbook.
- **Cluster 3** Q-CD-7 / Q-CD-8 / Q-CD-9 — TREASURY Safe / DEPLOYER form / BE FUND_FLOOR / TARGET / CEILING. BE funding cannot start until TREASURY exists and Q-CD-9 thresholds committed.
- **Cluster 4** Q-CD-10 / Q-CD-11 / Q-CD-12 / Q-CD-16 / Q-CD-17 / Q-CD-18 — PFV revenue receiver, rebates, insurance, cadences, policy version.
- AUDIT-EXT engagement (P0-1) — independent track.
- Mainnet protocol contracts deployment + manifest full fill.
- Backend implementation milestones above.
- Sepolia drill rehearsals (M-1, M-3, D-6) + staging rehearsal (L-5/L-6/L-7).

---

## 8. Private artefact integrity anchor

```
Private artefact sha256 :
  45c4256bbb1bd8b385e0020c030eebf37ffc167c09035de70ad9a7094f1653ab
Private artefact path :
  ~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION.private.md
  (mode 600; dir mode 700; outside all 3 git sub-repos)
Hash log path :
  ~/DEOPT/private/mainnet_custody/CLUSTER_HASHES.txt
  (mode 600; appended with Cluster 2 entry)
```

The Cluster 1 + Cluster 2 hash log is the consolidated integrity
anchor for the custody decision trail. A reader can verify the hashes
against the operator's offline binder copy.

---

## 9. What this document does NOT contain

```text
- NO KMS / HSM / MPC vendor name
- NO KMS account ID / IAM ARN / IAM policy JSON values
- NO API keys / secret tokens / passwords
- NO mainnet BACKEND_EXECUTOR EOA address (not yet derived)
- NO mTLS certificate material / fingerprints / SHA256 of certs
- NO exact region names
- NO private network topology details (subnets, VPC IDs, FQDNs)
- NO signer-service deployment URLs
- NO personal emails / phone numbers / contact details
- NO private keys / seed phrases / mnemonics
- NO RPC API keys / RPC URLs containing secrets
- NO admin tokens / DATABASE_URL values
```

The private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_2_RESOLUTION.private.md`
(mode 600) is the operator's binder counterpart. Vendor name + exact
regions + IAM policy JSON live in the operator's offline custody
binder.

---

## 10. Next milestone

`MAINNET-CUSTODY-CLUSTER-3-RESOLUTION` — operator + Treasury + Finance leads resolve **Q-CD-7** (TREASURY Safe form), **Q-CD-8** (DEPLOYER form), and **Q-CD-9** (BE FUND_FLOOR / FUND_TARGET / FUND_CEILING). Unlocks BE funding flow, DEPLOYER manifest slot, and TREASURY operational policy.

In parallel:
1. **`MAINNET-BE-SIGNER-SERVICE-DESIGN`** — read-only design milestone for the signer microservice (Pattern C) per §5.2 above. Backend + Security.
2. **`MAINNET-KMS-VENDOR-SELECTION`** — sub-decision under Q-CD-5; operator + security + backend. Outputs vendor name into offline binder; emits region-pair finalisation as follow-up.
3. **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship handoff bundle including Cluster 1 + Cluster 2 redacted closure summaries.

All three can run in parallel with Cluster 3 resolution.

---

## 11. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §6 / §7 / §13 / §14
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` Q-CD-5 / Q-CD-6 / Q-CD-14 / Q-CD-15
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` §4 (KMS / backend impact)
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_2_NEXT_ACTIONS.md` (companion — created by this milestone)
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` §4 Y-F
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` §4.10 + §7 Q-26..Q-30
- `~/DEOPT/BACKEND_EXECUTOR_CUSTODY.md`
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` §13
- `~/DEOPT/deopt-v2-backend/docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` §3.1 + §7
- `~/DEOPT/RUN_STATE.md`

**End of public redacted Cluster 2 resolution summary.**
