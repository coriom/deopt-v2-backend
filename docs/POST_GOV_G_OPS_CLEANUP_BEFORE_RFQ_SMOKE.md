# Post-GOV-G operational cleanup — before RFQ live smoke

**Posture:** read+write operational cleanup; **no chain mutation,
no broadcast, no governance touch.** Two scoped operational
improvements landed; a third (broader DB cleanup of legacy perp
fixtures) deferred for explicit operator authorisation.

**Date (UTC):** 2026-06-08

**Scope:** Base Sepolia (chain `84532`) only. No mainnet.

---

## 1. Baseline pre-cleanup

| Subject | Value |
|---|---|
| chain_id | `84532` |
| `/executor/status` | live-capable (executionEnabled=true, dryRun=true, realBroadcastEnabled=true, broadcastEnabled=true, simulationEnabled=true, rpcConfigured=true) |
| Timelock.owner / pendingOwner / guardian | OPS_MULTISIG / 0x0 / OPS_MULTISIG |
| Timelock.proposers/executors(DEPLOYER) | false / false (GOV-G G-5 strip applied) |
| NEW_OME.paused | false |
| NEW_OME.isExecutor(BACKEND_EXECUTOR) / (DEPLOYER) | true / false |
| BE.nonce | 1 (first live smoke counted) |
| R5 drift | 0 |
| First-live-smoke intent `f9e765c2-…` status | `broadcast_submitted` (pre-cleanup — confirmation worker disabled) |
| `OPTION_CONFIRMATION_WORKER_ENABLED` env | UNSET (default false) |
| Backend log noise | ~1/s `executor dry-run tick failed error=execution intent is missing PerpTrade metadata: buyer_is_maker` |

---

## 2. Phase A — chain/governance safety re-verify

All probes GREEN. GOV-G closure intact: DEPLOYER fully retired
from Timelock proposer/executor/owner/guardian. NEW_OME and
downstream protocol state unchanged.

---

## 3. Phase B — enable option confirmation worker

### 3.1 Source analysis

| Source | Finding |
|---|---|
| `src/options/confirmation_worker.rs:5-52` | `OptionConfirmationConfig` struct; `validate_startup` requires persistence + RPC if `require_rpc=true`. |
| `src/config/env.rs:352-363` | env keys: `OPTION_CONFIRMATION_WORKER_ENABLED` (default `false`), `OPTION_CONFIRMATION_POLL_INTERVAL_MS` (`15000`), `OPTION_CONFIRMATION_FINALITY_BLOCKS` (`3`), `OPTION_CONFIRMATION_BATCH_SIZE` (`25`), `OPTION_CONFIRMATION_REQUIRE_RPC` (`true`). |
| `src/main.rs:109` | `spawn_option_confirmation_worker(state.clone())` — unconditional spawn; worker itself no-ops if `state.option_confirmation_config.enabled = false`. |
| `src/options/confirmation_worker.rs:116-154` | Worker behavior: spawns tokio task, polls `confirm_pending_option_execution_transactions` every `poll_interval_ms`, **read-only chain queries (`eth_getTransactionReceipt`, `eth_blockNumber`)**, never broadcasts. |

**Safety verdict:** worker is broadcast-safe. Enabling adds
read-only chain queries at 15-second cadence + DB updates to
intent.status transitions. No broadcast surface touched.

### 3.2 Env edit

- Backed up `.env` → `.env.bak.ops_cleanup.20260608T140556Z` (mode 0600).
- Appended single line: `OPTION_CONFIRMATION_WORKER_ENABLED=true`
- Diff vs backup: `+1` line, `0` modifications.
- All other env keys unchanged (verified via diff on sorted non-comment lines).

### 3.3 Restart

- Graceful TERM → 2-second exit → fresh nohup spawn.
- `/executor/status` post-restart: all 6 broadcast-capability flags green (unchanged).
- Startup log banner shows `option_confirmation_worker_enabled=true` ✓ (per `src/main.rs:121`).

### 3.4 Outcome

| Check | Result |
|---|---|
| Worker spawn | ✓ confirmed via tracing `INFO option confirmation worker tick batch=1 current_block=42579693` |
| First-live-smoke intent `f9e765c2-…` status | **`broadcast_submitted` → `broadcast_confirmed`** ✓ |
| Worker tick error | none observed |

