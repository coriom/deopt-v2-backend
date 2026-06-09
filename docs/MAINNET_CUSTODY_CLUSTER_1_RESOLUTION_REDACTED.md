# Mainnet custody — Cluster 1 resolution (REDACTED public summary)

**Posture:** READ-ONLY redacted public summary. **No chain mutation.
No `.env` edit. No Safe-tx. No broadcast. No mainnet broadcast.**
Public companion to the private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md`
(mode 600, outside all repo trees).

**Date validated (UTC):** 2026-06-09
**Validator:** read-only doc + on-chain Base mainnet checks (`chainId = 8453`)

This document contains **NO** signer names, signer EOA addresses,
hardware-wallet serials, personal emails, phone numbers, private
keys, seed phrases, mnemonics, recovery phrases, RPC secrets,
admin tokens, or DATABASE_URL values. Only thresholds, counts,
booleans, the two public Safe addresses, sha256 hashes, and the
UTC timestamp.

---

## 0. Cluster 1 closure status

**Cluster 1 (Q-CD-1 / Q-CD-2 / Q-CD-3 / Q-CD-4 / Q-CD-13): RESOLVED PRIVATELY.**

| Q-CD | Status | Notes |
|---|---|---|
| **Q-CD-1** OPS_MULTISIG signer roster | **OPERATOR-DECIDED-PRIVATE** | 3 signers; per-signer detail in offline binder |
| **Q-CD-2** OPS_MULTISIG threshold | **OPERATOR-DECIDED: 2-of-3** | minimum acceptable per custody policy §4.2 |
| **Q-CD-3** GOVERNANCE_MULTISIG signer roster | **OPERATOR-DECIDED-PRIVATE** | 5 signers; per-signer detail in offline binder |
| **Q-CD-4** GOVERNANCE_MULTISIG threshold | **OPERATOR-DECIDED: 3-of-5** | minimum acceptable per custody policy §4.1 |
| **Q-CD-13** Sepolia rehearsal commitment | **OPERATOR-DECIDED: TRUE** | rehearsal mandatory before mainnet roster lock |

---

## 1. Public Safe addresses (Base mainnet)

| Role | Address | Network |
|---|---|---|
| OPS_SAFE_MAINNET | `0xce0e46Db1072B820CB5eCf30188ED76cb560C932` | Base (chainId 8453) |
| GOV_SAFE_MAINNET | `0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166` | Base (chainId 8453) |

Both Safes are independently verifiable on a Base mainnet block
explorer (no API key required).

---

## 2. Independent on-chain verification

Performed via public Base RPC (`https://mainnet.base.org`; no API
key, no secret printed):

| Subject | Value | Verdict |
|---|---|---|
| Reported `chain_id` | `8453` | ✓ Base mainnet |
| OPS Safe code length | `171` bytes | ✓ Safe v1.4.1 proxy shape |
| OPS Safe `VERSION()` | `"1.4.1"` | ✓ |
| OPS Safe `getThreshold()` | `2` | ✓ matches operator-attested 2-of-3 |
| OPS Safe owners count | `3` | ✓ |
| OPS Safe `nonce()` | `0` | ✓ fresh — no tx signed yet |
| GOV Safe code length | `171` bytes | ✓ Safe v1.4.1 proxy shape |
| GOV Safe `VERSION()` | `"1.4.1"` | ✓ |
| GOV Safe `getThreshold()` | `3` | ✓ matches operator-attested 3-of-5 |
| GOV Safe owners count | `5` | ✓ |
| GOV Safe `nonce()` | `0` | ✓ fresh — no tx signed yet |
| **Roster disjointness** | computed off-disk; owner addresses never persisted to a tracked or untracked file | **`0` overlap** ✓ R-8 satisfied at chain level |
| Sepolia DEPLOYER (`0xc35F7A8A…3C27`) `isOwner` on OPS | `false` | ✓ structural sanity |
| Sepolia DEPLOYER `isOwner` on GOV | `false` | ✓ structural sanity |

**Mainnet DEPLOYER address is not yet committed (Q-CD-8 still
OPEN).** Operator-attested "no DEPLOYER in either Safe" cannot be
fully verified on chain until Q-CD-8 resolves. The structural probe
against the only known DEPLOYER identity (Sepolia DEPLOYER) returns
`false` on both Safes.

