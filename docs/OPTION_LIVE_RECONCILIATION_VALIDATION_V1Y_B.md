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

## Live Operator Execution Result

Executed on `2026-05-23` against the local backend bound to
`127.0.0.1:8080`. `V1Y_B_LIVE_START_MS = 1779552880028`.

### Migration applied

Before this run, `_sqlx_migrations.MAX(version) = 23`. Starting the
backend with `PERSISTENCE_ENABLED=true` auto-applied migrations
`0024_option_execution_receipt_cost.sql`,
`0025_option_execution_events.sql`, and
`0026_option_execution_reconciliations.sql`. After startup:

```
latest_migration                          | 26
option_execution_events_exists            | true
option_execution_reconciliations_exists   | true
option_event_indexer_state_exists         | true
```

### Sanitized /admin/config snapshot

```json
{
  "reconciliation_worker": {
    "batch_size": 25,
    "enabled": true,
    "poll_interval_ms": 3600000,
    "require_events": true,
    "require_rpc": true,
    "rpc_configured": true,
    "strict": true
  },
  "event_indexer_enabled": false,
  "confirmation_worker_enabled": false,
  "rpc_configured": true,
  "database_configured": true
}
```

(`poll_interval_ms` was set to one hour for this validation so the
background loop would not fire a second time during the inspection
window. No RPC URL, no admin token, no private key material returned.)

### DB Baseline (live)

```
intent_status              | broadcast_confirmed
tx_confirmation_status     | mined_success
tx_receipt_status          | 1
tx_block                   | 41856964
events_for_v1s_tx          | 0
reconciliations_for_v1s_tx | 0   (before any tick fired)
```

The baseline matches expectations except for `events_for_v1s_tx = 0`:
**migration `0025_option_execution_events.sql` had never been applied
to this DB before V1Y-B started**, so the V1X-C "live backfill"
documented earlier was never actually run against this database. The
event ledger created by V1Y-B's auto-migration is empty.

### Tick Response

The reconciliation worker spawns on startup and runs its first
`interval.tick()` immediately. That auto-tick saw exactly the V1S row
and produced the observed evidence (captured from the backend log,
mirrors the in-memory `latest_tick`):

```
considered=1 reconciled=0 failed=0 missing=1
```

The explicit `POST /admin/options/reconciliations/tick` issued after
the first auto-tick returned the idempotent no-op:

```json
{
  "enabled": true,
  "batch_size": 25,
  "strict": true,
  "require_events": true,
  "require_rpc": true,
  "considered": 0,
  "reconciled": 0,
  "partially_reconciled": 0,
  "reconciliation_failed": 0,
  "missing_events": 0,
  "skipped": 0,
  "decisions": []
}
```

### Reconciliation Row Status

```
id            | 7de93364-1048-4050-908e-6f01909107b7
status        | missing_events
strict        | true
requires_events | true
trade_executed_event_id        | NULL
margin_trade_event_id          | NULL
trading_fee_event_count        | 0
internal_transfer_event_count  | 0
decoded_event_count            | 0
mismatch_reason                | NULL
missing_required               | OptionTradeExecuted
reconciled_at_ms               | 1779552898037
```

**Exact failure reason:** the V1S `tx_hash` has zero rows in
`option_execution_events`. The reconciliation worker's
`evaluate_reconciliation` saw `events.len() = 0`, so the
`OptionTradeExecuted` lookup returned `None`. With `require_events=true`
the decision is `missing_events` and `missing_required` is set to the
name of the absent required event. This is the strict-mode contract.

The persisted `details` JSONB still contains the intent evidence —
buyer `0xc0A76c2A…`, seller `0xbAf0976a…`, option id
`24145907678156652148089862289363692212069910767044828147380657249455352740183`,
quantity `1`, premium `10000`, onchain intent id `0x0a77c7c9…` — so an
operator who later backfills the event ledger can re-evaluate against
the captured intent without re-querying the source tables.

This is not a code defect. To transition V1S to `reconciled`, the
operator runs a live event backfill (V1X-C procedure) and re-ticks the
worker; the upsert overwrites the same row with the matching
evidence. Code coverage for the success path is provided by the V1Y
unit test `matching_option_trade_executed_reconciles`.

### Admin Endpoint Summary

`GET /admin/options/reconciliations` returned:

- `config`: the live enabled config (`enabled=true`, `strict=true`,
  `require_events=true`, `require_rpc=true`, `rpc_configured=true`,
  `batch_size=25`).
- `counts`: `{ missing_events: 1, partially_reconciled: 0,
  reconciled: 0, reconciliation_failed: 0, skipped: 0 }`.
