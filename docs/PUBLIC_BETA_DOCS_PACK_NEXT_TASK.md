# PUBLIC-BETA-DOCS-PACK — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md` §15.
**Target:** prepare the public-beta docs surface for the DeOpt V2 Base Sepolia testnet beta, now that the on-chain trade lifecycle is end-to-end demonstrated and the backend mirrors it.
**Posture:** **Docs-only. NEVER chain transactions. NEVER mainnet. NEVER Safe tx. NEVER AWS / KMS. NEVER production signer. NEVER `.env` edit. NEVER private key in any artefact.**

> **This task is NOT executed by the calling milestone. It packages the docs/UX work into one approval-gated sequence; each public-facing doc requires its own operator review before publication.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 Base Sepolia public-beta docs pack for this run."

Properties:
* Authorises docs creation + cross-doc updates only.
* Does NOT authorise mainnet deployment, audit engagement, bug-bounty launch, fundraising claims, or any "audited" / "mainnet-ready" wording.
* Expires 4 hours after the approval is received.

---

## 2. Scope (what the pack DOES)

* `docs/PUBLIC_BETA_OVERVIEW.md` — what DeOpt V2 is; what beta means; what is and isn't ready.
* `docs/PUBLIC_BETA_TESTNET_RUNBOOK.md` — refresh of `~/DEOPT/TESTNET_RUNBOOK.md` for the public-beta context (Sepolia only).
* `docs/PUBLIC_BETA_USER_GUIDE.md` — step-by-step user guide: connect wallet, faucet, get mUSDC, place trade.
* `docs/PUBLIC_BETA_RISK_DISCLOSURES.md` — testnet posture; no real value; no audit; no production guarantees.
* `docs/PUBLIC_BETA_API_REFERENCE.md` — pointer to `openapi/trading-api.openapi.json` + curated examples.
* `docs/PUBLIC_BETA_KNOWN_LIMITATIONS.md` — current gaps, including:
  * mock oracle with `maxDelay=60s`;
  * single-pair series #0 demonstrated;
  * legacy stale ME `0xf2D1D85…` retained on chain (not used by the canonical flow);
  * backend reconciliation worker is local-only (no shared cluster yet);
  * front-end wallet broadcast flow not yet exercised end-to-end.
* `docs/PUBLIC_BETA_OPERATOR_PLAYBOOK.md` — how the operator runs a Sepolia smoke once the feeds and balances are in the documented baseline state.
* Refresh of the existing `~/DEOPT/TESTNET_RUNBOOK.md` index with the retargeted addresses.
* Cross-link banner on `RUN_STATE.md` pointing operators to the public-beta entry.

## 3. Scope (what the pack DOES NOT do)

* No mainnet (chain id `8453`).
* No external audit engagement.
* No bug-bounty program launch.
* No marketing / fundraising material.
* No claim "audited" / "mainnet-ready" / "production-grade".
* No source-code modification.
* No `.env` edit.
* No private file edit.
* No chain transactions.
* No DB writes (unless the docs-publishing flow exposes a backend admin endpoint that writes — out of scope here).

---

## 4. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` shows BS-1 through BS-6 all CLOSED / CONFIRMED | read |
| P3 | `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md` exists and reports `status=reconciled` for tx `0x748c9484…` | read |
| P4 | No mainnet RPC URL appears in any of the new docs (grep on draft → mainnet keywords) | grep |
| P5 | `.env` mtime unchanged | `stat -c '%y'` |
| P6 | Private file mode + mtime unchanged | `stat` |

---

## 5. Hard stops

* Any new doc that contains the word "audited" or "mainnet-ready" without an explicit "NOT" prefix.
* Any URL or chain-id reference to mainnet (`8453`).
* Any private key, RPC URL, or DB credential in any doc.
* Any sequence that requires a chain transaction.

---

## 6. Execution sequence

```
6.0 Preflight (P1 .. P6)
6.1 Draft each public-beta doc from the existing internal docs
6.2 Cross-link from existing docs to new ones
6.3 Validations
6.4 Optional: publish to docs site (out of this brief's scope)
```

---

## 7. Acceptance criteria

* All 7 new docs in §2 created and self-consistent.
* `~/DEOPT/TESTNET_RUNBOOK.md` refreshed with retargeted addresses (or a new `TESTNET_RUNBOOK_V2.md` published alongside; operator picks).
* `RUN_STATE.md` closure paragraph added.
* `git diff --check` clean.
* Sensitive-string scan clean.
* `.env` and private file untouched.
* No source code changed.
* No mainnet / audit / production wording in any new doc.

---

## 8. Cross-links

* `docs/SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`
* `docs/SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`
* `docs/SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/openapi/trading-api.openapi.json`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of public-beta-docs-pack next-task brief.**
