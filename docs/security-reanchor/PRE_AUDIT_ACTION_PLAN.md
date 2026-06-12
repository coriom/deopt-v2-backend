# DeOpt V2 — Pre-Audit Action Plan

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

The concrete action list to close the BLOCKERS + SHOULD-FIX items from `AUDIT_READINESS_GAP_ANALYSIS.md`. **This document does NOT initiate audit outreach.** Outreach is a strictly separate later milestone tracked in `deopt-v2-backend/docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md`.

---

## Sequencing

Items are ordered by dependency, not by importance. Some can be parallelised; the dependency graph is noted per item.

| # | Item | Owner | Depends on | Estimated effort |
|---|---|---|---|---|
| 1 | Publish Solidity test inventory + coverage delta | Sol maintainer | — | S |
| 2 | Write `INVARIANTS.md` | Sol maintainer | 1 | M |
| 3 | Write `THREAT_MODEL.md` | Sol + backend leads | — | M |
| 4 | Write `KNOWN_ISSUES.md` (audit-facing) | Maintainer + operator | 2, 3 | S |
| 5 | Write `OUT_OF_SCOPE.md` (audit-facing) | Maintainer | — | XS |
| 6 | Write `AUDIT_REQUEST_OUTLINE.md` (NOT INITIATED) | Operator | — | XS |
| 7 | Re-confirm `freeze-manifest.json` against on-chain bytecode | Operator | — | XS |
| 8 | Write backend test inventory + coverage one-pager | Backend maintainer | — | S |
| 9 | CI-run the Playwright suite on Linux + archive results | Frontend maintainer | — | XS |
| 10 | Write backend-side signature-verification posture doc | Backend maintainer | — | XS |
| 11 | Write indexer reorg-handling policy | Backend maintainer | — | XS |
| 12 | Add storage-layout CI diff hook (sol) | Sol maintainer | — | S |
| 13 | Resolve open `Q-CD-*` decisions | Operator | — | M |
| 14 | Add audit-trail requirement to flag-flip runbook | Operator | — | XS |
| 15 | Write status envelope semantics rationale | Backend maintainer | — | XS |
| 16 | Write public-vs-admin API boundary doc | Backend maintainer | — | XS |
| 17 | Cross-link the address registry from the security packet | Maintainer | — | XS |

Effort scale: XS = ≤ 1 hour, S = ≤ 1 day, M = ≤ 1 week, L = > 1 week.

---

## 1. Publish Solidity test inventory + coverage delta

### What
A `deopt-v2-sol/docs/TEST_INVENTORY.md` containing:
* The actual location of the Solidity test suite (path).
* The commands to run it (`forge test`, `forge coverage`, etc.).
* A per-contract coverage table.
* Known gaps (which functions are NOT tested + why).

### Why
First thing an auditor asks for. Without it, scope estimation balloons.

### Acceptance
* Doc exists at the path above.
* `forge test` reproduces from the documented path.
* Coverage table includes every contract in `freeze-v2-product-rc1/`.

---

## 2. Write `INVARIANTS.md`

### What
`deopt-v2-sol/docs/security-review-packet/INVARIANTS.md` with one section per invariant. Required invariants per the next-task brief §3:

* **Nonce monotonicity:** `nonces(addr)` is monotonically non-decreasing on every successful `executeTrade`.
* **Collateral sufficiency at trade time:** `vault.balances(seller, settlement) - premium ≥ short collateral requirement`.
* **Fee zero-sum:** Net fee transferred to fee recipient equals `buyer_side_fee + seller_side_fee` recorded in the FeesManager event.
* **Position zero-sum per series:** Buyer-long quantity + seller-short quantity sums to zero across the buyer/seller pair.
* **Oracle freshness gate:** No `executeTrade` succeeds with a stale oracle (`OracleRouter.getPriceSafe` returns `ok=false`).

Each section: statement, where enforced, evidence (test pointer), known assumptions.

### Why
Audit deliverable + reduces auditor discovery.