---

## 3. Operator-attested booleans (from input file §8 public summary)

| Attestation | Value |
|---|---|
| No DEPLOYER in OPS roster | `true` |
| No DEPLOYER in Governance roster | `true` |
| OPS / Governance rosters disjoint | `true` (independently confirmed on chain — §2) |
| Hardware wallets required for all signers | `true` |
| Sepolia rehearsal required before mainnet roster lock | `true` |

All five required-true attestations are present in the operator's
private input file.

---

## 4. Private artefact hashes (integrity anchor)

| Artefact | sha256 |
|---|---|
| Private input file (filled in place; mode 600) | `81e2fbe02384ea123a99e6afbbd060ae0c589a188c596dcfbc9854cfc6f2f01d` |
| Private resolution file (mode 600) | `a845594dfd57401ce5d80fb0a972754255e8ab2874cff51742481f4476a50a26` |

Both artefacts live at `~/DEOPT/private/mainnet_custody/` (dir mode
700, files mode 600). Outside all 3 git sub-repos.

A reader who later wants to audit the private state without
re-asking the operator can verify these hashes against the operator's
offline binder copy.

---

## 5. Validator summary

```text
private_input_present                : ✓
private_input_outside_all_repos      : ✓
private_input_mode                   : 0600 ✓
private_dir_mode                     : 0700 ✓
no_secret_patterns_in_private_input  : ✓ (hex≥64 / KEY=val / BIP-39 / email / phone all 0)
no_secret_patterns_in_tracked_docs   : ✓
ops_threshold_attested               : 2-of-3
gov_threshold_attested               : 3-of-5
both_safes_deployed_on_base          : ✓ (chainId 8453, Safe v1.4.1)
ops_safe_threshold_on_chain          : 2 ✓
ops_safe_owners_on_chain             : 3 ✓
gov_safe_threshold_on_chain          : 3 ✓
gov_safe_owners_on_chain             : 5 ✓
both_safes_nonce_fresh               : 0 / 0 ✓
roster_disjointness_on_chain         : 0 overlap ✓
no_DEPLOYER_attested                 : ✓ (Sepolia DEPLOYER probed false on both Safes;
                                          mainnet DEPLOYER attestation pending Q-CD-8)
hardware_wallet_requirement_attested : ✓
sepolia_rehearsal_commitment         : TRUE
sign_off_labels_filled               : ✗ left blank in input file (operator follow-up)
sign_off_utc_date_filled             : ✗ left blank in input file (operator follow-up)
overall_status                       : RESOLVED PRIVATELY; substantive decisions chain-anchored
```

---

## 6. What this resolution unlocks (now writeable)

### 6.1 Manifest fill — 13 Group A slots now writeable

Per `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` §3 Group A:

| `mainnet.template.json` slot (line) | Now writeable as |
|---|---|
| `governanceRoles.governanceOwner` (77) | OPS_SAFE_MAINNET |
| `governanceRoles.timelockProposers[0]` (83) | OPS_SAFE_MAINNET |
| `governanceRoles.timelockExecutors[0]` (86) | OPS_SAFE_MAINNET |
| `governanceRoles.governanceGuardian` (99) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.collateralVault` (101) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.oracleRouter` (102) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.marginEngine` (103) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.perpEngine` (104) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.feesManager` (105) | `address(0)` per policy note A-1 (FM-V2 has no guardian field) |
| `governanceRoles.moduleGuardians.insuranceFund` (106) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.matchingEngine` (107) | OPS_SAFE_MAINNET |
| `governanceRoles.moduleGuardians.perpMatchingEngine` (108) | OPS_SAFE_MAINNET |
| `governanceRoles.finalGovernanceOwner` (78) | GOV_SAFE_MAINNET |
| `governanceRoles.timelockOwner` (79) | GOV_SAFE_MAINNET |

### 6.2 V2G-Y phases unlocked

