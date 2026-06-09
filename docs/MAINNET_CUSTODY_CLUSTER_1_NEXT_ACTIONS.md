# Mainnet custody — Cluster 1 next actions

**Posture:** READ-ONLY dependency / next-action doc. **No chain
mutation. No `.env` edit. No Safe-tx. No broadcast. No mainnet.** Companion
to `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` and
`deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md`.

**Date:** 2026-06-08 (initial; closure update 2026-06-09)
**Cluster 1 status (updated 2026-06-09):** **RESOLVED PRIVATELY.**
Operator filled the private input file; both OPS_SAFE_MAINNET and
GOV_SAFE_MAINNET are deployed on Base mainnet at the expected 2-of-3 /
3-of-5 thresholds; roster disjointness confirmed on chain.
Public redacted summary at
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`.
Private artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md`
(mode 600; sha256 in `CLUSTER_1_HASHES.txt`).
**Next actionable step:** `MAINNET-CUSTODY-CLUSTER-2-RESOLUTION`
(KMS / HSM provider + BE topology).

---

## 0. Hard stops (this doc)

```text
no chain tx                                                         ✅
no Safe tx                                                          ✅
no .env edit                                                        ✅
no broadcast                                                        ✅
no mainnet                                                          ✅
no real signer names / contact details written here                 ✅
no signer EOA addresses written here                                ✅
no hardware-wallet serials written here                             ✅
```

---

## 1. Cluster 1 inventory (recap)

| Q-CD | Topic | Required sign-off | Current status |
|---|---|---|---|
| Q-CD-1 | OPS_MULTISIG signer roster | Operator + Security | **OPERATOR-DECIDED-PRIVATE** (3 signers; per-signer detail in offline binder; OPS Safe deployed) |
| Q-CD-2 | OPS_MULTISIG threshold | Operator + Security | **OPERATOR-DECIDED: 2-of-3** (chain-verified `getThreshold() = 2`) |
| Q-CD-3 | GOVERNANCE_MULTISIG signer roster | Operator + Governance | **OPERATOR-DECIDED-PRIVATE** (5 signers; per-signer detail in offline binder; GOV Safe deployed) |
| Q-CD-4 | GOVERNANCE_MULTISIG threshold | Operator + Governance | **OPERATOR-DECIDED: 3-of-5** (chain-verified `getThreshold() = 3`) |
| Q-CD-13 | Sepolia rehearsal commitment | Operator + Security | **OPERATOR-DECIDED: TRUE** (rehearsal log artefact pending) |

Independent on-chain verification (Base mainnet, chainId 8453, public
RPC, no API key, no secret printed):
- OPS_SAFE_MAINNET `0xce0e46Db…C932` — Safe v1.4.1; threshold 2; owners 3; nonce 0.
- GOV_SAFE_MAINNET `0x7C6Ce20e…b166` — Safe v1.4.1; threshold 3; owners 5; nonce 0.
- Roster disjointness: 0 overlap (computed off-disk; owner addresses never persisted).
- Sepolia DEPLOYER not owner of either Safe (mainnet DEPLOYER Q-CD-8 still OPEN — full DEPLOYER attestation reverifies once Q-CD-8 resolves).

