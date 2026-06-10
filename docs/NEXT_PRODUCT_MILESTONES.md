# DeOpt V2 — Next Product Milestones

**Date:** 2026-06-10
**Posture:** sequenced milestone DAG. **Docs-only**. Companion to
`PRODUCT_READINESS_ROADMAP.md`.

## 1. DAG

```
M-P0  PRODUCT-READINESS-ROADMAP-AND-GAP-ANALYSIS    ← THIS MILESTONE
        │
        ▼
M-P1  SOL-PRODUCT-SCOPE-FREEZE-AND-VIEW-FUNCTIONS
        │
        ▼
M-P2  BACKEND-TRADING-API-CONSOLIDATION
        │
        ▼
M-P3  FRONTEND-TRADING-MVP-WIRING ──────┐
        │                               ├─ partial parallelism with M-P2
        ▼                               │  (UI mocks against schema before backend lands)
M-P4  E2E-LOCAL-TRADING-LIFECYCLE       │
        │                               ▼
        ▼                       (V2G-W3 SSR proxy + Strict CSP closure runs in parallel)
M-P5  E2E-SEPOLIA-TRADING-LIFECYCLE
        │
        ▼
M-P6  PUBLIC-DOCS-BETA-PACK
        │
        ▼
M-P7  SECURITY-REVIEW-LATER-PACK  →  unlocks MAINNET-AUDIT-EXT-DISPATCH
                                     (handoff bundle frozen)
```

## 2. Per-milestone tables

### M-P1 — SOL-PRODUCT-SCOPE-FREEZE-AND-VIEW-FUNCTIONS

| Field | Value |
|---|---|
| Repo | `deopt-v2-sol/` |
| Owner | sol team |
| Prerequisites | M-P0 closed |
| Forbidden | mainnet tx, Sepolia broadcast, ownership change, guardian change, fee withdrawal, rebate allocation, custody mutation, source changes outside in-scope view-function additions, no introducing new state slots that break layout, no breaking ABI for existing public functions |
| Outputs | source diffs for view-function additions ONLY (no behavioural change); `SOL_PRODUCT_SCOPE_FREEZE_RESULT.md` listing: contracts frozen, view-function additions, event additions, ABI diff (additive only), test coverage delta; updated `TEST_MATRIX.md` |
| Validations | `forge fmt --check`; `forge build`; `forge test --no-match-path 'test/fork/*'`; `forge test --match-path 'test/scenario/*'`; storage-layout diff: no slot shifted; ABI diff: additive only; product-MVP lifecycle (create / quote / fill / position / exercise / close / settle) every view function present |
| Definition of done | every UI-required view function present + tested; freeze commit tagged `v2-product-freeze-rc1`; existing tests still green; no behavioural change |

### M-P2 — BACKEND-TRADING-API-CONSOLIDATION

| Field | Value |
|---|---|
| Repo | `deopt-v2-backend/` |
| Owner | backend team |
| Prerequisites | M-P1 closed; freeze commit tagged |
| Forbidden | mainnet tx, Sepolia broadcast, `.env` edit, AWS resource creation, KMS provisioning, source changes touching admin Bearer / SSR / production signer code paths, `RemoteSignerClient::new` modification, deletion of existing endpoints |
| Outputs | new endpoints per `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §3.2`; OpenAPI 3.1 export + JSON-schema bundle; updated DTO surface (`src/api/dto.rs` extended); 200+ new unit / integration tests; `BACKEND_TRADING_API_CONSOLIDATION_RESULT.md` |
| Validations | `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --all-targets --all-features`; OpenAPI schema lints clean; every new endpoint has positive + negative test case; cardinality policy preserved (no per-tx-hash, no per-user-address metric label) |
| Definition of done | every endpoint in §3.2 present; schemas documented; UI hooks can compile against the schema; existing routes unchanged; signer code path unchanged |

### M-P3 — FRONTEND-TRADING-MVP-WIRING