### Acceptance
* Doc covers the 5 invariants above.
* Each row points at the enforcement code + a test (when 1 is done).

---

## 3. Write `THREAT_MODEL.md`

### What
`deopt-v2-sol/docs/security-review-packet/THREAT_MODEL.md` covering:

* **Actors:** buyer, seller, executor, operator, contract owner, attacker, public observer.
* **Trust assumptions:** which actors are trusted with what (operator: oracle pushes on testnet; executor: broadcasts a signed trade; etc.).
* **Attack surface:** signature replay, nonce manipulation, oracle manipulation, vault drain, fee dilution, indexer poisoning, signature forgery, etc.
* **Per-attack:** which control mitigates, what residual risk remains.

### Why
Audit deliverable; structures the auditor's planning.

### Acceptance
* Doc enumerates the actors / assumptions / attack surface above.
* Each attack has a mitigation pointer.

---

## 4. Write `KNOWN_ISSUES.md` (audit-facing)

### What
Consolidate the known issues already scattered across docs:

* Legacy stale `OptionMatchingEngine 0xf2D1D85…` on chain (cannot call canonical MarginEngine).
* Mock oracle 60s `maxDelay` (testnet only; production design pending).
* Backend-side signature verification disabled (chain-side still enforced — link to S-4 in the gap analysis).
* Public-beta link placeholders deliberately retained (no live channel yet).
* `Q-CD-*` decisions still open (link to S-7).

### Why
Auditor reads first, pre-scopes around known issues.

### Acceptance
* Doc consolidates the 5+ known issues.
* Each row has a "documented in" pointer.

---

## 5. Write `OUT_OF_SCOPE.md` (audit-facing)

### What
Re-frame the OUT-OF-SCOPE-DEFER and OUT-OF-SCOPE-FOREVER columns from `CROSS_REPO_SCOPE_MATRIX.md` as scope-letter constraints.

### Why
Prevents auditor scope-creep + sets expectations on day 0.

### Acceptance
* Doc lists what an auditor must NOT review (with a one-line "because" per item).

---

## 6. Write `AUDIT_REQUEST_OUTLINE.md` (NOT INITIATED)

### What
Sketch of the scope letter we'd send if we were initiating audit outreach:

* Target scope (the in-scope items from `CROSS_REPO_SCOPE_MATRIX.md`).
* Target timeline (placeholder — TBD).
* Target deliverables (audit report + findings + executive summary).

### Why
Forces articulation. Outreach is much higher quality when this is pre-written.

### Acceptance
* Doc exists.
* Doc is prominently marked **NOT INITIATED** at the top.

---

## 7. Re-confirm `freeze-manifest.json` against on-chain bytecode

### What
Read-only chain-state check (Base Sepolia only):
```bash
cast code 0x5a5EBF9A9CCd7c012518569DE8283982982670f6 --rpc-url <base-sepolia-rpc> | sha256sum
cast code 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30 --rpc-url <base-sepolia-rpc> | sha256sum
```
Compare against the artefacts in `deopt-v2-sol/abis/freeze-v2-product-rc1/`.

### Why
The packet's integrity rests on the frozen artefacts matching what's actually on chain.

### Acceptance
* Either drift-free (artefact hashes match on-chain code) OR drift documented with a specific re-freeze sub-version (`freeze-v2-product-rc2`).
* NO chain transaction sent.

---

## 8. Backend test inventory one-pager

### What
`deopt-v2-backend/docs/TEST_INVENTORY.md`:
* List the test modules (`mod tests`) grouped by domain.
* Documented gaps (which paths are NOT tested + why).
* Reproduce commands (`cargo test`, `cargo test --release`, env required).

### Why
Audit deliverable. Mirrors item 1 on the backend side.

### Acceptance
* Doc covers the ~49 test modules.
* Reproduce commands documented.

---

## 9. CI-run the Playwright suite on Linux

### What
Spin up a CI run on a Linux box with `libnspr4` available. Archive the result HTML / JSON.

