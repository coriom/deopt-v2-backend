# E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT (M-P5 — Dry-Run Phase)

**Date:** 2026-06-10
**Milestone:** `E2E-SEPOLIA-TRADING-LIFECYCLE` (M-P5)
**Phase:** **DRY-RUN / READ-ONLY ONLY.** No Sepolia broadcast performed.
**Posture:** **No mainnet. No mainnet RPC. No mainnet config.
No live Sepolia tx. No Safe tx. No AWS / KMS calls. No `.env` edit.
No production secrets. No real funds movement.**

## 1. Purpose

M-P5 is a two-phase milestone:

* **Phase A (THIS DOC)** — dry-run / read-only verification that the
  stack is technically ready for Sepolia. **No broadcast performed.**
* **Phase B** — operator-approval-gated single Sepolia test
  broadcast. Brief at
  `E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` (if Phase A is fully
  green) OR at `E2E_SEPOLIA_FIXES_NEXT_TASK.md` (if blockers remain).

This document covers Phase A. Phase B has **not** been executed; a
separate operator sign-off and a separate run are required.

## 2. Repos / docs inspected

* `~/DEOPT/deopt-v2-{sol,backend,frontend}/**` (read-only)
* `~/DEOPT/RUN_STATE.md`, `~/DEOPT/TESTNET_RUNBOOK.md`
* `deopt-v2-backend/docs/E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK.md`
* `deopt-v2-backend/docs/{E2E_LOCAL_TRADING_LIFECYCLE_RESULT,E2E_LOCAL_AUTOMATION_RUNBOOK,E2E_LOCAL_TX_STATUS_CYCLER_RESULT,BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT,BACKEND_TRADING_API_PHASE_5_RESULT,FRONTEND_TRADING_API_HANDOFF}.md`
* `deopt-v2-backend/docs/openapi/trading-api.openapi.json`
* `deopt-v2-frontend/docs/{FRONTEND_CREATE_INTENT_UX_RESULT,TRADING_CREATE_INTENT_FLOW_RUNBOOK,FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_RESULT}.md`
* `deopt-v2-sol/docs/{MARGIN_ENGINE_V2_PHASE1_BROADCAST_AUTH_PACKET_V2D_O,MARGIN_ENGINE_V2_REWIRE_BROADCAST_RESULT_V2D_R}.md` (for known Sepolia addresses)
* `deopt-v2-sol/abis/freeze-v2-product-rc1/freeze-manifest.json`

## 3. Sepolia prerequisites inventory

### 3.1 Network

| Field | Value | Status |
|---|---|---|
| Chain id | `84532` | KNOWN |
| Network name | Base Sepolia | KNOWN |
| RPC URL | _operator-supplied_ (placeholder) | OPERATOR_INPUT_REQUIRED |

### 3.2 Contract addresses (Base Sepolia 84532)

The Solidity workspace already documents Sepolia deployment addresses
for the V2 option surface in the `MARGIN_ENGINE_V2_PHASE1_*` packet
docs. These are reproduced below from existing checked-in artefacts;
**no new addresses were invented in this milestone**.

| Field | Known Sepolia value (from existing sol/docs) | Status |
|---|---|---|
| OPTION_PRODUCT_REGISTRY | `0x3d52b033fab00ed6104dd3bc0a715f8648344eca` | KNOWN |
| OPTION_MATCHING_ENGINE | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` | KNOWN |
| OPTION_MARGIN_ENGINE | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (rewire target — operator confirms current binding) | OPERATOR_CONFIRMATION_REQUIRED |
| OPTION_COLLATERAL_VAULT | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` | KNOWN |
| OPTION_COLLATERAL_VAULT_VIEWS | _not in checked-in sol/docs_ | OPERATOR_INPUT_REQUIRED |
| OPTION_MARGIN_ENGINE_LENS | _not in checked-in sol/docs_ | OPERATOR_INPUT_REQUIRED |
| OPTION_ORACLE_ROUTER | `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` | KNOWN |
| FEES_MANAGER_V2 | `0x00dA0B…774f` (per `OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`) | OPERATOR_CONFIRMATION_REQUIRED |
| PROTOCOL_FEE_VAULT | _operator confirmation pending_ | OPERATOR_INPUT_REQUIRED |
| RiskModule | NOT_REQUIRED_FOR_DRY_RUN | NOT_REQUIRED_FOR_DRY_RUN |
| InsuranceFund | NOT_REQUIRED_FOR_DRY_RUN | NOT_REQUIRED_FOR_DRY_RUN |

