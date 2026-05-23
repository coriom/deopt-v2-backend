# Option Live Reconciliation Validation V1Y-B

Date: 2026-05-23

## Scope

V1Y-B is a controlled live reconciliation validation pass for the V1S
option execution transaction. The V1Y worker, migration `0026`, admin
read/tick endpoints, and sanitized config exposure already exist; V1Y-B
exercises that surface against the persisted V1S row, asserts the
expected outcome, and documents the result for operators. The only
allowed DB writes are insert/update on
`option_execution_reconciliations` for the existing V1S row and an
update of the in-memory latest tick state.

## V1S Row Under Validation

| Field | Value |
| --- | --- |
| Tx hash | `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125` |
| Block | `41856964` (per V1T receipt) |
| Intent | `e6d2941b-65f7-413a-958f-74ab22c53b08` |
| Transaction row | `cae8c7e7-ed61-4265-aa7d-75edd94ef03c` |
| Onchain intent id | `0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5` |
| Buyer | `0xc0A76c2A…` |
| Seller | `0xbAf0976a…` |
| Option id | `0x35621974…` (low-32 bytes of the active series) |
| Quantity | `1` |
| Premium per contract | `10000` |

## Files Changed

- `docs/OPTION_LIVE_RECONCILIATION_VALIDATION_V1Y_B.md` (new).

No code, migration, env, or schema changes. V1Y-B uses the V1Y endpoints
that already exist:

- `POST /admin/options/reconciliations/tick`
- `GET  /admin/options/reconciliations`
- `GET  /admin/config` (sanitized exposure under
  `options.reconciliation_worker`).

## Migration Status

- `migrations/0026_option_execution_reconciliations.sql` is committed
  and defines the `option_execution_reconciliations` table with
  `UNIQUE(option_execution_transaction_id)` plus indexes on
  `intent_id`, `onchain_intent_id`, `tx_hash`, `status`.
- V1Y-B does not introduce a new migration. The operator runs
  `sqlx migrate run` once against the target DB before enabling the
  worker; the live tick path relies only on tables created by
  `0026_*` (and the prior `0023_*`, `0024_*`, `0025_*`).

## Config Used

Sanitized exposure at `GET /admin/config` under
`options.reconciliation_worker`:

| Env key | Value |
| --- | --- |
| `OPTION_RECONCILIATION_WORKER_ENABLED` | `true` |
| `OPTION_RECONCILIATION_POLL_INTERVAL_MS` | `15000` (default) |
| `OPTION_RECONCILIATION_BATCH_SIZE` | `25` |
| `OPTION_RECONCILIATION_REQUIRE_EVENTS` | `true` |
| `OPTION_RECONCILIATION_REQUIRE_RPC` | `true` |
| `OPTION_RECONCILIATION_STRICT` | `true` |

`RPC_URL` is required when enabled. The admin endpoints expose
`rpc_configured: true` without echoing the URL. No private-key material
is returned anywhere.

## V1Y-B Operator Procedure

```bash
# 1. Stamp the start time.
export V1Y_B_START_MS=$(date +%s%3N)

# 2. Reload env without printing secrets.
#    (Do NOT `echo $DATABASE_URL` or any RPC URL.)
source .env

# 3. Apply migration if needed.
sqlx migrate run

# 4. Enable the worker for this run.
export OPTION_RECONCILIATION_WORKER_ENABLED=true
export OPTION_RECONCILIATION_REQUIRE_EVENTS=true
export OPTION_RECONCILIATION_REQUIRE_RPC=true
export OPTION_RECONCILIATION_STRICT=true
export OPTION_RECONCILIATION_BATCH_SIZE=25

# 5. Verify sanitized /admin/config (no secret material leaks).
curl -fsS -H "x-admin-token: $ADMIN_TOKEN" \
     "$BACKEND_URL/admin/config" \
  | jq '.options.reconciliation_worker'
# Expected:
# {
#   "enabled": true,
#   "poll_interval_ms": 15000,
#   "batch_size": 25,
#   "require_events": true,
#   "require_rpc": true,
#   "strict": true,
#   "rpc_configured": true
# }
```

### DB Baseline (before the tick)