### Why
The local sandbox (WSL2) cannot run chromium; the packet currently relies on `--list` output for the spec graph. Auditor will ask for an actual run.

### Acceptance
* CI log archived.
* All specs green OR known failures documented.

---

## 10. Backend-side signature verification posture doc

### What
A one-page doc explaining:
* Why backend-side EIP-712 verification is currently disabled.
* Why chain-side verification is still authoritative (no security loss).
* When backend-side verification will be enabled (separate later milestone).

### Why
Auditor will notice and ask. Get ahead of it.

### Acceptance
* Doc exists.

---

## 11. Indexer reorg-handling policy

### What
A one-page doc:
* Confirmation depth (currently `N` blocks — fill in the actual value).
* Reorg-recovery contract: indexer rewinds to divergence point and re-processes.
* What "deep reorg" means and what the operator does.

### Why
Auditor will ask.

### Acceptance
* Doc exists.

---

## 12. Storage-layout CI diff hook

### What
CI step that, for each in-scope contract, runs:
```bash
forge inspect <Contract> storageLayout > /tmp/<Contract>.layout
diff -u abis/freeze-v2-product-rc1/storage-layouts.txt <(jq -r ".<Contract>" /tmp/<Contract>.layout)
```
PR fails on drift.

### Why
The `storage-layouts.txt` snapshot is only useful if it's enforced.

### Acceptance
* CI step active on every PR touching `deopt-v2-sol/src/`.

---

## 13. Resolve open `Q-CD-*` decisions

### What
For each open decision in `MAINNET_CUSTODY_POLICY.md`:
* `Q-CD-2` — OPS_MULTISIG threshold (2-of-3 vs 3-of-5).
* `Q-CD-5` — KMS / HSM vendor selection.
* `Q-CD-6` — perp scope at launch.

Either decide OR explicitly defer with a date.

### Why
Auditor will ask. "Open" without a date is a red flag.

### Acceptance
* Each decision row has a resolution status (`DECIDED: <value>` or `DEFERRED until <date>`).

---

## 14. Audit-trail requirement to flag-flip runbook

### What
Update `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`:
* Every flip must produce a structured log entry (timestamp, operator id, reason, prior value, new value).
* Multi-operator approval required for flipping `false → true`.
* The audit log is retained.

### Why
Currently relies on the operator's manual log. Audit-grade requires automation.

### Acceptance
* Runbook updated.
* (Implementation of the audit log itself is POST-AUDIT.)

---

## 15-17. Smaller docs

### 15. Status envelope semantics rationale
* Short doc on why `partial` is the right response on optional-source absence.

### 16. Public vs admin API boundary
* For each route: public / admin / which gate / fail-closed behaviour.

### 17. Cross-link address registry
* Add explicit cross-links from this packet to `docs/public-beta/CONTRACT_ADDRESSES_BASE_SEPOLIA.md` so the auditor doesn't have to hunt.

---

## Out of this plan (explicit)

This plan does **NOT** include:

* Contacting an audit firm. (Tracked in `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md`.)
* Drafting outreach copy. (Same.)
* Negotiating SOW / scope letter. (Same.)
* Launching a bug-bounty program. (Strictly later.)
* Mainnet deployment. (Strictly later.)
* AWS KMS / Safe multisig cutover. (Tracked in `MAINNET_LAUNCH_READINESS_NEXT_TASK.md`.)
* Production monitoring / alerting / IR. (Same.)

If any of these creep in: pause the milestone, route to the appropriate next-task brief.

---

## Definition of done

The packet is ready for an external auditor when:
* All 7 BLOCKERS (B-1 through B-7) are closed.
* All 8 SHOULD-FIX (S-1 through S-8) are closed OR explicitly accepted with rationale.
* The DOCUMENT-FOR-AUDIT items (D-1 through D-4) are written.
* This plan + the rest of the security re-anchor packet are signed off by the operator + Sol/backend/frontend maintainers.

Only then does `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` get its approval line.

---

**End of pre-audit action plan.**