**Phase B status: CLOSED GREEN.** The intent lifecycle gap noted
in `FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md` §11.1
is now resolved for future smokes.

---

## 4. Phase C — legacy perp dry-run runner noise

### 4.1 Root cause

Source path traced in FX-Q1-C-VERIFY-AFTER-FLAG-FLIP §"Source-level
root cause":
- `src/execution/runner.rs:7-19` spawns the perp dry-run loop.
- `src/execution/executor.rs:50-118` polls
  `execution_intents WHERE status='pending'` and calls
  `perp_trade_payload()` per row.
- `src/execution/intent.rs:38-67` requires `buyer_is_maker`,
  `buyer_nonce`, `seller_nonce`, `deadline_ms` to be non-null;
  otherwise emits `BackendError::MissingExecutionMetadata`.

The error logged hits `runner.rs:14` and the `?` operator
propagates without state mutation, so the offending row stays
`pending` and the loop re-errors every poll interval.

Spawn gate at `src/main.rs:95`:
```rust
if config.execution.execution_enabled && config.execution.dry_run {
    spawn_executor(...);
}
```
Both required and currently both true; cannot be disabled without
also disabling the option-execution broadcast surface (env.rs:551
requires `OPTION_EXECUTION_BROADCAST_ENABLED=true` implies
`EXECUTION_ENABLED=true`).

Per `executor.rs:54-58`, the perp scaffold has a hard stop on
real broadcast: `"real on-chain execution is not implemented yet"`.
**The runner cannot broadcast any tx**, regardless of upstream env
flags. The noise is log-only.

### 4.2 DB inventory

5 rows in `execution_intents` with `status='pending'`, all clearly
from pre-V2 dev/test sessions:

| intent_id | buyer | seller | buyer_is_maker | buyer_nonce | deadline_ms | created_at_ms |
|---|---|---|---|---|---|---|
| `eb4c9a2e-…` | `0xtaker` (placeholder) | `0xmaker` (placeholder) | **NULL** ← cleanup target | NULL | NULL | 1777713479073 |
| `7d0d6748-…` | `0xtaker2` | `0xmaker2` | f | 4001 | 4102444800000 | 1777752278278 |
| `c9981296-…` | `0x…0b03` (test addr pattern) | `0x…0a03` | f | 10001 | 4102444800000 | 1777806835740 |
| `60bde78e-…` | `0xc0A76c…cc976` (older dev fixture) | `0xbAf0976…1B1c3` | f | 2 | 1778268190427 | 1778268180473 |
| `dbecb9b8-…` | `0xc0A76c…cc976` | `0xbAf0976…1B1c3` | f | 3 | 1778322188818 | 1778322178900 |

**Confirmed:** none of these are in `option_execution_intents`
(separate table; verified via `src/db/repository.rs:52, 65, 1264,
2155, 2210` — perp and option intents live in independent tables).

### 4.3 Cleanup action — single row only (per scoped authorisation)

Backed up the single orphan row matching the originally specified
condition (`status='pending' AND buyer_is_maker IS NULL`) to
`/tmp/orphan_execution_intent_20260608T140938Z.json` (447 bytes).

Executed strict-WHERE UPDATE:
```sql
UPDATE execution_intents
   SET status = 'failed', updated_at_ms = <now_ms>
 WHERE intent_id = 'eb4c9a2e-f56a-4040-b97f-31c55cf87d92'
   AND status = 'pending'
   AND buyer_is_maker IS NULL
   AND buyer = '0xtaker'
   AND seller = '0xmaker';
```
Result: `UPDATE 1` (exactly one row mutated; idempotent if re-run).

### 4.4 Observed outcome — noise shifted, not eliminated

- `PerpTrade metadata: buyer_is_maker` error stopped at `14:09:57Z`
  (immediately after UPDATE landed).
- New error pattern emerged at `14:10:51Z`:
  `executor dry-run tick failed error=malformed account address`
- Cause: the runner advanced to the next `pending` row
  (`7d0d6748-…`), whose `buyer="0xtaker2"` is a non-address string
  that fails address parsing.

The 4 remaining `pending` rows are all the same class of legacy
perp fixtures from pre-V2 dev sessions. Broader cleanup would
silence the log noise entirely.

### 4.5 Broader cleanup — DEFERRED pending operator authorisation