| Field | Value |
|---|---|
| Repo | `deopt-v2-frontend/` |
| Owner | frontend team |
| Prerequisites | M-P1 closed (ABI frozen); M-P2 closed OR schema published early (M-P3 can wire against schema with mocks while M-P2 lands implementations) |
| Forbidden | mainnet tx, Sepolia broadcast, admin Bearer reuse for trading calls, deletion of admin routes, `@web3modal/*` in admin path, `dangerouslySetInnerHTML` anywhere |
| Outputs | new `src/components/trading/`, `src/components/wallet/`, `src/components/tx/`, `src/components/ui/`; new `src/lib/trading-api.ts` + `src/lib/eip712.ts`; new routes per `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §4.6`; viem (+ optional wagmi) dep added; `FRONTEND_TRADING_MVP_RESULT.md` |
| Validations | `next build` clean; `eslint` clean; Playwright smoke (wallet connect → market list → trade ticket render → mock-broadcast); CI guards: `@web3modal/*` absent from admin path; `dangerouslySetInnerHTML` absent everywhere; `wagmi`/`viem`/`ethers` may NOW appear in trading path (guard scope narrowed to admin path) |
| Definition of done | UI renders the full lifecycle against a local mocked backend; tests pass; unaudited / testnet banners present on every page; V2G-W3 SSR proxy + Strict CSP closure also lands in M-P3 (admin) |

### M-P4 — E2E-LOCAL-TRADING-LIFECYCLE

| Field | Value |
|---|---|
| Repo | all three repos; harness lives in `deopt-v2-backend/tests/e2e/` |
| Owner | backend lead |
| Prerequisites | M-P1 + M-P2 + M-P3 closed |
| Forbidden | mainnet tx, Sepolia broadcast, real AWS, real KMS, `.env` edit beyond local-only fixtures, no real production EVM addresses |
| Outputs | E2E harness (docker-compose or local processes; anvil + postgres + backend + frontend); Playwright + viem test suite; `E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md`; CI workflow that runs the suite |
| Validations | every scenario in `E2E_TRADING_LIFECYCLE_TEST_PLAN.md §1` passes; R5 drift = 0; recovery scenarios converge to a single final state per intent |
| Definition of done | the full trading lifecycle (connect → deposit → trade → exercise → withdraw) runs green end-to-end on a clean anvil + backend + frontend launch; failure-case matrix § 6 covered |

### M-P5 — E2E-SEPOLIA-TRADING-LIFECYCLE

| Field | Value |
|---|---|
| Repo | all three repos; orchestration in `deopt-v2-backend/tests/e2e_sepolia/` |
| Owner | backend lead + sol lead |
| Prerequisites | M-P4 closed |
| Forbidden | mainnet tx, real AWS / KMS, `.env` edit beyond Sepolia rehearsal values, NEVER commit Sepolia private key, no operator-private addresses in tracked output |
| Outputs | Sepolia E2E suite; `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md`; R5 drift verification artefact |
| Validations | every scenario in `E2E_TRADING_LIFECYCLE_TEST_PLAN.md §2` passes; R5 drift = 0; CloudTrail-equivalent log of every Sepolia tx; tx hashes captured + linkable in the result doc |
| Definition of done | Sepolia full lifecycle green; R5 drift = 0 cumulative; result doc lists every tx hash + block; matches Sepolia rehearsal posture |

### M-P6 — PUBLIC-DOCS-BETA-PACK

| Field | Value |
|---|---|
| Repo | top-level `docs/` + per-repo `README.md` / `docs/` |
| Owner | docs lead |
| Prerequisites | M-P5 closed |
| Forbidden | mainnet timeline claim, "audited" claim, production EVM addresses, real AWS / KMS values, production RPC URL, admin Bearer in examples |
| Outputs | docs per `PUBLIC_DOCS_BETA_CHECKLIST.md §2` (14 docs); generated artefacts §4; hosting setup §5; `PUBLIC_DOCS_BETA_PACK_RESULT.md` |
| Validations | "not yet audited" banner on every page; testnet-only badge on every page; all cross-links resolve; no production EVM / no real AWS sweep clean; ABI references compile-frozen at the M-P6 closure commit |
| Definition of done | beta docs pack is ready to publish; bug-reporting channel live; "do not deposit real funds" wording present |

