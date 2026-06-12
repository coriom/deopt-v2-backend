# SEPOLIA-BACKEND-RECONCILIATION-FIX — Next Task Brief

**Date written:** 2026-06-12
**Origin:** `SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md` §10.
**Target:** project the on-chain success of tx `0x748c9484…` into the backend DB so the `option_execution_events` indexer + reconciliation worker show the trade as `reconciled`.
**Posture:** **DB writes + backend processes allowed. NEVER chain transactions. NEVER mainnet. NEVER Safe tx. NEVER AWS / KMS. NEVER production signer. NEVER `.env` edit. NEVER `executeTrade` / mint / approve / transfer / deploy / governance.**

> **This task is NOT executed by the calling milestone. The harness must require the literal operator approval line in §1 before any DB write or backend process start.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve Base Sepolia backend reconciliation fix for this run."

Properties:
* Authorises local backend startup with broadcast hard-disabled.
* Authorises local DB writes only by the indexer + reconciliation workers (and a one-shot SQL cursor-advance + one-shot SQL shadow-intent INSERT, both scoped to the success tx).
* Expires 4 hours after the approval is received.
* Does NOT authorise any on-chain action.

---

## 2. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `cast chain-id` returns `84532` | required first call |
| P3 | Postgres `127.0.0.1:5432/deopt_v2_backend` reachable | TCP probe |
| P4 | Backend binary exists at `target/release/deopt-v2-backend` (or freshly built) | `stat` |
| P5 | Backend `.env` `OPTION_MATCHING_ENGINE_ADDRESS = 0x5a5EBF9A…` | read-only `awk` |
| P6 | Backend `.env` `CHAIN_ID = 84532` | read-only `awk` |
| P7 | `.env` (`/home/corio/DEOPT/deopt-v2-backend/.env`) untouched | `stat -c '%y'` before vs after |
| P8 | Private file `~/DEOPT/private/operator-private/sepolia.inputs.private.env` untouched, mode `600` | `stat` |

If any precondition fails → STOP.

---

## 3. Hard stops

* Any `cast send`, `forge create`, or `forge script --broadcast` invocation.
* Backend startup without `EXECUTION_ENABLED=false` AND `EXECUTOR_REAL_BROADCAST_ENABLED=false` overrides in the launching shell env.
* Backend startup log shows any "executor started" / "broadcasting" line.
* Mainnet chain id `8453` anywhere in any output.
* Cursor advance beyond block `42750521 + 50` (constrain the rescan window).
* Shadow-intent INSERT with `quantity_contracts != '1'` or `premium_per_contract_native != '1000000'` (these must match the on-chain tuple exactly).

---

## 4. Execution sequence

```
4.0 Preflight (P1 .. P8)
4.1 Build (if needed) — `cargo build --release -p deopt-v2-backend`
4.2 Backend startup with overrides — executor disabled
4.3 Advance indexer cursor to safe block
4.4 Trigger indexer tick — confirm OptionTradeExecuted captured
4.5 Backfill shadow option_execution_intents row
4.6 Trigger reconciliation worker
4.7 API postchecks (read-only)
4.8 Frontend smoke (optional)
4.9 Docs + RUN_STATE
4.10 Final validations
```

### 4.1 Build (skip if recent)

```bash
cd /home/corio/DEOPT/deopt-v2-backend
cargo build --release -p deopt-v2-backend 2>&1 | tail -10
```

### 4.2 Backend startup with overrides

```bash
cd /home/corio/DEOPT/deopt-v2-backend
EXECUTION_ENABLED=false \
EXECUTOR_REAL_BROADCAST_ENABLED=false \
EXECUTOR_DRY_RUN=true \
SIMULATION_ENABLED=false \
  ./target/release/deopt-v2-backend > /tmp/deopt-backend.log 2>&1 &
BACKEND_PID=$!
sleep 5
# Verify startup
curl -sS --max-time 3 http://localhost:8080/trading/health
# Verify NO executor task was spawned
grep -iE 'executor|broadcast|signer' /tmp/deopt-backend.log | head -5
```

The harness MUST confirm in the startup log that:
* `EXECUTION_ENABLED=false` was respected (no executor task).
* `EXECUTOR_REAL_BROADCAST_ENABLED=false` was respected.
* No `signer init` line.

If any of those fail → kill the process and stop.

### 4.3 Advance indexer cursor

```bash
psql "$DATABASE_URL" -c "
UPDATE option_event_indexer_state
SET last_indexed_block = 42750000,
    updated_at_ms = (extract(epoch from now()) * 1000)::bigint
WHERE id = 'option_events_base_sepolia';
"
```

This advances by ~388k blocks and lets the next tick scan a tight ~520-block window.

### 4.4 Indexer tick

If the backend has an admin endpoint to force a tick, call it. Otherwise wait for the periodic worker.