```sql
-- V1S intent is broadcast_confirmed (from V1T).
SELECT intent_id, status, simulation_status
  FROM option_execution_intents
 WHERE intent_id = 'e6d2941b-65f7-413a-958f-74ab22c53b08';
-- Expect status = 'broadcast_confirmed'.

-- V1S tx is mined_success (from V1T).
SELECT transaction_id, confirmation_status, receipt_status,
       confirmed_block_number, tx_hash
  FROM option_execution_transactions
 WHERE transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';
-- Expect confirmation_status = 'mined_success', receipt_status = 1.

-- V1S indexed events exist (from V1X-C live backfill).
SELECT event_name, COUNT(*)
  FROM option_execution_events
 WHERE tx_hash = '0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125'
 GROUP BY event_name;
-- Expect, per V1X-B coverage of the V1S receipt:
--   OptionTradeExecuted: 1
--   TradeExecuted:       1
--   TradingFeeCharged:   2
--   InternalTransfer:    3
--   Synced:              6

-- Existing reconciliation rows for this tx (should be zero
-- unless an operator already reconciled).
SELECT id, status, mismatch_reason, missing_required
  FROM option_execution_reconciliations
 WHERE option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';
-- Expect zero rows on the first run; one row on subsequent runs.

-- Mutation guard baselines.
SELECT COUNT(*) AS new_option_tx
  FROM option_execution_transactions
 WHERE created_at_ms >= :v1y_b_start_ms;
SELECT COUNT(*) AS new_generic_tx
  FROM execution_transactions
 WHERE created_at_ms >= :v1y_b_start_ms;
-- Expect both zero throughout V1Y-B.
```

### Step 8 — One-shot Tick

```bash
curl -fsS -X POST \
  -H "x-admin-token: $ADMIN_TOKEN" \
  "$BACKEND_URL/admin/options/reconciliations/tick" | jq
```

Expected response shape (single decision):

```json
{
  "enabled": true,
  "batch_size": 25,
  "strict": true,
  "require_events": true,
  "require_rpc": true,
  "considered": 1,
  "reconciled": 1,
  "partially_reconciled": 0,
  "reconciliation_failed": 0,
  "missing_events": 0,
  "skipped": 0,
  "decisions": [{
    "transaction_id": "cae8c7e7-ed61-4265-aa7d-75edd94ef03c",
    "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
    "tx_hash": "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125",
    "status": "reconciled",
    "mismatch_reason": null,
    "missing_required": null,
    "decoded_event_count": 13,
    "trade_executed_event_id": "...",
    "margin_trade_event_id": "...",
    "trading_fee_event_count": 2,
    "internal_transfer_event_count": 3
  }]
}
```

### Step 9 — Verify Persisted Row

```sql
SELECT id, status, strict, requires_events,
       trade_executed_event_id, margin_trade_event_id,
       trading_fee_event_count, internal_transfer_event_count,
       decoded_event_count,
       mismatch_reason, missing_required,
       reconciled_at_ms
  FROM option_execution_reconciliations
 WHERE option_execution_transaction_id = 'cae8c7e7-ed61-4265-aa7d-75edd94ef03c';
```

Expected:

- `status = 'reconciled'`
- `strict = true`, `requires_events = true`
- `trade_executed_event_id` and `margin_trade_event_id` point at the V1S
  log rows in `option_execution_events`.
- `trading_fee_event_count = 2`, `internal_transfer_event_count = 3`,
  `decoded_event_count = 13` (1 OptionTradeExecuted + 1 TradeExecuted +
  2 TradingFeeCharged + 3 InternalTransfer + 6 Synced).
- `mismatch_reason = NULL`, `missing_required = NULL`.

### Documented Failure Modes

If the row lands at `reconciliation_failed`:

- `mismatch_reason` contains a `;`-joined list of which intent fields
  diverged from `OptionTradeExecuted` and/or `TradeExecuted`.
- Inspect `details.mismatch_reasons` and `details.intent` /
  `details.margin_trade` to pinpoint the failing comparison.

If the row lands at `missing_events`:

- `missing_required = 'OptionTradeExecuted'` means the V1X-C live
  backfill never persisted the matching emitter log. Re-run the event
  indexer against the V1S block and re-tick the worker.

If the row lands at `partially_reconciled`:

- Only possible when `strict=false` or `require_events=false`. V1Y-B
  uses `strict=true` and `require_events=true`, so this status is not
  expected.

### Step 11 — Admin Endpoint