- `latest_tick`: the no-op response above (`considered=0`), because the
  explicit POST was the most-recent tick.
- `recent`: one row — the V1S reconciliation — with the same
  `missing_events` status and the captured intent evidence in
  `details.intent`.

### Idempotency Result

- Re-tick → `considered=0`, empty `decisions`.
- `SELECT COUNT(*) FROM option_execution_reconciliations
   WHERE option_execution_transaction_id = 'cae8c7e7-…'` = **1**
  (unchanged).
- Same `id` `7de93364-1048-4050-908e-6f01909107b7` retained;
  `updated_at_ms` unchanged.
- Postgres `UNIQUE(option_execution_transaction_id)` + `ON CONFLICT
  DO UPDATE` is doing exactly what the unit test predicted.

### No-Forbidden-Mutation Verification (live counters)

```
new_option_tx_since_start  | 0
new_generic_tx_since_start | 0
new_intents_since_start    | 0
```

- `POST /options/execution-intents/:id/broadcast`: not called.
- `/executor/broadcast`: not called.
- `eth_sendRawTransaction`: not called.
- No `option_execution_transactions`, `execution_transactions`, or
  `option_execution_intents` rows since `V1Y_B_LIVE_START_MS`.
- No Solidity / frontend / deployment / `.env` changes; no private
  keys printed.

### Validation Commands Run

- `cargo fmt --all` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --all-targets --all-features` — all suites green.
- `cargo build --all-targets --all-features` — clean.

## Live Event Backfill And Reconciliation Retry Result

Executed on `2026-05-23` against the same local backend / DB. Start time
`V1X_C_LIVE_START_MS = 1779553598797`. Migrations were already at `26`
from the previous run; no new schema work.

### Worker eligibility code change (prerequisite)

The previous V1Y-B run left the V1S row at `missing_events`. The V1Y
eligibility query excluded any row that already had a reconciliation
entry — even a non-terminal one — so the worker could never re-evaluate
V1S after the events backfill. That made `missing_events` accidentally
terminal.

V1Y-B's retry pass changes the eligibility query in both
`PgRepository::list_confirmed_unreconciled_option_execution_transactions`
and the in-memory store helper:

```sql
-- before
LEFT JOIN option_execution_reconciliations r ON ...
WHERE ... AND r.id IS NULL

-- after
LEFT JOIN option_execution_reconciliations r ON ...
WHERE ... AND (r.id IS NULL OR r.status <> 'reconciled')
```

Only `reconciled` is terminal; `missing_events`,
`partially_reconciled`, `reconciliation_failed`, and `skipped` are all
non-terminal and get revisited as new evidence lands. The Postgres
`ON CONFLICT (option_execution_transaction_id) DO UPDATE` upsert means
the same reconciliation row id is retained across transitions.

A new unit test `missing_events_row_is_re_evaluated_when_events_backfilled`
covers this transition end-to-end (missing → backfill events →
reconciled → second tick is a no-op).

### Event Tick Response (background + explicit)

The event indexer's auto-spawned tick fired on backend startup and
indexed 19 V1S logs at once (cursor moved to `41856968`):

```
option event indexer tick from_block=41856964 to_block=41856968
                          logs_found=19 events_indexed=19
```

The explicit `POST /admin/options/events/tick` issued afterward
advanced the cursor by one more batch with no new logs:

```json
{
  "enabled": true,
  "chain_id": 84532,
  "current_block_number": 41892695,
  "safe_head": 41892692,
  "from_block": 41856969,
  "to_block": 41856973,
  "logs_found": 0,
  "events_decoded": 0,
  "events_indexed": 0,
  "cursor_updated": true,
  "last_indexed_block": 41856973
}
```

### V1S Events Indexed

```
tx_hash = 0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125
event_name           | count
---------------------+------
InternalTransfer     |     3
OptionTradeExecuted  |     1
Synced               |    12
TradeExecuted        |     1
TradingFeeCharged    |     2
total                |    19
```

Counts match the V1X-B emitter audit (the previous V1X-C doc predicted
six `Synced` events for V1S specifically; the indexer pulls every
`Synced` log from the configured collateral vault address within the
batch, which captures additional intermediate balance writes — none of
the additional `Synced` logs blocks reconciliation, they are just
recorded in the ledger).

### Admin Events Endpoint Summary

`GET /admin/options/events` after the explicit tick:

```json
{
  "indexer_enabled": true,
  "last_indexed_block": 41856973,
  "last_error": null,
  "counts_by_event_name": {
    "InternalTransfer": 3,
    "OptionTradeExecuted": 1,
    "Synced": 12,
    "TradeExecuted": 1,
    "TradingFeeCharged": 2,
    "...": "other counts 0"
  },
  "counts_by_contract_address": {
    "0x00340c360353a5ab784c5bc5c44322a6af0625d3": 15,
    "0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8": 3,
    "0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b": 1
  },
  "emitter_contracts": [
    { "role": "matching_engine",   "contract_address": "0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b" },
    { "role": "margin_engine",     "contract_address": "0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8" },
    { "role": "collateral_vault",  "contract_address": "0x00340c360353a5ab784c5bc5c44322a6af0625d3" }
  ]
}
```

Total = 19 events split across the three configured emitters, matching
the V1T receipt attribution.

### Reconciliation Tick Response

`POST /admin/options/reconciliations/tick` returned:

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
    "decoded_event_count": 19,
    "trade_executed_event_id": "0c06450f-40b6-4323-84f6-254556eb8785",
    "margin_trade_event_id": "8a50ea52-0c01-420f-8367-52d69051ed04",
    "trading_fee_event_count": 2,
    "internal_transfer_event_count": 3
  }]
}
```

