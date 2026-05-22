# Option Broadcast Gas Safety V1O

Backend safety patch that gates `POST /options/execution-intents/:intent_id/broadcast` behind a fresh `eth_estimateGas` preflight against the exact transaction the executor would sign. This task did not perform any broadcast, did not call `/executor/broadcast`, did not create `option_execution_transactions` or `execution_transactions` rows from a live run, and did not modify Solidity, frontend, or deployment.

## Original Failure Summary

The first live option execution broadcast on Base Sepolia produced tx `0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c`, which reverted on-chain with receipt `status=0` and `gasUsed=982941` against a transaction `gasLimit=1000000`.

Diagnosis (`docs/OPTION_FIRST_BROADCAST_FAILURE_0xe832365b.md`):

- The simulation was run with no gas cap because `OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`.
- The live broadcast used `gas_limit=1000000`, inherited from `EXECUTOR_MAX_GAS_LIMIT` because `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0`.
- `eth_estimateGas` for the same calldata returned `1040080`.
- `eth_call` at `gas=1000000` reverted; at `gas=1040080` (or no cap) succeeded.
- `cast run` showed top-level `EvmError: OutOfGas` inside seller-side risk computation.

## Root Cause

A `simulation_ok` produced under an uncapped `eth_call` does not prove that the broadcast — capped at a smaller, fixed `gas_limit` — has enough gas to complete. The two paths used inconsistent gas conditions.

## New Gas Safety Rules

Before any broadcast the backend now performs:

1. `eth_estimateGas` against the exact prepared transaction (`from = EXECUTOR_FROM_ADDRESS`, `to = OPTION_MATCHING_ENGINE_ADDRESS`, `data = intent.calldata`, `value = 0`). The same `from` that will sign and send the live tx is used; if it is not an authorized executor on-chain, estimation fails and broadcast is rejected.
2. Compute `required_gas = estimated_gas * OPTION_EXECUTION_GAS_SAFETY_BPS / 10000`. The bps field is conservative by default (see config).
3. Determine the effective broadcast cap: `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` when nonzero, else `EXECUTOR_MAX_GAS_LIMIT`. A resolved effective cap of `0` is rejected as "uncapped broadcast" — the backend refuses to submit a transaction with no gas limit.
4. Compare:
   - `broadcast_gas_limit < estimated_gas` → reject (`broadcast_cap_too_low`).
   - `estimated_gas <= broadcast_gas_limit < required_gas` → reject (`below_safety_margin`).
   - `broadcast_gas_limit >= required_gas` → proceed (`ok`).
5. Persist the gas-check outcome on the resulting `option_execution_transactions` row (`estimated_gas`, `required_gas`, `simulation_gas_limit`, `broadcast_gas_limit`, `gas_safety_bps`, `gas_check_status`, `gas_check_error`). Failing checks insert a `failed` transaction row and move the intent to `broadcast_failed`. No signing or send occurs when the check fails.

The preflight runs after `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` is satisfied. The legacy `simulation_ok` status alone is no longer sufficient to allow broadcast.

### Why uncapped simulation alone is insufficient

`eth_call` without a `gas` field uses the node's block gas limit, which is far above any per-tx `gas_limit` the backend will sign. A passing uncapped simulation says "the calldata can execute under arbitrarily large gas," not "the calldata fits within `OPTION_EXECUTION_BROADCAST_GAS_LIMIT`." `eth_estimateGas` returns the minimum gas the EVM observed during execution and is the load-bearing measurement; the backend then applies a safety multiplier to absorb future gas-cost variation (storage slot warmups, oracle re-fetches, etc.).

## Config Variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` | `0` (use `EXECUTOR_MAX_GAS_LIMIT`) | Per-tx gas cap for option execution broadcasts. Used both for the signed transaction and as the effective `broadcast_gas_limit` in the safety check. |
| `OPTION_EXECUTION_SIMULATION_GAS_LIMIT` | `0` (uncapped `eth_call`) | Optional gas cap for the manual `eth_call` simulation. Recorded on each broadcast row; uncapped simulation does not bypass the preflight. |
| `OPTION_EXECUTION_GAS_SAFETY_BPS` | `12500` (25% margin) | Safety multiplier in basis points: `required_gas = estimated_gas * bps / 10000`. Validated to `[10000, 50000]`. `10000` disables the margin; `50000` is the conservative ceiling. |
| `EXECUTOR_MAX_GAS_LIMIT` | from env | Fallback per-tx cap when `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=0`. The safety check still uses this resolved cap. |

The admin config endpoint exposes the sanitized values under `options.execution_gas_safety_bps`, `options.execution_broadcast_gas_limit`, and `options.execution_simulation_gas_limit`.

## Persistence

Migration `0022_option_execution_gas_safety.sql` adds nullable columns to `option_execution_transactions`:

- `estimated_gas BIGINT`
- `required_gas BIGINT`
- `simulation_gas_limit BIGINT`
- `broadcast_gas_limit BIGINT`
- `gas_safety_bps INTEGER`
- `gas_check_status TEXT` — one of `skipped`, `ok`, `estimate_failed`, `broadcast_cap_too_low`, `below_safety_margin`, `uncapped_broadcast_rejected`.
- `gas_check_error TEXT`

Existing rows remain valid (all new columns are nullable). The historical failed broadcast row is preserved unchanged.

The broadcast HTTP response surfaces the same fields so operators can confirm the cap, estimate, and required gas without inspecting the database.

## Expected Operator Workflow

For the next live attempt:

1. Verify external preconditions (oracle feed freshness, executor authorization, balances).
2. Set live env:
   - `OPTION_EXECUTION_BROADCAST_GAS_LIMIT >= 1_300_000` (above observed `1_040_080` with margin to spare). This will be re-validated by the preflight, so an under-set value cannot reach the chain.
   - Optionally `OPTION_EXECUTION_SIMULATION_GAS_LIMIT` equal to the broadcast cap to keep the simulation comparable; the safety check no longer relies on this value for safety, only for visibility.
   - `OPTION_EXECUTION_GAS_SAFETY_BPS` left at the default `12500` unless deliberately tightening.
3. Recreate the intent + signatures + fresh simulation against a clean (non-`broadcast_submitted`) intent.
4. `POST /options/execution-intents/:id/broadcast`. The backend will:
   - run `eth_estimateGas`,
   - confirm `broadcast_gas_limit >= estimated_gas * safety_bps / 10000`,
   - sign and submit only if the check returns `ok`,
   - record `gas_check_status` on the resulting row.

If the response shows `gas_check_status != "ok"`, no transaction was submitted: raise the relevant cap (or fix the failing `from`) and retry with a fresh intent.

## Patch Scope

- No transaction was submitted during this task.
- No call was made to `/executor/broadcast` or the option broadcast endpoint.
- No new rows were inserted into `option_execution_transactions` or `execution_transactions` from a live broadcast.
- No private keys were printed.
- No Solidity, frontend, or deployment was changed.
- No existing evidence row was cleaned up.

## Remaining Blocker Before The Next Live Broadcast

A new option execution intent + signatures must be created (the preserved intent is already `broadcast_submitted` against the failed tx hash). Set `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` to a value comfortably above the on-chain `eth_estimateGas` result for the new calldata; the preflight will reject the broadcast if not.