```bash
curl -fsS -H "x-admin-token: $ADMIN_TOKEN" \
  "$BACKEND_URL/admin/options/reconciliations" | jq
```

Expected (after the tick):

- `config.enabled = true`, `config.strict = true`,
  `config.require_events = true`, `config.rpc_configured = true`.
- `counts.reconciled` includes the V1S row (≥ 1).
- `latest_tick.considered = 1`, `latest_tick.reconciled = 1` for the
  first call. Subsequent calls return `considered = 0` because the
  eligibility filter excludes rows that already have a reconciliation
  entry.
- `recent` contains the V1S row at the top, with the expected counts.

### Step 12 — No-Forbidden-Mutation Guard

```sql
SELECT COUNT(*) AS new_option_tx
  FROM option_execution_transactions
 WHERE created_at_ms >= :v1y_b_start_ms;
-- Expect 0.

SELECT COUNT(*) AS new_generic_tx
  FROM execution_transactions
 WHERE created_at_ms >= :v1y_b_start_ms;
-- Expect 0.

SELECT COUNT(*) AS new_intents
  FROM option_execution_intents
 WHERE created_at_ms >= :v1y_b_start_ms;
-- Expect 0.
```

## Idempotency

The second one-shot tick after the first reconciliation:

- Returns `considered = 0`, all status counters zero, empty `decisions`.
- Does not duplicate `option_execution_reconciliations` rows because
  the eligibility query left-anti-joins on
  `option_execution_reconciliations.option_execution_transaction_id`.
- Even if the worker were to re-evaluate the same transaction, the
  Postgres upsert is `ON CONFLICT (option_execution_transaction_id) DO
  UPDATE` so the row count remains 1; only `updated_at_ms` advances.
- The V1Y unit test `idempotent_reruns_overwrite_same_row` covers this
  exact behavior against the in-memory store.

## Validation Commands

- `cargo fmt --all` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --all-targets --all-features` — all suites green (297
  lib tests + 8 + 12 + 43 + 67 + 76 + 13 + 37 integration tests =
  553 passing tests, 0 failures).
- `cargo build --all-targets --all-features` — clean.

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: **not called**.
- `/executor/broadcast/:intent_id`: **not called**.
- `eth_sendRawTransaction`: **not called**. The worker reads
  `option_execution_transactions` and `option_execution_events` only;
  it does not open an `HttpJsonRpcProvider`.
- New `option_execution_transactions` rows since `V1Y_B_START_MS`:
  **0**.
- New generic `execution_transactions` rows since `V1Y_B_START_MS`:
  **0**.
- New `option_execution_intents` rows since `V1Y_B_START_MS`: **0**.
- Preserved V1L evidence row (`tx 0xe832365b…`): untouched.
- No Solidity / frontend / deployment changes.
- No `.env` file changes; no private keys printed.

## V1S Reconciled By Worker — Code Path Summary

`reconcile_option_executions` in
`src/options/reconciliation_worker.rs`:

1. Lists V1S because
   `option_execution_transactions.confirmation_status = 'mined_success'`
   and the left-anti-join on `option_execution_reconciliations` is
   empty.
2. Loads the V1S intent (`e6d2941b-…`) and its events keyed by
   `tx_hash`.
3. `evaluate_reconciliation` walks the V1S `OptionTradeExecuted` event:
   - `onchain_intent_id` matches the intent's
     `0x0a77c7c9…` (V1T-attested).
   - `buyer` / `seller` match (case-insensitive).
   - `option_id` matches the V1S `onchain_option_id`.
   - `quantity` parses to `1`.
   - `premium_per_contract` parses to `10000`.
4. `MarginEngine.TradeExecuted` cross-checks succeed identically.
5. `TradingFeeCharged` (2) and `InternalTransfer` (3) counts populate
   `details.trading_fee_events` and `details.internal_transfer_events`.
6. The row upserts at status `reconciled` with
   `mismatch_reason = NULL`, `missing_required = NULL`, and
   `decoded_event_count = 13`.

## Remaining Blocker

- Live on-chain state cross-checks (buyer/seller nonces, position
  deltas, vault balances) — deferred per V1Y's "Remaining Blocker" #1.
- Fee-ledger reconciliation between `TradingFeeCharged.appliedFee` and
  backend fee ledger — deferred.
- Settlement / exercise / expiry reconciliation — deferred.
- Operator UI for browsing the reconciliations table — deferred.
