# Option Broadcast Confirmation & Reconciliation V1T

Date: 2026-05-22

## Scope

V1T confirms and reconciles the V1S option execution transaction
(`0x5964a7b3…`) against its on-chain receipt, decodes the emitted event
logs, verifies post-trade nonces / positions / vault balances, and adds a
minimal backend confirmation endpoint so the persisted `option_execution_intents` row
transitions out of `broadcast_submitted` into either
`broadcast_confirmed` (status 1) or `broadcast_reverted` (status 0).
The scope is intentionally narrow: receipt-driven status reconciliation
for the option-specific transaction table. **No event indexer was built**;
the V1S logs are inspected once, attributed by selector, and recorded in
this document.

V1T did **not** broadcast any new transaction, did not call
`/executor/broadcast`, did not call the option broadcast endpoint, did not
create or modify generic `execution_transactions`, did not cleanup the
preserved V1L evidence row, and did not print private keys.

## V1S Tx Under Review

- Tx hash: `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`
- Intent: `e6d2941b-65f7-413a-958f-74ab22c53b08`
- DB transaction row: `cae8c7e7-ed61-4265-aa7d-75edd94ef03c`

## Preserved DB Evidence (pre-confirmation)

`option_execution_intents` (intent `e6d2941b-…`):

| Field | Value |
| --- | --- |
| `status` | `broadcast_submitted` |
| `simulation_status` | `simulation_ok` |
| `simulation_block_number` | `41856962` |
| `simulated_at_ms` | `1779482212191` |
| `onchain_intent_id` | `0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5` |
| `error` | NULL |

`option_execution_transactions` (row `cae8c7e7-…`):

| Field | Value |
| --- | --- |
| `tx_hash` | `0x5964a7b3…` |
| `sender` | `0xc35f7a8a…` |
| `target` | `0xf2D1D85c…` |
| `status` | `submitted` |
| `gas_limit` | `1500000` |
| `estimated_gas` | `1091120` |
| `required_gas` | `1363900` |
| `broadcast_gas_limit` | `1500000` |
| `gas_safety_bps` | `12500` |
| `gas_check_status` | `ok` |
| `confirmation_status` | NULL (pre-V1T) |

Generic `execution_transactions` rows for the V1S tx hash: **0**.

DB calldata equals tx input byte-for-byte (both `1674` chars / `837` bytes, `cmp=match`).

## On-chain Receipt

| Field | Value |
| --- | --- |
| `status` | **1 (success)** |
| `blockNumber` | `41856964` |
| `blockHash` | `0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853` |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (executor) |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (OptionMatchingEngine) |
| `gasUsed` | `1057772` (within the 1.5M cap, slightly below `estimated_gas=1091120`) |
| `effectiveGasPrice` | `6000000` wei |
| `cumulativeGasUsed` | `1672948` |
| `transactionIndex` | `5` |
| `type` | `2` (EIP-1559) |

`cast tx` confirms:

- `from = executor`, `to = OptionMatchingEngine`, `value = 0`, `chainId = 84532`, `nonce = 523`, `gasLimit = 1500000`, `maxFeePerGas = 1_000_000_000`, `maxPriorityFeePerGas = 1_000_000`, `type = 2`.
- Input selector `0x031f77b3` (`executeTrade(OptionTrade,bytes,bytes)`).
- Input encodes `intentId = 0x0a77c7c9570198c969b1fa597ea193cb6fee563e3bfae514e9a3f0c4e01705f5` (matches stored intent) and `optionId = 0x35621974bccc555e161c6707f0a1a3bca2d02be5e3a4d380980bfaef656e7957` (low-32 bytes of the active series id).

## Event Log Attribution

All 14 emitted logs are accounted for; topic0 hashes were verified against `cast keccak`:

