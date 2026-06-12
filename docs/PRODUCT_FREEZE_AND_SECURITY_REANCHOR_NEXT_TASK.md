# PRODUCT-FREEZE-AND-SECURITY-REANCHOR — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `PUBLIC_BETA_DOCS_PACK_NEXT_TASK.md` + `docs/public-beta/KNOWN_LIMITATIONS_AND_RISKS.md` §1.
**Target:** re-confirm the frozen ABI / selectors at `deopt-v2-sol/abis/freeze-v2-product-rc1/` against the canonical retargeted matching engine + margin engine, and assemble a public-facing **security-review packet** as a precursor to (not a replacement for) any external audit engagement.
**Posture:** **Docs + ABI verification only. NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER claim "audited" or "mainnet-ready".**

> **This task is NOT executed by the calling milestone. It packages the product-freeze re-anchor + security packet drafting into one approval-gated milestone.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 product freeze and security re-anchor for this run."

Properties:
* Authorises read-only chain checks (selectors, owner, version).
* Authorises docs + ABI manifest updates.
* Authorises drafting the security-review packet.
* Does NOT authorise external audit engagement, bug-bounty launch, mainnet deployment, or any chain transaction.

---

## 2. Scope — product freeze re-anchor

* Re-confirm `deopt-v2-sol/abis/freeze-v2-product-rc1/freeze-manifest.json` matches the bytecode + selectors at the canonical retargeted addresses (`0x5a5EBF9A…` ME and `0x506cD65a…` MarginEngine).
* If drift is detected, document it and decide: re-freeze to `freeze-v2-product-rc2`, OR update the manifest to flag the new sub-version. (Operator picks; no contract changes in this brief.)
* Verify `selectors.txt` and `storage-layouts.txt` against the on-chain bytecode via `cast` (read-only):
  ```bash
  cast code 0x5a5EBF9A9CCd7c012518569DE8283982982670f6 --rpc-url $RPC | sha256sum
  cast code 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30 --rpc-url $RPC | sha256sum
  ```
* Cross-check that the EIP-712 domain separator returned by `OptionMatchingEngine.domainSeparatorV4()` matches the value derived from the frozen ABI's `eip712Domain()` parameters.

## 3. Scope — security-review packet drafting

Draft the following docs under `deopt-v2-sol/docs/security-review-packet/`:

* `README.md` — packet overview, intended audience (internal reviewers, future auditors).
* `THREAT_MODEL.md` — actors (buyer, seller, executor, operator, attacker), trust assumptions, attack surface enumeration.
* `INVARIANTS.md` — protocol invariants:
  * `nonces(addr)` is monotonically non-decreasing on every successful executeTrade.
  * `vault.balances(seller, settlement) - premium ≥ short collateral requirement` at trade time.
  * Net fee transferred to fee recipient equals `buyer_side_fee + seller_side_fee` recorded in the FeesManager event.
  * Position quantities sum to zero across buyer + seller per series.
  * No `executeTrade` succeeds with a stale oracle (where `OracleRouter.getPriceSafe` returns `ok=false`).
* `KNOWN_ISSUES.md` — known limitations: legacy stale ME on chain, mock-oracle 60s maxDelay, signature-verification disabled on the backend (chain-side verification still enforced).
* `OUT_OF_SCOPE.md` — explicitly out-of-scope for any future review: AWS / KMS / production signer integration (not deployed); mainnet-specific gas optimizations (no mainnet target).
* `AUDIT_REQUEST_OUTLINE.md` — placeholder for the eventual external audit request: scope, timeline, deliverables. Mark "NOT INITIATED" prominently.

## 4. Scope — explicitly NOT this milestone

* **No external audit engagement.** This packet is a precursor; no firm is contacted under this approval.
* **No bug-bounty program launch.** Same.
* **No mainnet deployment.** Same.
* **No chain transaction.** Read-only `cast` calls only.
* **No claim "audited" / "mainnet-ready" / "production-grade" / "safe for real funds"** in any doc.

---

## 5. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | Chain id `84532` | `cast chain-id` |
| P3 | Both retargeted addresses have bytecode | `cast code` |
| P4 | Frozen ABI manifest at `freeze-v2-product-rc1/freeze-manifest.json` exists and parses as JSON | `jq .` |
| P5 | `.env` (`deopt-v2-backend/.env`) untouched | `stat -c '%y'` |
| P6 | Private file untouched | `stat -c '%a %y'` |
| P7 | `~/DEOPT/private/mainnet_custody/` NOT read (out of scope) | trust |

---

## 6. Forbidden

* No mainnet RPC.
* No source-code changes that touch contracts.
* No "audited" / "mainnet-ready" / "production-grade" wording.
* No private key handling.
* No bug-bounty rules published (this packet is a precursor; bounty rules come later, separately).

---

## 7. Acceptance criteria

* `freeze-manifest.json` either reconfirmed unchanged OR drift flagged with a specific sub-version.
* All six security-packet docs created with substantive content (not placeholders).
* `docs/public-beta/KNOWN_LIMITATIONS_AND_RISKS.md` §1 updated to point to the new packet location.
* `git diff --check` clean.
* No chain transaction invoked.
* Mainnet / audit / bug bounty remain explicitly out of scope.

---

## 8. Cross-links

* `deopt-v2-sol/abis/freeze-v2-product-rc1/`
* `deopt-v2-sol/docs/SOL_PRODUCT_SCOPE_FREEZE_RESULT.md`
* `deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md`
* `docs/public-beta/KNOWN_LIMITATIONS_AND_RISKS.md`
* `docs/public-beta/README.md`
* `~/DEOPT/RUN_STATE.md`

**End of product freeze and security re-anchor next-task brief.**