Public redacted summary doc:
`deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md`.
Private artefact + sha256:
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md`
+ `CLUSTER_1_HASHES.txt`.

---

## 2. What Cluster 1 unlocks (when resolved)

This catalogues the downstream unblocks that arrive when ALL FIVE
Cluster 1 Q-CDs are filled, signed, and validated. Source:
`deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` §2 / §3 / §5.

### 2.1 Manifest groups unblocked

Per `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` §3 Group A:

| Manifest slot (line) | Cluster 1 dependency | Identity still NEEDS |
|---|---|---|
| `governanceRoles.governanceOwner` (77) | Q-CD-1 + Q-CD-2 + Q-CD-13 | OPS_MULTISIG Safe deploy |
| `governanceRoles.finalGovernanceOwner` (78) | Q-CD-3 + Q-CD-4 + Q-CD-13 | GOVERNANCE_MULTISIG Safe deploy |
| `governanceRoles.timelockOwner` (79) | Q-CD-3 + Q-CD-4 + Q-CD-13 | GOVERNANCE_MULTISIG Safe deploy |
| `governanceRoles.timelockProposers[0]` (83) | Q-CD-1 + Q-CD-2 + Q-CD-13 | OPS_MULTISIG Safe deploy |
| `governanceRoles.timelockExecutors[0]` (86) | Q-CD-1 + Q-CD-2 + Q-CD-13 | OPS_MULTISIG Safe deploy |
| `governanceRoles.governanceGuardian` (99) | Q-CD-1 + Q-CD-2 + Q-CD-13 | OPS_MULTISIG Safe deploy |
| `governanceRoles.moduleGuardians.*` (101-108, 7 module guardians) | Q-CD-1 + Q-CD-2 + Q-CD-13 | OPS_MULTISIG Safe deploy |

**Total manifest slots structurally unblocked:** 13 (governance owners + Timelock roles + 9 module guardian seats).

Remaining manifest slots require OTHER Q-CDs (Cluster 2/3/4) plus
contract deployment + audit sign-off:
- Group B (15 protocol contract addresses): `NEEDS_DEPLOYMENT`.
- Group C (token addresses + deposit caps): mostly `KNOWN` (USDC / WETH) + `BLOCKED_BY_AUDIT` (caps).
- Group D (oracle feeds): `NEEDS_EXTERNAL_SOURCE`.
- Group E (fee recipient / merkle root): Q-CD-11 + V2G-RX deploy.
- Group F (risk params): `BLOCKED_BY_AUDIT`.
- Group G (backend executors): Q-CD-5 + Q-CD-6 + Q-CD-14 + Q-CD-15 (Cluster 2).
- Group K (insurance): Q-CD-7 + Q-CD-12 + Q-CD-17 (Clusters 3 + 4).

### 2.2 Audit sections unblocked

Per `deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md §7 + §14`:

| Audit review item | Cluster 1 unlock |
|---|---|
| Q-28: confirm GOVERNANCE_MULTISIG roster and OPS_MULTISIG roster do not overlap (R-8) | Q-CD-1 + Q-CD-3 resolved with disjointness attested |
| Roster review per §11 engagement-kickoff checklist | Q-CD-1 + Q-CD-3 (rosters identified, even if redacted) |
| Threshold review | Q-CD-2 + Q-CD-4 |
| Sepolia rehearsal evidence per Q-CD-13 | Q-CD-13 TRUE + rehearsal log archived |
| Safe v1.4.1 SafeL2 architecture review | architecture spec already in custody policy §4 |

**AUDIT-EXT kickoff CAN start before Cluster 1 fully resolves** — the
auditor reviews the policy shape + plan + manifest schema. But:
- Sepolia rehearsal evidence (Q-CD-13) MUST be available before the auditor's roster sign-off.
- The roster review item Q-28 requires Q-CD-1 + Q-CD-3 attested-disjoint.

### 2.3 V2G-Y phases unblocked

Per `deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md §3 + §4`:

| V2G-Y phase | Cluster 1 dependency |
|---|---|
| Y-A (guardian wiring on 9 targets to OPS_MULTISIG_MAINNET) | Q-CD-1 + Q-CD-2 + Q-CD-13 |
| Y-B / Y-C / Y-D / Y-E (Timelock-targeted transfers) | indirectly — gate on §3 prerequisites that include OPS Safe existing |
| Y-F (NEW_OME executor migration to mainnet BE) | NOT unblocked by Cluster 1 — requires Cluster 2 (KMS) too |
| Y-G-1 / 1b / 2 / 3 (Timelock cleanup setup → transferOwnership to GOVERNANCE_MULTISIG) | Q-CD-1 + Q-CD-2 + Q-CD-13 (OPS Safe) AND Q-CD-3 + Q-CD-4 (GOV Safe identity) |
| Y-G-4 (point of no return — GOVERNANCE_MULTISIG acceptOwnership) | Q-CD-3 + Q-CD-4 + Q-CD-13 (GOV Safe deployed + roster verified + rehearsed) |
| Y-G-5 / 6 (DEPLOYER strip + 72h minDelay) | Q-CD-3 + Q-CD-4 (GOV Safe signs) |

**Conclusion:** Cluster 1 unlocks Y-A and **partially** unlocks Y-G (the GOV-4-and-after portion). Y-F still requires Cluster 2 (KMS).

### 2.4 Workstream A from P0 roadmap

Per `deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md §4.1`:

| Workstream A step | Cluster 1 dependency |
|---|---|
| Lock custody policy | DONE (`MAINNET_CUSTODY_POLICY.md` shipped) |
| Lock OPS_MULTISIG mainnet roster (≥ 3-of-5, hardware MFA) | Q-CD-1 + Q-CD-2 |
| Lock GOVERNANCE_MULTISIG mainnet roster (≥ 3-of-5 + MFA, disjoint from OPS) | Q-CD-3 + Q-CD-4 |
| Deploy both Safes via Safe Web UI or safe-cli per V2G-GOV-D2 mainnet variant | requires Cluster 1 done first |
| Provision mainnet BACKEND_EXECUTOR EOA + KMS interface | requires Cluster 2 (Q-CD-5, Q-CD-14, Q-CD-15) |
| Sepolia compromise → freeze → rotate → unpause drill (Q-CD-13 / D-6) | partial — Q-CD-13 commitment plus separately-scheduled drill |

**Workstream A is ~50% unblocked by Cluster 1.** The remaining ~50%
(KMS + BE) requires Cluster 2.

---

## 3. Remaining blockers AFTER Cluster 1 resolves

If/when Cluster 1 resolves cleanly, these blockers remain:

### 3.1 Custody decision blockers (Clusters 2 + 3 + 4)

| Cluster | Q-CDs | Unblocks |
|---|---|---|
| Cluster 2 | Q-CD-5, Q-CD-6, Q-CD-14, Q-CD-15 | KMS impl; Y-F; AUDIT-EXT trust-boundary review |
| Cluster 3 | Q-CD-7, Q-CD-8, Q-CD-9 | TREASURY ops; BE funding; DEPLOYER manifest slot |
| Cluster 4 | Q-CD-10, Q-CD-11, Q-CD-12, Q-CD-16, Q-CD-17, Q-CD-18 | Group E + K manifest slots; post-launch ops |

### 3.2 Operational blockers (independent of Cluster 1)

| Item | Source |
|---|---|
| AUDIT-EXT engagement | P0-1; ~10-12 wk external timeline |
| Mainnet manifest fill (76 distinct slots) | P0-2 |
| Mainnet protocol contracts deployment | DeployCore.s.sol + downstream |
| KMS / HSM signer interface (backend code) | gap-list D-1; requires Cluster 2 |
| Backend should_broadcast implementation | gap-list C-4 |
| Monitoring + alerts wiring | gap-list E-1..E-10 |
| V2G-W3 SSR proxy + admin OIDC/MFA + Strict CSP | gap-list J-1 / J-5 / J-6 |
| Sepolia drill rehearsals (M-1, M-3, D-6) | Workstream E |
| Staging rehearsal (L-5 / L-6 / L-7) | full deploy + smoke + drills |

Cluster 1 closure does NOT unblock these — they progress in parallel
on independent timelines.

---

## 4. Next recommended milestone (Cluster 1 RESOLVED state)

Cluster 1 closed privately on 2026-06-09 with chain-anchored
attestation. Next milestones in priority order:

1. **`MAINNET-CUSTODY-CLUSTER-2-RESOLUTION`** — Q-CD-5 / Q-CD-6 / Q-CD-14 / Q-CD-15 (KMS / HSM provider + BE topology + region pair + key-deletion lock). Unlocks gap-list D-1 (backend KMS interface impl) and V2G-Y Y-F. **Highest single-leverage item: Q-CD-5** (KMS provider Pattern A/B/C).
2. **`MAINNET-AUDIT-EXT-KICKOFF`** (P0-1) — ship handoff bundle including the redacted Cluster 1 closure summary. Longest external timeline (~10-12 weeks). Runs in parallel.
3. **`MAINNET-MANIFEST-FILL-GROUP-A`** — read-only manifest-fill PR (no chain action) writing the 13 Group A slots that Cluster 1 unblocks. Companion to P0-2 progress.
4. **`MAINNET-CUSTODY-CLUSTER-1-REHEARSAL-LOG`** (operator follow-up) — execute the Q-CD-13 Sepolia rehearsal sessions for both signer rosters; archive rehearsal log; back-fill the §6 rehearsal-log path + §7.2 sign-off labels in the private input file (no public-doc change required).
5. **`MAINNET-CUSTODY-CLUSTER-3-RESOLUTION`** (after Cluster 2 or in parallel) — Q-CD-7 / Q-CD-8 / Q-CD-9 (TREASURY + DEPLOYER + BE funding).

Items 1, 2, 3, and 4 can run in parallel.

---

## 5. Files produced / referenced by this milestone

| Path | Status |
|---|---|
| `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_OPERATOR_INPUT_TEMPLATE.private.md` | filled in place by operator on 2026-06-09; mode 600 |
| `~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md` | **CREATED** on 2026-06-09 (mode 600, outside all repo trees) |
| `~/DEOPT/private/mainnet_custody/CLUSTER_1_HASHES.txt` | **CREATED** on 2026-06-09 (mode 600) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md` | **CREATED** on 2026-06-09 (public redacted summary) |
| `deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_NEXT_ACTIONS.md` | **UPDATED** on 2026-06-09 (this file; resolution state) |
| `~/DEOPT/RUN_STATE.md` | **APPENDED** with redacted closure note (no sensitive details) |