| topic0 | Emitter | Event | Notes |
| --- | --- | --- | --- |
| `0x77178bcf8f3c991d39734824771477a42787fe19b60d5a29c0ec72de167699b3` | `0x00340c…0625D3` (CollateralVault) | `InternalTransfer(address indexed token, address indexed from, address indexed to, uint256 amount)` | Premium 10_000 from buyer → seller; buyer fee 6 to recipient `0x009f38…7500`; seller fee 4 to same recipient. |
| `0xf67cd268c5cb2f9e884934944bae45c8d46cffe591c6243336b8c007ca4cf067` | `0x00340c…0625D3` (CollateralVault) | `Synced(address indexed user, address indexed token, uint256 newBalance)` | One per balance write — buyer pre-fee, seller pre-fee, buyer-post-fee, seller-post-fee, fee-recipient (two updates). |
| `0x12cf63383901008103b6e03c39d208d7757a2f9842d9d4e18e58bc13f75f7f7b` | `0x6c5665De…5b5F8` (MarginEngine) | `TradingFeeCharged(address indexed trader, address indexed recipient, address indexed settlementAsset, uint256 optionId, bool isMaker, uint256 premium, uint256 notionalImplicit, uint256 notionalFee, uint256 premiumCapFee, uint256 appliedFee, bool cappedByPremium)` | Two emits: buyer (taker, fee 6) and seller (maker, fee 4). |
| `0x6f0909c4bf7f20fe8de71b889c29e66311610d5f753a42ae63495e08bbb65f7e` | `0x6c5665De…5b5F8` (MarginEngine) | `TradeExecuted(address indexed buyer, address indexed seller, uint256 indexed optionId, uint128 quantity, uint128 price)` | quantity `1`, price `10_000`. |
| `0xb2387b9f0e4823ecef9a16ea4aaba6598c0703fb5e9d8dba37ef303add4cb808` | `0xf2D1D85c…F420b` (OptionMatchingEngine) | `OptionTradeExecuted(bytes32 indexed intentId, address indexed buyer, address indexed seller, uint256 optionId, uint128 quantity, uint128 premiumPerContract, bool buyerIsMaker, uint256 buyerNonce, uint256 sellerNonce)` | intentId matches `0x0a77c7c9…`; quantity `1`; premium `10_000`; buyerIsMaker false; both pre-trade nonces `0`. |

This attribution is the minimum required to *prove* the executeTrade path ran end-to-end; V1T does not persist these events to any indexer table.

## Nonce Reconciliation

| Account | Pre-V1S (`nonces(addr)`) | Post-V1S (`nonces(addr)`) | Expected |
| --- | ---: | ---: | --- |
| Buyer `0xc0A76c2A…` | `0` | **`1`** | +1 (one trade applied) |
| Seller `0xbAf0976a…` | `0` | **`1`** | +1 |

Both nonces incremented exactly once on the OptionMatchingEngine, matching the single `executeTrade` call.

## Position Reconciliation

Read from `MarginEngine` (`0x6C5665De…`) and `OptionProductRegistry` for the active series (optionId `24145907678156652148089862289363692212069910767044828147380657249455352740183`):

| View | Buyer | Seller |
| --- | ---: | ---: |
| `getPositionQuantity(addr, optionId)` (int128) | `+1` | `-1` |
| `getTraderSeriesLength(addr)` | `1` | `1` |
| `totalShortContracts(addr)` | `0` | `1` |

Long/short position deltas exactly match the single-contract trade. Buyer is long 1 call, seller is short 1 call.

## Vault Reconciliation

Read from `CollateralVault.balances(user, token)` (`0x00340c…0625D3`, settlement asset `0x6eAe407f…`):

| Account | Balance (post-V1S, settlement-native units) |
| --- | ---: |
| Buyer `0xc0A76c2A…` | `9_998_489_994` |
| Seller `0xbAf0976a…` | `9_999_409_996` |

Cross-check against the in-flight ledger documented in the V1S receipt logs (premium 10_000 + buyer fee 6 + seller fee 4):

- Buyer net change ≈ `−10_006` (premium out + maker-vs-taker fee). Matches the InternalTransfer pair (buyer→seller for premium, buyer→fee-recipient for fee).
- Seller net change ≈ `+9_996` (premium in − seller fee out).

(Comparison is informational: this is the first option-trade on the live testnet account; the V1L baseline numbers from the previous failed broadcast are not directly comparable because V1L used a different `premium_per_contract_native` and never landed.)

## Backend Support Audit