### 3.3 Known accounts (from `~/DEOPT/TESTNET_RUNBOOK.md`)

| Role | Address | Status |
|---|---|---|
| Executor / deployer | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | KNOWN |
| Test buyer | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` | KNOWN |
| Test seller | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` | KNOWN |

### 3.4 Operator-confirmed gating items

| Item | Status |
|---|---|
| Backend executor authorised at OPTION_MATCHING_ENGINE | OPERATOR_CONFIRMATION_REQUIRED |
| Buyer + seller hold testnet ETH for gas | OPERATOR_CONFIRMATION_REQUIRED |
| Buyer + seller hold testnet collateral | OPERATOR_CONFIRMATION_REQUIRED |
| Series / product exists with active oracle feed | OPERATOR_CONFIRMATION_REQUIRED |
| R5 drift check runnable | KNOWN (`R5 drift = 0` constraint preserved per V2G-GOV-G) |
| Reconciliation worker operational | KNOWN (existing workers spawn on startup) |

## 4. Dry-run / read-only preflight

### 4.1 Backend posture verified

| Check | Result |
|---|---|
| `CHAIN_ID=84532` accepted by config loader | ✓ via existing `parse_env(..., "CHAIN_ID", "84532")` default |
| `EXECUTION_ENABLED=false` default | ✓ (`ExecutionConfig::execution_enabled` defaults `false`) |
| `EXECUTOR_DRY_RUN=true` default | ✓ (`ExecutionConfig::dry_run` defaults `true`) |
| `EXECUTOR_REAL_BROADCAST_ENABLED=false` default | ✓ |
| Mainnet refused by 4 gates (factory + runtime + admin gate + frontend banner) | ✓ unchanged from M-P4c |
| Trading-views env keys parse 0x-40-hex placeholders | ✓ verified by M-P2e tests (`trading_views_addresses_all_present_parsed_to_lowercase_canonical_form`) |
| Trading-views env keys reject malformed | ✓ verified by M-P2e tests |
| Public create-intent endpoint visible in OpenAPI | ✓ (M-P2f) |
| Local-test fixtures DISABLED by default on Sepolia | ✓ — `LocalTestFixturesConfig::disabled()` returns false; the factory refuses to enable for mainnet but ALSO does not auto-enable for Sepolia at startup |

### 4.2 Backend test sweep

| Suite | Result |
|---|---|
| `cargo build --lib` | exit 0 |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --no-deps --all-features -- -D warnings` | clean |
| `cargo test --all-targets --no-fail-fast` | **1203 passed** |

No broadcast worker, no signer call, no AWS / KMS call. The signer
adapter only activates when `EXECUTOR_REAL_BROADCAST_ENABLED=true` —
which the operator must explicitly enable for Phase B.

### 4.3 Frontend test sweep

| Check | Result |
|---|---|
| `npx tsc --noEmit` | exit 0 |
| `npx eslint src/ tests/` | exit 0 |
| `npx next build` | exit 0 (9 routes) |
| `npx playwright test --list` | 21 specs across 10 files |
| `npx playwright test` (browser run) | NOT RUN — operator-side `npm run e2e:install` required first |

### 4.4 Local-to-Sepolia frontend pointing

The frontend points at Sepolia via `NEXT_PUBLIC_TRADING_API_BASE_URL`
pointing at a backend running with `CHAIN_ID=84532`. **No `.env`
edit is performed by this milestone**; the operator supplies the URL
at backend startup. No production secrets, no mainnet config.

### 4.5 Create-intent / signing-payload status

* `POST /options/execution-intents` — wired in M-P2f. Returns
  `{intent_id, status: "signatures_required", ...}` for valid input.
  **No broadcast.**
* `GET /options/execution-intents/:id/signing-payload` — existing
  M-P3b endpoint. Returns the EIP-712 envelope for the intent.
* `POST /options/execution-intents/:id/signatures` — stores
  signature only; **broadcast is operator-side and gated by
  `EXECUTOR_REAL_BROADCAST_ENABLED`**.

In dry-run mode (default), submitting a signature does NOT trigger a
broadcast. The operator must explicitly flip the flag and approve
the run.

### 4.6 R5 / event / indexer / reconciliation

* **R5 drift** — must remain 0 at Phase B start (V2G-GOV-G
  invariant). Measurable via the existing reconciliation worker
  output; the worker is already wired and tested.
* **Event indexer** — `OptionEventIndexer` workers spawn on startup
  via `spawn_option_event_indexer(state)`; read-only against the
  RPC.
* **Reconciliation worker** — `OptionReconciliationWorker` spawns
  via `spawn_option_reconciliation_worker(state)`; read-only.
* **Confirmation worker** — `OptionConfirmationWorker` spawns; only
  polls receipts, does not broadcast.

All workers are existing infrastructure tested in 1203 backend tests.

## 5. Hard stops triggered during dry-run

None. The dry-run completed cleanly. The Phase B (live broadcast)
preconditions are documented in the approval-gate doc — execution
**requires** explicit operator sign-off and is **not** performed by
this milestone.

## 6. Live approval gate

See `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`. The gate requires:

1. Operator confirms all OPERATOR_INPUT_REQUIRED / OPERATOR_CONFIRMATION_REQUIRED
   rows in §3.
2. Operator confirms `EXECUTOR_REAL_BROADCAST_ENABLED=true` is set
   in the Sepolia-only backend env (NEVER mainnet).
3. Operator confirms `R5 drift = 0` at run start.
4. Operator types the literal approval line:
   `I approve one Base Sepolia test broadcast for this run.`
5. Maximum one broadcast per approval. The approval expires after
   the run completes (or after 4h if not used).

## 7. Docs created

* `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (this doc)
* `docs/E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_FIXES_NEXT_TASK.md` (because B-1 / B-2 / B-3
  in §3.2 remain open — see blockers doc)

