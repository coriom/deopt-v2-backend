# DeOpt V2 — Product Readiness Roadmap

**Date:** 2026-06-10
**Posture:** docs-only planning milestone. **No chain mutation. No `.env`
edit. No mainnet. No Sepolia broadcast.**
**Strategic pivot:** previous trajectory pointed at immediate external audit
kickoff. **Now:** external audit / bug bounty / contest are deferred until
the platform is product-complete, testable end-to-end, and frozen for review.

## 1. Why this pivot

The audit-readiness layer already exists (kickoff finalisation, scope finals,
risk register, handoff index, outreach draft — `MAINNET_AUDIT_*_FINAL.md`).
Reviewing now would lock the auditor to an incomplete trading surface:

- frontend has only an admin dashboard;
- trading interface (wallet connect, market selector, option chain, trade
  ticket, positions table, exercise/close actions) is **not implemented**;
- public docs (README quickstart, testnet guide, user guide, market-maker
  guide, developer API docs) do not exist as a beta pack;
- end-to-end local + Sepolia trading lifecycle tests are not wired together
  into a single repeatable suite.

A pre-product-complete audit produces findings against a surface that will
change, then re-audit cost / churn. A post-product-complete audit pinpoints
the surface that will not change pre-mainnet.

The audit kickoff pack is preserved on disk and resumed at the freeze gate.

## 2. Three concentric goals

```
Goal 1 — sol product scope frozen:
        contracts ready as built; view-function surface complete for UI;
        events complete for indexer; test coverage gaps closed.

Goal 2 — backend trading API consolidated:
        every UI-required endpoint exists with stable response schema;
        wallet-side signature payloads are produced server-side; status +
        history + positions endpoints complete; admin endpoints kept
        separate.

Goal 3 — frontend trading MVP wired:
        wallet connect → market selector → option chain → trade ticket →
        positions table → exercise/close → transaction status → history.
        Local + Sepolia E2E green.
```

## 3. Roadmap (8 milestones)

```
M-P0  PRODUCT-READINESS-ROADMAP-AND-GAP-ANALYSIS         ← this milestone
M-P1  SOL-PRODUCT-SCOPE-FREEZE-AND-VIEW-FUNCTIONS
M-P2  BACKEND-TRADING-API-CONSOLIDATION
M-P3  FRONTEND-TRADING-MVP-WIRING
M-P4  E2E-LOCAL-TRADING-LIFECYCLE
M-P5  E2E-SEPOLIA-TRADING-LIFECYCLE
M-P6  PUBLIC-DOCS-BETA-PACK
M-P7  SECURITY-REVIEW-LATER-PACK    ← unlocks AUDIT-EXT-DISPATCH
```

Critical path: M-P1 → M-P2 → M-P3 → M-P4 → M-P5 → M-P6 → M-P7. M-P0 is
this docs-only milestone. Parallelism is described in
`NEXT_PRODUCT_MILESTONES.md §3`.

## 4. Out-of-scope (until freeze)

The following remain explicitly out of scope until M-P7 closure:

- External audit dispatch (handoff bundle is frozen post-M-P7);
- Bug bounty program;
- Code-4rena / Sherlock contest hosting;
- Mainnet contract deployment;
- AWS KMS production-key provisioning (rehearsal infra remains in place);
- V2G-Y mainnet ownership migration;
- Treasury / Insurance Operator Safe creation;
- Mainnet manifest fill;
- Custody Cluster 4 launch invariant verifier run on mainnet.

## 5. What we still preserve

- `MAINNET_AUDIT_*_FINAL.md` audit-side handoff layer (5 sol + 1 backend + 1 frontend).
- `MAINNET_KMS_VENDOR_SELECTION_DECISION.md` + `AWS_KMS_OPERATOR_SETUP_PACK.md` + 4 siblings.
- `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` + 5 siblings (preflight checklist, GO/NO-GO, next-safe milestones, missing-values table, next-task prompt).
- `MAINNET_SIGNER_*` runbooks + rehearsal plans.
- `MAINNET_CUSTODY_CLUSTER_*_RESOLUTION_REDACTED.md`.
- 4 custody cluster closures; OPS/GOV Safe roster anchor.

These are not deleted, not modified, and not invoked. They reactivate at
M-P7 closure when product freeze unlocks security review.

## 6. Strategic posture statements

- **"DeOpt is not yet audited."** This statement must appear in README, the testnet UI banner, and the developer docs front matter until M-P7 closure + external audit completes.
- **"DeOpt is a testnet beta."** Public-facing wording at M-P6 closure.
- **"Mainnet activation is gated on external audit completion + closure matrix sign-off."** Internal posture statement; no public mainnet timeline is set.

## 7. Risk acknowledgement

This pivot accepts the following risks:

| Risk | Mitigation |
|---|---|
| Findings discovered post-freeze require rework | Freeze scope precisely; auditor reads frozen commit; remediation cost bounded |
| Audit timeline (4-8 weeks active review + 4-6 weeks remediation) postponed | Product-complete trading surface is more valuable to the auditor than a partial surface |
| Trading interface bugs not caught in audit pre-freeze | M-P4 + M-P5 E2E lifecycle catches functional bugs; auditor focuses on security |
| Frontend XSS / RBAC / SSR issues caught only post-freeze | V2G-W3 SSR proxy + Strict CSP closure is M-P3 prerequisite, not deferred |

## 8. Sub-doc cross-links

- `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md` — per-layer gap inventory (this milestone)
- `TRADING_INTERFACE_REQUIREMENTS.md` — frontend trading UI requirements (this milestone)
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md` — local + Sepolia test plan (this milestone)
- `PUBLIC_DOCS_BETA_CHECKLIST.md` — public-doc plan (this milestone)
- `NEXT_PRODUCT_MILESTONES.md` — sequenced milestone DAG (this milestone)
- `SOL_PRODUCT_SCOPE_FREEZE_AND_VIEW_FUNCTIONS_NEXT_TASK.md` — M-P1 prompt (this milestone)

## 9. What this roadmap does NOT do

```text
- Does NOT modify source code
- Does NOT send any transaction
- Does NOT touch mainnet or Sepolia
- Does NOT invoke any Safe tx
- Does NOT cancel or supersede existing audit-side docs
- Does NOT claim DeOpt is audited
- Does NOT claim mainnet readiness
- Does NOT set a public mainnet timeline
- Does NOT change role assignments / Timelock / ownership
- Does NOT create AWS resources / KMS keys / IAM roles
```

**End of product readiness roadmap.**