Before V1T, the backend had **no** option-specific confirmation/reconciliation
path. Per V1I scope: "Option indexer, reconciliation, and confirmation
remain deferred." Per V1S "Remaining Blocker #1": the same. Therefore V1T
ships a **minimal** patch (no full indexer) so the persisted
`option_execution_transactions` row can transition from `submitted` to a
receipt-attested terminal state, and the linked intent can transition
from `broadcast_submitted` to `broadcast_confirmed` /
`broadcast_reverted`.

## Code Patch (Required)

### Migration

- `migrations/0023_option_execution_confirmation.sql` (new, applied to live DB):
  - `ALTER TABLE option_execution_transactions ADD COLUMN IF NOT EXISTS confirmation_status TEXT NULL`
  - `confirmed_at_ms BIGINT NULL`
  - `confirmed_block_number BIGINT NULL`
  - `receipt_status BIGINT NULL`
  - `confirmation_error TEXT NULL`
  - All columns nullable; idempotent re-apply safe.

### Types

`src/options/types.rs`:

- New enum `OptionExecutionConfirmationStatus { Pending, MinedSuccess, MinedReverted, ReceiptMissing, ReceiptError }` with `as_str` / `parse`.
- New variants on `OptionExecutionIntentStatus`: `BroadcastConfirmed`, `BroadcastReverted` (parse + serialise wired).
- New fields on `OptionExecutionTransaction`: `confirmation_status`, `confirmed_at_ms`, `confirmed_block_number`, `receipt_status`, `confirmation_error`.

### Store / Repository

- `OptionSeriesStore::update_option_execution_confirmation(transaction_id, status, ts, block, receipt_status, error)`.
- `PgRepository::get_option_execution_transaction(transaction_id)` (single-row lookup).
- `PgRepository::update_option_execution_confirmation(...)` (parameterised UPDATE statement).
- Row reader and SELECTs extended to include the new columns; existing insert path stays unchanged because all new columns default to NULL at insert time.

### Service

- `service::confirm_option_execution_intent(state, intent_id)` (HTTP entry path; opens an `HttpJsonRpcProvider` against `RPC_URL`).
- `service::confirm_option_execution_intent_with_provider<P: TransactionReceiptProvider>(state, intent_id, provider)` (testable form):
  1. Load intent + most-recent `submitted` transaction row.
  2. `provider.transaction_receipt(tx_hash)`.
  3. Map receipt status → `MinedSuccess` (1), `MinedReverted` (0), `ReceiptMissing` (no row / mismatched hash), `ReceiptError` (rpc/parse failure).
  4. Persist receipt fields to `option_execution_transactions` row (via repo or in-memory store).
  5. If outcome is `MinedSuccess` → intent status `BroadcastConfirmed`; if `MinedReverted` → `BroadcastReverted`; otherwise leave intent as-is (no flapping).
- New struct `OptionExecutionConfirmationOutcome`.

### Route

- `POST /options/execution-intents/:intent_id/confirm` → `OptionExecutionConfirmationResponse` (intent status + transaction id + tx hash + confirmation status + receipt status + confirmed block + confirmed timestamp + confirmation error).

### Tests

Six new `tokio::test` cases in `src/options/service.rs`, all using a new `MockReceiptProvider` (implements `TransactionReceiptProvider`):

- `option_execution_confirm_mined_success_transitions_to_broadcast_confirmed`
- `option_execution_confirm_mined_reverted_transitions_to_broadcast_reverted`
- `option_execution_confirm_missing_receipt_does_not_change_intent_status`
- `option_execution_confirm_receipt_error_does_not_change_intent_status`
- `option_execution_confirm_rejects_intent_without_submitted_transaction`
- `option_execution_confirm_idempotent_on_already_confirmed_row`

All six pass. Existing test count: **231 passing lib tests** (`+6` from V1S) plus the eight integration suites (HTTP, options HTTP, RFQ HTTP, orderbook, etc.).

## DB Status Updates (V1T)

After calling `POST /options/execution-intents/e6d2941b-…/confirm` against live Base Sepolia RPC:

| Field | Before V1T | After V1T |
| --- | --- | --- |
| `option_execution_intents.status` for `e6d2941b-…` | `broadcast_submitted` | **`broadcast_confirmed`** |
| `option_execution_transactions.status` for `cae8c7e7-…` | `submitted` | `submitted` (unchanged: terminal status is intentionally on the intent + the confirmation fields) |
| `option_execution_transactions.confirmation_status` | NULL | **`mined_success`** |
| `option_execution_transactions.receipt_status` | NULL | **`1`** |
| `option_execution_transactions.confirmed_block_number` | NULL | **`41856964`** |
| `option_execution_transactions.confirmed_at_ms` | NULL | **`1779489313015`** |
| `option_execution_transactions.confirmation_error` | NULL | NULL |

The confirm endpoint response payload mirrored the persisted values:

```json
{
  "intent_id": "e6d2941b-65f7-413a-958f-74ab22c53b08",
  "intent_status": "broadcast_confirmed",
  "transaction_id": "cae8c7e7-ed61-4265-aa7d-75edd94ef03c",
  "tx_hash": "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125",
  "confirmation_status": "mined_success",
  "receipt_status": 1,
  "confirmed_block_number": 41856964,
  "confirmed_at_ms": 1779489313015,
  "confirmation_error": null
}
```

## No-Forbidden-Mutation Verification

- `POST /options/execution-intents/:id/broadcast`: **not called**
- `/executor/broadcast`: **not called**
- `eth_sendRawTransaction`: **not called** (only `eth_getTransactionReceipt`)
- `option_execution_transactions` rows since V1T start: **0 new** (only the existing `cae8c7e7-…` row was updated via UPDATE, not INSERT)
- `execution_transactions` rows since V1T start: **0 new** (the V1S tx hash never appeared on the generic table; that count remains 0 as of every prior phase)
- Preserved V1L row (`tx 0xe832365b…`) untouched
- No new option execution intents created during V1T
- No Solidity / frontend / deploy changes
- No private keys printed

## Validation Commands

Code changed → ran the full V1T validation suite:

- `cargo fmt --all` → clean
- `cargo clippy --all-targets --all-features -- -D warnings` → clean
- `cargo test --all-targets --all-features` → all suites pass; `231 + 13 + 37 + 67 + 76 + 12 + 8 + 43 + 0` passing tests, 0 failures
- `cargo build --all-targets --all-features` → clean

## Files Changed

- `migrations/0023_option_execution_confirmation.sql` (new, 5 ALTER TABLEs)
- `src/options/types.rs` — `OptionExecutionConfirmationStatus`; new intent statuses; new fields on `OptionExecutionTransaction`
- `src/options/mod.rs` — re-export new types
- `src/options/store.rs` — `update_option_execution_confirmation`
- `src/db/repository.rs` — `get_option_execution_transaction`, `update_option_execution_confirmation`, row reader includes new columns, SELECTs include new columns
- `src/options/service.rs` — `confirm_option_execution_intent`, `confirm_option_execution_intent_with_provider`, `persist_option_execution_confirmation`, `OptionExecutionConfirmationOutcome`; `option_execution_transaction_from_request` initialises new fields to None; six new tokio tests + `MockReceiptProvider`
- `src/api/routes.rs` — `POST /options/execution-intents/:intent_id/confirm`, `OptionExecutionConfirmationResponse` DTO
- `docs/OPTION_BROADCAST_CONFIRMATION_RECONCILIATION_V1T.md` (this doc)

## Remaining Blocker

None for the V1T scope (confirm + reconcile V1S). Deferred follow-ups (intentionally not built here per "no full indexer"):

1. **Background confirmation worker** for option execution intents (today V1T is operator-triggered via the explicit `confirm` endpoint).
2. **Required-blocks finality gating** (V1T accepts the first-mined receipt as terminal; for higher-value broadcasts the existing perp-side `decide_confirmation` finality logic could be reused, with a configurable `OPTION_EXECUTION_CONFIRMATION_REQUIRED_BLOCKS`).
3. **Persist `gas_used`, `effective_gas_price`, `cumulative_gas_used`** for cost analytics — out of scope for V1T but a single migration + ConfirmationReceipt extension away.
4. **Option event indexer** (`TradeExecuted` / `OptionTradeExecuted` / `TradingFeeCharged` / `InternalTransfer`) — V1T attributes those topics in this doc but does not persist them to a queryable table. A future V1U/W could subscribe to logs.
5. **Settlement / exercise / expiry paths** for the broadcast option position (`buyer_pos=+1`, `seller_pos=-1`) — out of scope.