## 8. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — M-P5-A closure paragraph
prepended.

## 9. Files changed

| Path | Status |
|---|---|
| `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` | new |
| `docs/E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md` | new |
| `docs/E2E_SEPOLIA_BLOCKERS_AND_FIXES.md` | new |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | new |
| `docs/E2E_SEPOLIA_FIXES_NEXT_TASK.md` | new |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

**No source code changed.** Phase A is docs + validation only.

## 10. Validations

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --no-deps --all-features -- -D warnings` | clean |
| `cargo test --all-targets --no-fail-fast` | 1203 passed |
| `npx tsc --noEmit` | exit 0 |
| `npx eslint src/ tests/` | exit 0 |
| `npx next build` | exit 0 |
| `npx playwright test --list` | 21 specs |
| `python3 -m json.tool docs/openapi/trading-api.openapi.json` | valid JSON |
| `git diff --check` | clean |
| Sensitive-string scan on new docs | zero leaks |

## 11. Blockers

See `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`. Summary:

| Blocker | Severity | Phase B impact |
|---|---|---|
| BS-1 OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS unknown on Sepolia | Medium | UI will degrade to partial for balances/portfolio reads |
| BS-2 OPTION_MARGIN_ENGINE_LENS_ADDRESS unknown on Sepolia | Medium | UI will degrade to partial for portfolio/positions/exercise reads |
| BS-3 Executor authorisation at OPTION_MATCHING_ENGINE not confirmed for Sepolia | High | Broadcast will revert — operator MUST confirm before Phase B |
| BS-4 Buyer / seller testnet collateral pre-funding not confirmed | High | Trade will fail at settlement — operator MUST top up |
| BS-5 Active series / product with live oracle feed not confirmed | High | Quote preview will return partial; create-intent may reject |

Blockers BS-1 / BS-2 are **soft** — the existing partial-fallback
path renders correctly. BS-3 / BS-4 / BS-5 are **hard** — must be
closed before Phase B.

## 12. Next milestone recommendation

**Recommended next:** `E2E_SEPOLIA_FIXES_NEXT_TASK` (closes BS-1 …
BS-5 with operator input). Once those are green, proceed to
`E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK` (Phase B, single
operator-approved Sepolia broadcast).

**Then:** M-P6 → M-P7 → MAINNET-AUDIT-EXT-DISPATCH (still ungated;
audit dispatch posture unchanged).

## 13. Cross-links

* `E2E_SEPOLIA_READ_ONLY_PREFLIGHT_RUNBOOK.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `E2E_SEPOLIA_FIXES_NEXT_TASK.md`
* `BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` (M-P2f)
* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e)
* `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (M-P4c)
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_CREATE_INTENT_UX_RESULT.md` (M-P3c)
* `~/DEOPT/TESTNET_RUNBOOK.md` (existing Sepolia ops runbook)

**End of M-P5 Phase A (dry-run) result.**
