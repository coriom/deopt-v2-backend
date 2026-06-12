# SEPOLIA-POST-BROADCAST-BACKEND-RECONCILIATION — Result (BACKEND STARTUP SKIPPED, CHAIN + DB READ-ONLY VERIFIED)

> **CLOSURE (2026-06-12, later):** the deferred backend reconciliation
> work was completed by the follow-up milestone
> `SEPOLIA-BACKEND-RECONCILIATION-FIX`. Backend was rebuilt from
> source, started with broadcast hard-disabled, indexer caught up
> (cursor `42361398 → 42752194`), shadow intent + transaction
> backfilled, reconciliation worker tick succeeded with
> `status=reconciled` / `decoded_event_count=19` / fees + transfers
> matched. All API postchecks green. See
> `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`. The chain-side
> "skipped" rationale below is preserved for the historical record.


**Date:** 2026-06-12
**Milestone:** No-broadcast closure after `E2E_SEPOLIA_LIVE_BROADCAST_RETRY` success.
**Posture:** **No `cast send`. No `forge create`. No `forge script --broadcast`. No backend started. Zero on-chain state mutation. No mainnet. No `.env` edit. No private key / RPC URL / private-file contents printed.**

---

## 1. Outcome summary

| Phase | Status |
|---|---|
| A — global safety preflight | OK (chain id 84532, success tx receipt re-verified status `1`) |
| B — backend availability | **NOT RUNNING; STARTUP DEEMED UNSAFE — SKIPPED with documented rationale** |
| C — event indexer catch-up | SKIPPED (gated by Phase B) |
| D — confirmation + reconciliation | SKIPPED (gated by Phase B) |
| E — API postchecks | SKIPPED (gated by Phase B) |
| F — frontend smoke | SKIPPED (gated by Phase B) |
| G — docs | this doc + 4 cross-doc updates |

Chain-side success of tx `0x748c9484…` is fully verified via independent read-only `cast` calls; DB-side reconciliation is documented as a deferred follow-up.

---

## 2. Workspace

| | |
|---|---|
| Pack started | `2026-06-12T13:24:52Z` |
| Mode | read-only Sepolia + read-only DB |
| Approval line | none required — no broadcast in this milestone |

---

## 3. Global safety preflight (Phase A)

| Check | Result |
|---|---|
| `cast chain-id` | **`84532`** (Base Sepolia) |
| Mainnet hard-stop | NOT TRIGGERED |
| Success tx receipt for `0x748c9484…` | `status=1`, block `42750521`, gas `683_044`, 23 logs, from `0x295005fd…`, to `0x5a5EBF9A…` (correct ME) ✓ |
| `.env` (deopt-v2-backend) modified? | NO (`Jun  8 16:55:05` preserved) |
| Private file modified? | NO (mtime + mode 600 preserved) |
| Private file tracked? | NO |

Note: the `RETRY_RESULT.md` doc states "24 logs"; the canonical receipt JSON contains `23` logs (indices `0x6e..0x84` inclusive = 23). Off-by-one in the prior write-up; updated below.

---

## 4. Backend availability (Phase B) — SKIPPED with rationale

### 4.1 Detection

| Check | Result |
|---|---|
| Port `8080` (`/trading/health`, `/admin/recent`) | connection refused; not running |
| `pgrep` for `deopt`/`axum`/`cargo`/`target/release` patterns | no backend process |
| Postgres on `127.0.0.1:5432` | OPEN (running; key tables present) |
| Backend binary at `target/release/deopt-v2-backend` | PRESENT, **last built `2026-06-03 11:43:40Z`** |
| Backend `.env` chain id | `84532` ✓ |
| Backend `.env` `OPTION_MATCHING_ENGINE_ADDRESS` | **already** `0x5a5EBF9A…` (matches retarget) ✓ |
| Backend `.env` `EXECUTOR_FROM_ADDRESS` | `0x295005fd…` (matches EXECUTOR_ADDRESS) ✓ |

