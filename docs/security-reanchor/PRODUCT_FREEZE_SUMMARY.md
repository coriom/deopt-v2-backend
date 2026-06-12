# DeOpt V2 — Product Freeze Summary

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready. This document is a frozen reference for the security re-anchor packet.

---

## 1. Solidity ABI freeze

**Tag:** `v2-product-freeze-rc1` (local; not pushed to a public release tag).
**Artefact directory:** `deopt-v2-sol/abis/freeze-v2-product-rc1/`.
**Source commit referenced:** `d133e2c` (per the freeze manifest).
**Status declared in manifest:** `TESTNET_BETA_ONLY_NOT_AUDITED_NOT_MAINNET_DEPLOYED`.

### Contracts in the freeze (11)

| Contract | Family | External fn count | Why frozen |
|---|---|---|---|
| `OptionProductRegistry` | Product / series | ~77 | Public API surface for product + series lookup. |
| `OptionMatchingEngine` | Matching | ~32 | EIP-712 two-sided trade entry point; signature replay protection lives here. |
| `MarginEngine` | Margin | ~81 | Per-account collateral accounting; authorizes the matching engine. |
| `CollateralVault` | Collateral | ~71 | Holds mUSDC; ERC-20 allowance-based deposit/withdraw. |
| `CollateralVaultViews` | Collateral | ~60 | Read-only views consumed by backend + frontend. |
| `FeesManagerV2` | Fees | ~32 | Buyer + seller fee split + fee-event emission. |
| `ProtocolFeeVault` | Fees | ~22 | Fee accrual + guarded sweep. |
| `OracleRouter` | Oracle | ~30 | `getPriceSafe(under, settle)` with `maxDelay = 60 s`. |
| `MarginEngineLens` | Lens | ~7 | Composed reads for backend convenience. |
| `InsuranceFund` | Core | ~45 | Insurance accounting. |
| `RiskModule` | Risk | ~61 | IM / MM calc + free-collateral math. |

Total: ~458 selectors across the 11 contracts (per `selectors.txt`).

### Frozen artefacts

* `freeze-manifest.json` — machine-readable summary.
* `selectors.txt` — every selector (4-byte) per contract, alphabetized.
* `storage-layouts.txt` — slot-by-slot layout pin. **Any future PR modifying an in-scope contract MUST re-run `forge inspect <Contract> storageLayout` and diff against this snapshot. A slot move = hard incompatibility.**
* One `.abi.json` per contract.
* `README.md` — handoff posture.

### Out of scope of the freeze

* `MockPriceSource` (test-only).
* `FeesManager.sol` V1 (superseded by V2).
* `PerpEngine*` family — perp surface deferred per `Q-CD-6` decision (NOT_APPLICABLE_AT_LAUNCH).
* No `*Legacy*` / `*Deprecated*` contracts present in `src/`.

### Cross-references

* `deopt-v2-sol/docs/SOL_PRODUCT_SCOPE_FREEZE_RESULT.md` — confirms the product surface was already complete before this milestone; the freeze anchors the artefact, it does NOT change source code.
* `deopt-v2-sol/docs/SOL_BACKEND_FRONTEND_ABI_HANDOFF.md` — names the 11 contracts and the view functions the UI + backend rely on.

### Open: Solidity test suite

`forge test` setup not located under `deopt-v2-sol/test/`. If a separate test tree exists it should be linked here. **Action:** `PRE_AUDIT_ACTION_PLAN.md` calls this out as a "documents test inventory" item before any audit dispatch.

---

## 2. Backend public API freeze

**Spec:** `deopt-v2-backend/docs/openapi/trading-api.openapi.json` (version `0.1.0-mvp`).
**Posture:** read-only or intent-creation only on the public surface. NO signer call. NO broadcast call. NO admin endpoints documented in the public spec.

### Public paths (13)