**Scope expansion request:** UPDATE the 4 remaining `pending` rows
in `execution_intents` to `status='failed'`. Each row would be
backed up to JSON first. None of them are referenced from
`option_execution_intents` or any post-V2 path.

Recommended single SQL (operator-authorisable; agent will execute
in a separate task if approved):

```sql
-- Backup first
COPY (SELECT row_to_json(t) FROM (
  SELECT * FROM execution_intents
   WHERE status = 'pending'
     AND intent_id IN (
       '7d0d6748-7418-4192-b7c9-5d1906003928',
       'c9981296-ef77-4076-87a8-2b712eb1b982',
       '60bde78e-df4f-4e3a-b455-c65077100990',
       'dbecb9b8-54b8-48e3-90f8-5b7c76e52803'
     )
) t) TO '/tmp/orphan_execution_intents_full_<ts>.json';

-- Then mark failed (strict whitelist of 4 specific intent_ids)
UPDATE execution_intents
   SET status = 'failed', updated_at_ms = <now_ms>
 WHERE status = 'pending'
   AND intent_id IN (
     '7d0d6748-7418-4192-b7c9-5d1906003928',
     'c9981296-ef77-4076-87a8-2b712eb1b982',
     '60bde78e-df4f-4e3a-b455-c65077100990',
     'dbecb9b8-54b8-48e3-90f8-5b7c76e52803'
   );
```
Expected: `UPDATE 4`. After this, perp dry-run runner becomes
idle. Total backend log noise drops to zero non-PerpTrade /
non-malformed ERROR/WARN.

**Phase C status: PARTIAL — 1 of 5 orphan rows cleaned. Remaining
4 deferred for explicit operator authorisation. The new
"malformed account address" noise is structurally non-blocking:
same runner, same dry-run-only scaffold, same zero blast radius
on chain (cannot broadcast per `executor.rs:54-58`).**

### 4.6 Phase C extended — broader cleanup CLOSED (2026-06-08T14:20Z)

Authorised via `POST-GOV-G-LEGACY-PERP-DB-HOUSEKEEPING-COMPLETE`.

| Step | Action | Result |
|---|---|---|
| Backup | 4 rows → `/tmp/orphan_execution_intents_remaining_20260608T142021Z.json` (mode 0600, 2226 B) | ✓ 4 rows captured |
| FK reference check | `execution_intent_signatures`, `execution_simulations`, `execution_reconciliations`, `execution_transactions`, `rfqs.execution_intent_id` | UPDATE on `status` + `updated_at_ms` is non-cascading; safe |
| Transactional UPDATE | `BEGIN; UPDATE execution_intents SET status='failed', updated_at_ms=<ts> WHERE status='pending' AND intent_id IN (<4 IDs>); ... COMMIT;` | `UPDATE 4` ✓ |
| In-tx assertions (inside transaction) | `post_update_count_failed_targets = 4` ✓; `option_execution_intents_total = 10` (unchanged) ✓; `pending_count_post_update = 0` ✓ | all checks passed before COMMIT |
| Final `execution_intents` state | `confirmed=1, failed=5 (1 prior + 4 new), simulation_failed=2, pending=0` | ✓ |
| `option_execution_intents` count | `10` (untouched, identical to pre-cleanup) | ✓ |

**Backend log behaviour after COMMIT:**
- Last `malformed account address` error at `14:20:39Z` (pre-COMMIT).
- 30-second post-COMMIT log window: **0 new `PerpTrade metadata`, 0 new `malformed account address`, 0 new `executor dry-run tick failed` of any kind.**
- Perp dry-run runner is now polling an empty `pending` queue every poll interval; it computes a length-0 list and returns silently — no error path entered.
- Only periodic `INFO option confirmation worker tick batch=0 ...` log entries remain (Phase B worker; benign, expected, and confirms persistence is healthy).

**Targeted intent IDs (whitelist used in UPDATE):**
1. `7d0d6748-7418-4192-b7c9-5d1906003928`
2. `c9981296-ef77-4076-87a8-2b712eb1b982`
3. `60bde78e-df4f-4e3a-b455-c65077100990`
4. `dbecb9b8-54b8-48e3-90f8-5b7c76e52803`

**Phase C status: CLOSED GREEN.** All 5 originally-identified legacy
perp orphan rows in `execution_intents` are now `status='failed'`.
Backend log is clean. `option_execution_intents` was not touched.
R5 drift = 0 preserved.