### 4.2 Decision: SKIP startup

Three concurrent risks made startup unsafe under this milestone's constraints:

1. **Binary built before the retarget** (`2026-06-03` ≪ `2026-06-12`). Source code is unchanged, but I have not verified that the old binary's executor / indexer paths handle the address swap cleanly without surprise behaviour on first run.
2. **`EXECUTOR_REAL_BROADCAST_ENABLED=true`** is set in `.env`. The brief allows runtime overrides via shell env (no `.env` edit), but the interaction between `EXECUTION_ENABLED=true`, `EXECUTOR_DRY_RUN=true`, and `EXECUTOR_REAL_BROADCAST_ENABLED=true` is non-trivial; an inadvertent broadcast attempt would breach the brief's hard "no broadcast" constraint. The cost of a breach is high; the cost of skipping is low (documented gap, scheduled in next-task).
3. **Indexer cursor is 389,123 blocks behind** (see §5). Catch-up at typical Base Sepolia block production rates (~2 s/block) means roughly **778k seconds of work** if scanned naively (≈ 9 days), or a forced cursor advance + targeted re-scan. Either path warrants a dedicated milestone with explicit batch-window configuration, not an ad-hoc startup here.

The brief explicitly anticipates this branch (Phase B step 4: "document SKIPPED with exact reason and continue with read-only chain verification").

### 4.3 Effect on later phases

Phases C, D, E, F are all gated by a running backend. Each is recorded as `SKIPPED_BACKEND_OFFLINE`. The follow-up task brief in §11 captures the work to close them.

---

## 5. Event indexer catch-up (Phase C) — SKIPPED_BACKEND_OFFLINE

### 5.1 DB state at start of milestone

| Table | Row count |
|---|---|
| `option_execution_intents` | 13 (from prior smokes; oldest buyers `0xc0A76…`, newer `0x394291…`) |
| `option_execution_events` | 193 (older Sepolia option events) |
| `option_execution_transactions` | 0 |
| `option_execution_reconciliations` | (not row-counted, but no row matches our tx) |
| `execution_transactions` | 1 (a single very old PERP tx `0x6b837e4c…`, block `41119866`) |
| `execution_reconciliations` | 1 (PERP) |
| `option_event_indexer_state.last_indexed_block` | **`42361398`** (updated_at_ms = `1780491090404` ≈ `2026-06-03`) |
| `indexer_cursors[perp_matching_engine]` | `41120901` |

### 5.2 Gap to close

| Tx | Block | Gap from indexer cursor |
|---|---|---|
| Successful retry executeTrade `0x748c9484…` | `42750521` | **+389,123 blocks** |
| Two oracle refresh setPrices (`0xcd0bb69b…`, `0x07cbe098…`) | `42750518` / `42750519` | (same gap; not normally indexed by option-event indexer) |
| The earlier reverted attempt `0xb4e7984f…` | `42749084` | (also not in DB) |

### 5.3 DB queries run (read-only)

```sql
SELECT count(*) FROM option_execution_transactions WHERE lower(tx_hash) = '0x748c94…';  -- 0
SELECT count(*) FROM execution_transactions        WHERE lower(tx_hash) = '0x748c94…';  -- 0
SELECT intent_id, buyer, seller, quantity_contracts FROM option_execution_intents
  WHERE onchain_intent_id = '0x8179a3fff8…';  -- 0 rows
```

Our successful tx is NOT in the backend's view of the world yet. Reconciliation is therefore deferred.

---

## 6. Confirmation + reconciliation (Phase D) — SKIPPED_BACKEND_OFFLINE

No confirmation / reconciliation worker tick was run (gated by Phase B). The `execution_reconciliations` schema's foreign key targets `indexed_perp_trades` (option reconciliation runs through a parallel table set — see §10 for the gap to close).