* `GET /options/products` (list)
* `GET /options/products/{product_id}`
* `GET /options/products/batch?ids=…`
* `GET /options/series/{series_id}/details`
* `GET /options/quotes/preview?series_id=…&side=…&size=…&price_1e8=…&account=…`
* `POST /options/quotes/preview` (alt body shape)
* `POST /options/exercise/preview` (read-only)
* `POST /options/close/preview` (read-only)
* `POST /options/execution-intents` (NEW in M-P2f; user-wallet, NO signer, NO broadcast)
* `GET /options/execution-intents/{intent_id}/signing-payload`
* `POST /options/execution-intents/{intent_id}/signatures`
* `GET /accounts/{address}/{positions|portfolio|balances|history}`
* `GET /trading/health`

### Admin paths

* **NOT in the public spec.** Live on a separate router gated by `ADMIN_API_ENABLED` + `ADMIN_API_REQUIRE_TOKEN` env knobs.
* Frontend never attaches an `Authorization` header to a public-path XHR (enforced by `tests/e2e/no-admin-bearer.spec.ts`).

### Execution / broadcast safety gates (runtime env)

| Env var | Default in this freeze | Effect when `false` |
|---|---|---|
| `EXECUTION_ENABLED` | `false` | All chain-touching paths short-circuit. |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `false` | Broadcast service refuses to send `executeTrade`. |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | `false` | Per-option broadcast paths refuse. |
| `OPTION_EVENT_INDEXER_ENABLED` | `true` | Indexer reconciles on-chain events. |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `true` | Backend catches up after a broadcast. |

Single approval-gated retry broadcast occurred 2026-06-12 (tx `0x748c9484…`); after that, `EXECUTOR_REAL_BROADCAST_ENABLED` returned to `false` for normal operation.

### Test coverage signal

~49 `mod tests` declarations across `deopt-v2-backend/src/` (unit tests distributed inline; no separate test tree). **Action:** `PRE_AUDIT_ACTION_PLAN.md` includes "publish backend test-inventory summary + coverage delta".

### Result docs that anchor this freeze

* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e) — wired 6 endpoints to read-only trading-views; added 5 optional `OPTION_*_ADDRESS` env keys.
* `BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` (M-P2f) — added the user-wallet `POST /options/execution-intents` endpoint with NO signer, NO broadcast.
* `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md` — post-broadcast catch-up + safe restart with broadcast disabled.
* `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md` — canonical first-trade tx + chain-side validation.

---

## 3. Frontend freeze

### Mainnet hard-stop (defence-in-depth)

* `src/lib/chains.ts::isMainnetEnabled()` is **hard-coded `false`**.
* `src/lib/chains.ts::expectedChainId()` defaults to `BASE_SEPOLIA` and **refuses to ever return mainnet's chain id**, even when the env says so.
* `src/lib/wallet.tsx::signTypedData()` returns `{ ok: false, reason: "wrong_network" }` when `isMainnet || !isExpectedChain`.
* `MainnetDisabledBanner` (sticky, full-width red) + `WrongNetworkBanner` (sticky, full-width amber) wired in `(trading)/layout.tsx`.
* `TradeTicket.canSign` requires `!isMainnet && isExpectedChain` in addition to the other gates.

### Public-beta link config

* `src/lib/public-beta-links.ts` — 6 slots, all currently `status: "placeholder"`, all hrefs `{{PUBLIC_BETA_*_URL}}`. `isPlaceholderHref()` degrades to "(coming soon)" spans.
* `pendingPlaceholderCount()` helper for operator tooling.
* Operator-fill checklist: `docs/public-beta/OPERATOR_PUBLIC_BETA_URLS_FILL.md`.

### No-admin-bearer guarantee

* `tests/e2e/no-admin-bearer.spec.ts` enforces zero `Authorization` headers from the app runtime on any route, on any navigation.
* Public-beta footer DOM scanned for bearer-shaped strings, RPC URLs with keys, DATABASE_URL, 64-char hex (private-key shape) — all forbidden.

### Build / test surface