---

## 5. Final R5 invariants (preserved)

| Subject | Value | Verdict |
|---|---|---|
| `PFV.feeBalance(mUSDC)` | `3028` | ✓ unchanged |
| `PFV.rebateReserve(mUSDC)` | `0` | ✓ unchanged |
| `CV(PFV, mUSDC)` | `3028` | ✓ unchanged |
| **drift** | **`0`** | ✓ preserved across ops cleanup |
| BE.nonce | `1` | ✓ unchanged (no broadcast by agent) |
| NEW_OME.paused | `false` | ✓ |
| Timelock state | OPS_MULTISIG owner/guardian; DEPLOYER stripped | ✓ |

---

## 6. Files touched by agent (this milestone)

| Path | Change |
|---|---|
| `~/DEOPT/deopt-v2-backend/.env` | added `OPTION_CONFIRMATION_WORKER_ENABLED=true` (single line; testnet scope) |
| `~/DEOPT/deopt-v2-backend/.env.bak.ops_cleanup.20260608T140556Z` | pre-edit backup of `.env`, mode 0600 |
| Postgres table `execution_intents` | 1 row UPDATE: `intent_id='eb4c9a2e-…'` set to `status='failed'` |
| `/tmp/orphan_execution_intent_20260608T140938Z.json` | full backup of mutated row before UPDATE |
| Backend process | graceful restart (TERM → spawn) |

**Forbidden touches confirmed NOT taken:** no chain tx; no
ownership/guardian/Timelock role mutation; no broadcast flag
edit; no signer flag edit; no `option_execution_intents` touch;
no source patch; no mainnet; no secret printed.

---

## 7. Validations

```
chain_id                                  : 84532 ✓
Timelock.owner / pendingOwner             : OPS_MULTISIG / 0x0 ✓
Timelock.proposers/executors(DEPLOYER)    : false / false ✓
NEW_OME.paused                            : false ✓
NEW_OME.isExecutor(BE) / (DEPLOYER)       : true / false ✓
R5 drift                                  : 0 ✓
BE.nonce                                  : 1 (unchanged) ✓
.env net delta                            : +1 line (OPTION_CONFIRMATION_WORKER_ENABLED=true) ✓
.env backup                               : .env.bak.ops_cleanup.20260608T140556Z ✓
backend restart                           : graceful TERM, fresh spawn, ready in 1s ✓
/executor/status post-restart             : unchanged broadcast capabilities ✓
option confirmation worker spawn          : observed ✓
first-live-smoke intent transition        : broadcast_submitted → broadcast_confirmed ✓
DB row UPDATE                             : exactly 1 row (intent_id='eb4c9a2e-…') ✓
DB row backup                             : /tmp/orphan_execution_intent_20260608T140938Z.json ✓
option_execution_intents table touched    : NONE ✓
PerpTrade noise stopped                   : ✓ (at 14:09:57Z post-UPDATE)
malformed account address noise           : new (non-blocking; broader cleanup deferred)
chain mutations by agent                  : 0 ✓
broadcast attempts by agent               : 0 ✓
secrets printed                           : 0 ✓
```

---

## 8. Blockers
None for this milestone. Phase B closed green; Phase C partial
(1/5 rows; broader cleanup deferred per scope). Backend is fully
operational and ready for RFQ live smoke.

## 9. Next milestones (each separately authorised)

1. **`POST_GOV_G_OPS_CLEANUP_PHASE_C_EXTENDED`** *(optional)* —
   broader DB cleanup of 4 remaining legacy `execution_intents`
   pending rows (see §4.5). Single SQL with whitelist of 4
   intent_ids. Silences perp dry-run runner noise completely.
2. **`FIRST_LIVE_SMOKE-RFQ-PRECHECK-SEPOLIA-FEE-ONLY`** —
   analogous fee-only smoke on the RFQ flow.
3. **Backend monitoring/alerts hardening** — PagerDuty + Discord
   wiring per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`. Lifts
   the `PRE-2 WAIVE-FOR-SEPOLIA` waiver. Mainnet-prerequisite.
4. **`V2G-GOV-G-G6-MIN-DELAY-BUMP`** — OPS_MULTISIG → Timelock
   `setMinDelay(259200)`. Pre-mainnet posture.
5. **Mainnet planning** — separate macro track.

**End of document.**