The on-chain `OptionTradeExecuted` event was verified independently via `cast receipt` (see §7); the DB-side projection of it is the deferred work.

---

## 7. Chain-side read-only verification (substitutes for missing backend run)

Re-issued via `cast` after the prior milestone to confirm idempotent state:

| Check | Result |
|---|---|
| `nonces(BUYER)` on correct ME | `3` (= `2 + 1`; consistent with the trade) ✓ |
| `nonces(SELLER)` on correct ME | `3` ✓ |
| `getPositionQuantity(BUYER, series#0)` on MarginEngine | `3` (long) ✓ |
| `getPositionQuantity(SELLER, series#0)` on MarginEngine | `-3` (short) ✓ |
| Buyer vault deposit | `99_998_994_750` (Δ from pre-broadcast `99_999_995_000` = `-1_000_250` = premium 1.0 + buyer-side fee) ✓ |
| Seller vault deposit | `100_040_998_950` (Δ from `100_039_999_000` = `+999_950` = premium 1.0 − seller-side fee) ✓ |
| Tx receipt | `status=1`, `gasUsed=683_044`, `effectiveGasPrice=6_000_000`, `transactionIndex=5`, `to=0x5a5EBF9A…` |
| Log count | **`23`** (correcting the prior `24`) |

All conservation laws hold. The chain-side reconciliation is mathematically clean; the DB-side projection is the only gap.

---

## 8. API postchecks (Phase E) — SKIPPED_BACKEND_OFFLINE

| Endpoint | Status |
|---|---|
| `GET /trading/health` | UNREACHABLE |
| `GET /options/products` | UNREACHABLE |
| `GET /options/series/:id/details` | UNREACHABLE |
| `GET /accounts/:buyer/balances` | UNREACHABLE |
| `GET /accounts/:buyer/portfolio` | UNREACHABLE |
| `GET /admin/recent` | UNREACHABLE |

---

## 9. Frontend smoke (Phase F) — SKIPPED

Gated by Phase E. Not run; no frontend process needed. The retry milestone already exercised the on-chain path via `cast`; the UI path is not part of M-P5 closure and was already deemed "off-scope" in the original brief.

---

## 10. Backend-integration gap (precise)

To close the skipped phases, a future milestone must:

1. **Rebuild backend binary** from `HEAD` (or confirm `2026-06-03` binary handles retarget cleanly).
2. **Start backend with broadcast hard-disabled**, e.g.:
   ```bash
   EXECUTION_ENABLED=false EXECUTOR_REAL_BROADCAST_ENABLED=false \
   EXECUTOR_DRY_RUN=true SIMULATION_ENABLED=false \
     /home/corio/DEOPT/deopt-v2-backend/target/release/deopt-v2-backend
   ```
   Verify at startup log that NO executor task is spawned and NO signer is loaded.
3. **Advance `option_event_indexer_state.last_indexed_block`** to a safe pre-tx block (e.g., `42750000`) via SQL, so the next indexer tick scans the tight window containing our tx (~520 blocks of ~17 minutes).
4. **Trigger a single indexer tick** via the admin endpoint (or wait for the periodic worker) and confirm:
   * `option_execution_events` contains a row with `tx_hash = '0x748c94…'`, `event_name = 'OptionTradeExecuted'`, `block_number = 42750521`.
   * `chain_id = 84532`, `contract_address = 0x5a5EBF9A…`.
5. **Backfill an `option_execution_intents` shadow row** that mirrors the off-chain tuple (`onchain_intent_id = 0x8179a3fff8…`, buyer / seller / qty=1 / premium=1_000_000 / nonces=2/2 / etc.), so the reconciliation worker can pair the event with an intent.
6. **Trigger reconciliation worker tick** and confirm a row in `option_execution_reconciliations` with `status = 'reconciled'`.
7. **Run the read-only API postchecks** in §8 and capture responses (sanitised).