* `npm run typecheck` — `tsc --noEmit` clean.
* `npm run lint` — clean.
* `npm run build` — green (Next 16; 9 routes).
* `npx playwright test --list` — 30 tests in 12 files; targeted execution requires `libnspr4.so` (WSL2 sandbox limitation; CI/Linux unaffected).

### Admin routes

* `src/app/admin/*` exists (3 files: `admin-dashboard.tsx`, `page.tsx`, `production-readiness-section.tsx`). Operator-visibility; NOT part of the public testnet beta tester flow.
* The "production-readiness" section is an operator dashboard — it discusses production readiness internally but the trading-UI surface never claims production / audited / mainnet-ready.

---

## 4. Public-beta docs freeze

**Location:** `deopt-v2-backend/docs/public-beta/`.
**File count:** 15 files (~2,481 lines).

### Core docs (8)

* `PUBLIC_TESTNET_BETA_OVERVIEW.md`
* `BASE_SEPOLIA_QUICKSTART.md`
* `USER_TESTING_GUIDE.md`
* `CONTRACT_ADDRESSES_BASE_SEPOLIA.md`
* `DEVELOPER_API_GUIDE.md`
* `KNOWN_LIMITATIONS_AND_RISKS.md`
* `FEEDBACK_AND_BUG_REPORTING.md`
* `FAQ.md`

### Community feedback loop docs (6, added 2026-06-12)

* `BUG_REPORT_TEMPLATE.md`
* `FEEDBACK_TRIAGE_WORKFLOW.md`
* `COMMUNITY_ONBOARDING.md`
* `PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md`
* `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`
* `OPERATOR_PUBLIC_BETA_URLS_FILL.md`

Plus `README.md` index.

### Canonical addresses (Base Sepolia, testnet only)

| Role | Address |
|---|---|
| `OptionMatchingEngine` (canonical) | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` |
| `MarginEngine` (canonical) | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` |
| `OptionProductRegistry` | `0x3d52b033fab00ed6104dd3bc0a715f8648344eca` |
| `CollateralVault` / `CollateralVaultViews` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| `OracleRouter` | `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` |
| `MarginEngineLens` | `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26` |
| `mUSDC` (test collateral, 6 decimals) | `0x6eAe407f5640B006faC9965182e238582A3B412E` |

Legacy stale `OptionMatchingEngine 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` — **DO NOT USE**, explicitly flagged in every contract-address-bearing doc.

### Canonical reference trade

* tx hash: `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`
* block: `42750521`
* status: `1` (success)
* events: `19` (per indexer normalization)
* explorer: `https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`

This is a **public** value (already on chain). Quoted only as a reference for what a healthy lifecycle looks like.

---

## 5. What is NOT frozen

* Source code in any of the three repos (this packet does not modify code).
* Backend Postgres schema beyond what's covered by the migration history.
* Mainnet contract addresses (there are none).
* Production signer / KMS / Safe-tx wiring (not built / not in scope here).
* Bug-bounty program rules (no bounty).
* External audit firm selection / SOW / outreach copy (deliberately separated; see `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md`).
* Operator-supplied real URLs for public-beta channels (placeholders intentionally retained).

---

## 6. Verifiability

An external reviewer can verify the freeze in under an hour:

1. `git -C deopt-v2-sol log --oneline | head -10` to find the source commit.
2. `cd deopt-v2-sol && forge inspect OptionMatchingEngine storageLayout | sha256sum` and compare against `abis/freeze-v2-product-rc1/storage-layouts.txt`.
3. `cast code 0x5a5EBF9A9CCd7c012518569DE8283982982670f6 --rpc-url <public-base-sepolia-RPC> | sha256sum` to confirm the on-chain bytecode matches the frozen artefact.
4. Open `https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a` to verify the canonical trade.
5. `cd deopt-v2-backend && cargo check` to confirm the codebase still compiles against the OpenAPI spec.
6. `cd deopt-v2-frontend && npm run typecheck && npm run lint && npm run build && npx playwright test --list` to confirm UI parity.

No mainnet RPC required for any step.

---

**End of product freeze summary.**