### M-P7 — SECURITY-REVIEW-LATER-PACK

| Field | Value |
|---|---|
| Repo | top-level + `deopt-v2-sol/docs/` (re-issues prior audit-side handoff pack against the freeze commit) |
| Owner | security lead |
| Prerequisites | M-P6 closed |
| Forbidden | mainnet tx, real AWS / KMS, "audit complete" claim, "audited" claim, secret material |
| Outputs | revised `MAINNET_AUDIT_EXT_KICKOFF_FINAL.md` against the M-P6 freeze commit; revised `MAINNET_AUDIT_CONTRACT_SCOPE_FINAL.md` against the M-P1 freeze; revised `MAINNET_AUDIT_BACKEND_SCOPE_FINAL.md` against M-P2 closure; revised `MAINNET_AUDIT_FRONTEND_ADMIN_SCOPE_FINAL.md` against M-P3 closure; revised `MAINNET_AUDIT_RISK_REGISTER_FINAL.md` with any product-freeze-driven adjustments; revised `MAINNET_AUDIT_HANDOFF_INDEX_FINAL.md`; revised `MAINNET_AUDIT_OUTREACH_DRAFT.md`; `SECURITY_REVIEW_LATER_PACK_RESULT.md` |
| Validations | "not yet audited" banner present until external audit completes; revised handoff bundle anchored to the freeze commit; sensitive-string sanity scan clean |
| Definition of done | external audit dispatch is now unblocked; the operator can send the outreach draft and ship the handoff bundle under NDA against the frozen freeze commit |

## 3. Parallelism

- M-P3 can wire against the M-P2 schema with mocks while backend implementations land; this halves the serial wait if schema is published early.
- V2G-W3 SSR proxy + Strict CSP closure happens within M-P3 admin scope.
- Public docs partial drafting (M-P6) can begin during M-P4 / M-P5 if the freeze commit is stable; final ship is at M-P6.
- M-P7 cannot start until M-P6 closure (the handoff bundle must reference the actual public docs surface).

## 4. Critical path

The longest serial path is **M-P1 → M-P2 → M-P3 → M-P4 → M-P5 → M-P6 → M-P7**.
- M-P1: 1-2 weeks (sol view-function additions + freeze).
- M-P2: 2-3 weeks (~10 new endpoints + schema + tests).
- M-P3: 3-4 weeks (frontend trading MVP from scratch + V2G-W3 closure).
- M-P4: 1-2 weeks (E2E harness + Playwright suite).
- M-P5: 1 week (Sepolia run + result doc).
- M-P6: 1 week (docs pack).
- M-P7: 1 week (handoff bundle re-anchor).

**Total: ~10-14 weeks to product-complete freeze + audit-ready dispatch.**

Audit (4-8 weeks active + 4-6 weeks remediation) follows post-M-P7.

## 5. What this DAG does NOT do

```text
- Does NOT modify source code in this milestone
- Does NOT touch mainnet or Sepolia
- Does NOT change AWS / KMS / signer posture
- Does NOT change Safes / Timelock / ownership
- Does NOT delete existing audit-side handoff docs (preserved + re-issued in M-P7)
- Does NOT commit to a mainnet timeline
- Does NOT claim DeOpt is audited or audit-ready (audit-ready = at M-P7 closure)
```

## 6. Cross-links

- `PRODUCT_READINESS_ROADMAP.md`
- `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `TRADING_INTERFACE_REQUIREMENTS.md`
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md`
- `PUBLIC_DOCS_BETA_CHECKLIST.md`
- `SOL_PRODUCT_SCOPE_FREEZE_AND_VIEW_FUNCTIONS_NEXT_TASK.md`
- `MAINNET_AUDIT_EXT_KICKOFF_FINAL.md` (preserved; re-issued at M-P7)
- `MAINNET_AUDIT_HANDOFF_INDEX_FINAL.md` (preserved; re-issued at M-P7)
- `MAINNET_NEXT_SAFE_MILESTONES.md` (separate; mainnet activation track; gated on M-P7 + audit closure)

**End of next product milestones.**