- **Y-A** (guardian wiring on 9 targets → OPS_SAFE_MAINNET) — fully parameterised.
- **Y-G-1a / Y-G-1b / Y-G-2 / Y-G-3** (Timelock: setProposer + setExecutor + setGuardian + transferOwnership wiring OPS as proposer/executor/guardian, then transfer ownership pending acceptance by GOV) — fully parameterised.
- **Y-G-4** (GOV_SAFE_MAINNET acceptOwnership — point of no return) — Safe ready.
- **Y-G-5a / Y-G-5b / Y-G-6** (GOV strips DEPLOYER + sets minDelay 72h) — Safe ready.
- **Y-F** (NEW_OME executor migration to mainnet BE) — **NOT** unlocked; still needs Cluster 2 KMS.

### 6.3 AUDIT-EXT review items partially unblocked

- Q-28 (roster disjointness R-8) — confirmed on chain.
- Roster review per audit-package §11 — Safes deployed and independently verifiable.
- Threshold review — Safes deployed at expected shapes.
- Q-CD-13 Sepolia rehearsal evidence — pending the rehearsal-log artefact (operator follow-up).

---

## 7. What remains blocked

- **Cluster 2** Q-CD-5/6/14/15: KMS / HSM provider + region + BE topology.
- **Cluster 3** Q-CD-7/8/9: TREASURY Safe form + DEPLOYER form + BE funding policy.
- **Cluster 4** Q-CD-10/11/12/16/17/18: PFV revenue receiver, rebates, insurance, cadences.
- AUDIT-EXT engagement (P0-1; longest external timeline).
- Mainnet manifest full fill (≥ 62 of 76 distinct slots remain).
- Mainnet protocol contracts deployment.
- Actual protocol ownership migration (V2G-Y Y-A → Y-G execution).
- Backend `should_broadcast` impl (gap-list C-4).
- KMS / HSM signer interface impl (gap-list D-1; depends on Cluster 2).
- Monitoring + alerts wiring (E-1..E-10).
- V2G-W3 SSR proxy + admin OIDC/MFA + Strict CSP.
- Sepolia drill rehearsals (M-1, M-3, D-6); staging rehearsal (L-5/6/7).

---

## 8. What this document does NOT contain

```text
- NO signer names
- NO signer EOA addresses (Safe owner lists)
- NO hardware-wallet serial numbers
- NO personal emails
- NO phone numbers
- NO mailing addresses
- NO private keys
- NO seed phrases / mnemonics / recovery phrases
- NO RPC API keys / RPC URLs containing secrets
- NO admin tokens
- NO DATABASE_URL values
- NO sign-off label values from §7.2 of the input file
- NO per-signer hardware-wallet vendor / region / role / binder-ref details
```

The private resolution artefact at
`~/DEOPT/private/mainnet_custody/MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md`
(mode 600) is the operator's offline-binder counterpart. Per-signer
identity mapping lives in the operator's offline custody binder
referenced by opaque binder ref.

---

## 9. Next milestone

`MAINNET-CUSTODY-CLUSTER-2-RESOLUTION` — operator + Security +
Backend leads work through Q-CD-5 / Q-CD-6 / Q-CD-14 / Q-CD-15
(KMS / HSM provider + BE topology + region + key-deletion lock).
Unlocks gap-list D-1 (backend KMS interface impl) and V2G-Y Y-F.

In parallel, the operator may:
- Schedule Sepolia rehearsal sessions for both signer rosters (Q-CD-13 closure).
- Kick off `MAINNET-AUDIT-EXT-KICKOFF` (P0-1) shipping this redacted summary + the verified Safe addresses in the handoff bundle.
- Update mainnet manifest draft with the 13 Group A slots above (separate manifest-fill PR, no chain action).

---

## 10. Cross-links

- `~/DEOPT/MAINNET_CUSTODY_POLICY.md` §16 Q-CD source
- `~/DEOPT/MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` (per-Q-CD detail)
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_DECISION_DEPENDENCY_MAP.md` (unlock matrix)
- `~/DEOPT/deopt-v2-backend/docs/MAINNET_CUSTODY_CLUSTER_1_NEXT_ACTIONS.md` (updated by this milestone)
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` Group A
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md`
- `~/DEOPT/RUN_STATE.md`

**End of public redacted Cluster 1 resolution summary.**