Verify:

```bash
psql "$DATABASE_URL" -c "
SELECT tx_hash, event_name, block_number, log_index, onchain_intent_id
FROM option_execution_events
WHERE tx_hash = '0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a'
ORDER BY log_index;
"
```

Expect ≥ 1 row with `event_name = 'OptionTradeExecuted'` and `onchain_intent_id = '0x8179a3fff8…'`.

### 4.5 Backfill shadow intent

```sql
INSERT INTO option_execution_intents (
  intent_id, onchain_intent_id, source_type, source_id,
  option_series_id, onchain_option_id, buyer, seller,
  underlying, settlement_asset, expiry, strike_1e8, is_call,
  contract_size_1e8, quantity_contracts, source_size_1e8,
  source_price_1e8, premium_per_contract_native,
  buyer_is_maker, buyer_nonce, seller_nonce, deadline,
  …other required columns…
) VALUES (
  gen_random_uuid()::text,
  '0x8179a3fff83999d5525ad796691b7526664c6229935d30f14da7e729f8ce7de8',
  'shadow_offchain',
  'M-P5-Phase-B',
  '<series-id-text>', '<series-id-hex>',
  '0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC',
  '0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88',
  '0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02',
  '0x6eAe407f5640B006faC9965182e238582A3B412E',
  1893456000, '300000000000', true,
  '100000000', '1', '100000000',
  '300000000000', '1000000',
  false, '2', '2', <deadline_used_in_tuple>
);
```

Operator confirms the exact column list against the live schema (`\d option_execution_intents` shows 22+ columns; the brief here lists the load-bearing ones). One row only.

### 4.6 Reconciliation tick

Force a reconciliation tick (admin endpoint or wait). Verify:

```bash
psql "$DATABASE_URL" -c "
SELECT reconciliation_id, status, block_number, tx_hash
FROM option_execution_reconciliations
WHERE tx_hash = '0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a';
"
```

Expect 1 row with `status = 'reconciled'` (or equivalent terminal-success enum used by the codebase).

### 4.7 API postchecks (sanitised; no admin bearer in URL)

```bash
curl -sS http://localhost:8080/trading/health | jq .
curl -sS http://localhost:8080/options/products | jq '.data | length'
curl -sS "http://localhost:8080/options/series/<series_id>/details" | jq '.status, .warnings'
curl -sS "http://localhost:8080/accounts/0x394291A0…/balances" | jq '.status'
curl -sS "http://localhost:8080/accounts/0x394291A0…/portfolio" | jq '.status'
```

Record only `status`/length, never raw amounts.

### 4.8 Frontend (optional)

If a frontend dev process can be started safely, run `npm run dev` + Playwright `--list`. Do not run wallet-broadcast flow.

### 4.9 Docs

* New: `docs/SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`.
* Edit: cross-doc closure banners on `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`, `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`, `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`, `RUN_STATE.md`.

### 4.10 Validations

* `git diff --check` clean.
* `.env` mtime unchanged.
* Private file mtime + mode unchanged.
* No chain tx invoked.
* Sensitive-string scan: zero hits.
* `kill $BACKEND_PID` at the end (or leave running for further work — operator choice).

---

## 5. Scope — what the fix DOES

* 1 SQL UPDATE (cursor advance).
* 1 SQL INSERT (shadow option-execution-intent).
* Backend process: indexer + reconciliation workers (no executor).
* DB writes from those workers (events + reconciliation rows).
* Read-only API postchecks.

---

## 6. Scope — what the fix DOES NOT do

* No `cast send` of any kind.
* No `forge create` / `forge script --broadcast`.
* No mainnet RPC.
* No production signer / AWS / KMS.
* No Safe tx.
* No `.env` edit.
* No source-code change.
* No second on-chain broadcast retry.
* No additional series creation / governance.

---

## 7. Forbidden (whole fix)

* No mainnet (chain id `8453`).
* No Safe tx.
* No AWS / KMS creation.
* No production `.env` edit.
* No `.env.sepolia` commit.
* No private key in any log or doc.
* No raw `DATABASE_URL` printed.
* No second backfill INSERT under the same approval line.
* No skipping of any precheck.
* No claim "mainnet-ready" or "audited".

---

## 8. Acceptance criteria

* `option_execution_events` contains the `OptionTradeExecuted` row for tx `0x748c94…`.
* `option_execution_reconciliations` contains a `reconciled` row for tx `0x748c94…`.
* All API postchecks return `status: "ok"` (or `partial` with documented reason).
* `git diff --check` clean.
* `.env` and private file untouched.
* Backend process either shut down cleanly OR documented as left running.

---

## 9. Cross-links

* `docs/SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`
* `docs/SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/BACKEND_TRADING_API_PHASE_5_RESULT.md`
* `docs/BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of backend-reconciliation-fix next-task brief.**