The brief in §11 below packages this as the immediate next milestone.

---

## 11. Recommended next milestones

### 11.1 First: `SEPOLIA-BACKEND-RECONCILIATION-FIX` (created in §13)

Captures the §10 work in a single approval-gated brief. Required state-mutating action surface:
* DB writes from indexer / reconciliation workers (allowed by the original brief's Phase D/E surface).
* One SQL `UPDATE` to advance the indexer cursor.
* One SQL `INSERT` to backfill the shadow option-execution-intent.

**Forbidden in that follow-up:** chain transactions, oracle refresh, `executeTrade`, deploy, governance, mint, approve, transfer, `.env` edit, mainnet.

### 11.2 After backend reconciliation closes: `PUBLIC-BETA-DOCS-PACK`

The originally-recommended docs / UX work pack. Not blocked by the reconciliation gap (the chain-side success is already public-record), but it reads cleaner once the DB caught up.

---

## 12. Docs created / updated

| Path | Action |
|---|---|
| `docs/SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md` | NEW (this doc) |
| `docs/SEPOLIA_BACKEND_RECONCILIATION_FIX_NEXT_TASK.md` | NEW (§11.1 brief) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md` | edited (log-count correction `24 → 23`; backend-reconciliation status added) |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (backend-reconciliation row added) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | edited (DB inspection commands added) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (post-broadcast banner updated) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |
| Source code (`*.sol`, `*.rs`, `*.ts`, `*.tsx`) | UNCHANGED |
| `.env` (deopt-v2-backend) | UNCHANGED |
| Private file | UNCHANGED |
| Database | UNCHANGED (no INSERT / UPDATE / DELETE in this milestone — only SELECT) |

---

## 13. Files changed

**Source code: ZERO.** Only docs + RUN_STATE.

---

## 14. Validations

| Check | Result |
|---|---|
| `git diff --check` (backend) | clean |
| `git status --short` | only intended docs |
| Sensitive-string scan on new + edited docs | zero hits |
| `.env` modified? | NO (`Jun  8 16:55:05` preserved) |
| Private file modified? | NO (mtime + mode 600 preserved) |
| Private file committed? | NO |
| RPC URL printed? | NO |
| Private key printed? | NO |
| DATABASE_URL printed? | NO (presence-only) |
| Mainnet RPC used? | NO (chain id 84532) |
| Source code modified? | NO |
| `cast send` invoked? | NO |
| `forge create` / `forge script --broadcast` invoked? | NO |
| Backend started? | NO (skipped with documented rationale) |
| DB INSERT / UPDATE / DELETE invoked? | NO (read-only psql) |
| Safe / AWS / KMS / production signer | NO |

---

## 15. Remaining blockers

* **BS-1 / BS-2 / BS-3 / BS-4 / BS-5 / BS-6** — all CLOSED / CONFIRMED (unchanged).
* **M-P5 Phase B on-chain** — COMPLETE (unchanged).
* **Backend reconciliation projection** — OPEN; scoped in `SEPOLIA_BACKEND_RECONCILIATION_FIX_NEXT_TASK.md`.

The backend gap is a follow-up housekeeping item, not a blocker on the canonical Sepolia trade lifecycle.

---

## 16. Next milestone recommendation

`SEPOLIA-BACKEND-RECONCILIATION-FIX` (brief at `docs/SEPOLIA_BACKEND_RECONCILIATION_FIX_NEXT_TASK.md`), then `PUBLIC-BETA-DOCS-PACK` once the DB reconciliation is green.

Mainnet / audit / bug bounty remain explicitly out of scope.

---

## 17. Cross-links

* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`
* `docs/SEPOLIA_BACKEND_RECONCILIATION_FIX_NEXT_TASK.md`
* `docs/SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `docs/BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md`
* `docs/BACKEND_TRADING_API_PHASE_5_RESULT.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of post-broadcast reconciliation result.**