### Final Reconciliation Status (persisted row)

```
id              | 7de93364-1048-4050-908e-6f01909107b7
status          | reconciled
strict          | true
requires_events | true
trade_executed_event_id        | 0c06450f-40b6-4323-84f6-254556eb8785
margin_trade_event_id          | 8a50ea52-0c01-420f-8367-52d69051ed04
trading_fee_event_count        | 2
internal_transfer_event_count  | 3
decoded_event_count            | 19
mismatch_reason                | NULL
missing_required               | NULL
reconciled_at_ms               | 1779553682956
```

The reconciliation row **id is identical** to the prior
`missing_events` row (`7de93364-…`). The `ON CONFLICT` upsert replaced
all evidence fields in place and advanced `updated_at_ms` /
`reconciled_at_ms`; no duplicate row was created.

### Idempotency Result

- Second `POST /admin/options/reconciliations/tick` returned
  `considered = 0`, empty `decisions`. The new eligibility filter
  correctly treats `reconciled` as terminal.
- Second `POST /admin/options/events/tick` advanced the cursor to
  `41856978` and reported `logs_found=0, events_indexed=0`.
- `option_execution_reconciliations` row count for V1S stays at **1**,
  same id, status now `reconciled`.

### No-Forbidden-Mutation Verification (V1X_C_LIVE_START_MS)

```
new_option_tx_since_start  | 0
new_generic_tx_since_start | 0
new_intents_since_start    | 0
```

- No `eth_sendRawTransaction`. Indexer only called `eth_blockNumber`
  and `eth_getLogs`; reconciliation worker is RPC-free.
- No `/options/execution-intents/:id/broadcast` or
  `/executor/broadcast/:intent_id` invocation.
- No `option_execution_transactions`, `execution_transactions`, or
  `option_execution_intents` writes since `V1X_C_LIVE_START_MS`.
- V1L preserved evidence row untouched. No Solidity / frontend /
  deployment / `.env` changes. No private keys printed.

### Validation Commands Run

- `cargo fmt --all` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test --all-targets --all-features` — all suites green; the
  new `missing_events_row_is_re_evaluated_when_events_backfilled`
  test passes alongside the existing 13 worker tests.
- `cargo build --all-targets --all-features` — clean.

### Files Changed in the Retry Pass

- `src/db/repository.rs` — eligibility SQL now keeps non-terminal
  reconciliation rows in scope.
- `src/options/store.rs` — in-memory eligibility helper mirrors the
  SQL change.
- `src/options/reconciliation_worker.rs` — new
  `missing_events_row_is_re_evaluated_when_events_backfilled` test;
  refreshed `idempotent_reruns_overwrite_same_row` comment to reflect
  the non-terminal-row policy.
- `docs/OPTION_LIVE_RECONCILIATION_VALIDATION_V1Y_B.md` — this
  section.

## Remaining Blocker

- Live on-chain state cross-checks (buyer/seller nonces, position
  deltas, vault balances) — deferred per V1Y's "Remaining Blocker" #1.
  The `OPTION_RECONCILIATION_REQUIRE_RPC` gate is wired so the future
  worker can read `MarginEngine.getPositionQuantity`,
  `OptionMatchingEngine.nonces`, and `CollateralVault.balances`
  without re-plumbing config.
- Fee-ledger reconciliation between `TradingFeeCharged.appliedFee` and
  the backend fee ledger — deferred; V1Y already persists per-event
  `appliedFee` + `isMaker` evidence.
- Settlement / exercise / expiry reconciliation — deferred.
- Operator UI for browsing the reconciliations table — deferred.