**No source touched. No `.env` edit. No chain mutation. No Safe-tx.
No mainnet broadcast.** Read-only chain probes against
`https://mainnet.base.org` (public, no API key) verified Safe state.

---

## 6. What this doc does NOT do

```text
- Does NOT write real signer identities
- Does NOT write real mainnet addresses
- Does NOT mark any Q-CD as OPERATOR-DECIDED
- Does NOT broadcast any tx
- Does NOT edit any .env
- Does NOT modify any source code
- Does NOT touch mainnet
- Does NOT bind any decision until operator fills the private template
```

---

## 7. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §16 (verbatim Q-CD source)
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` (per-Q-CD detail)
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` (unlock matrix)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` §3 (Group A slots)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md` §3 + §4 (phase dependencies)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` §7 + §14 (audit Q-CD impact)
- `~/DEOPT/deopt-v2-backend/docs/P0_MAINNET_BLOCKER_CLOSURE_ROADMAP.md` §4.1 (Workstream A)
- `~/DEOPT/deopt-v2-sol/docs/GOVERNANCE_OPS_MULTISIG_DEPLOY_PLAN_V2G_GOV_D2.md` (Safe deploy reference; mainnet variant referenced)
- `~/DEOPT/RUN_STATE.md`

**End of Cluster 1 next-actions doc.**
