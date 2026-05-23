# Option Event Indexer V1X-B Multi-Emitter

Date: 2026-05-23

## Why V1X Was Insufficient

V1X decoded the right first set of option execution events, but it fetched
logs only from the configured `OptionMatchingEngine` address. That covers
`OptionTradeExecuted`, but it misses the accounting events emitted deeper
in the execution path.

For the successful V1S trade, the fee and vault audit showed:

- `OptionTradeExecuted` was emitted by `OptionMatchingEngine`.
- `TradeExecuted` and `TradingFeeCharged` were emitted by `MarginEngine`.
- Premium and fee movement evidence was emitted by `CollateralVault` as
  `InternalTransfer`, with balance writes visible as `Synced`.
- `FeesManager` computes/quotes fee configuration, but it does not move
  funds in the V1S execution path.

## Solidity Emitter Audit

Audited current Solidity sources under `../deopt-v2-sol/src`:

- `src/matching/OptionMatchingEngine.sol`
- `src/margin/MarginEngineTypes.sol`
- `src/margin/MarginEngineTrading.sol`
- `src/margin/MarginEngineOps.sol`
- `src/collateral/CollateralVaultStorage.sol`
- `src/collateral/CollateralVaultActions.sol`
- `src/collateral/CollateralVaultYield.sol`
- `src/fees/IFeesManager.sol`
- `src/fees/FeesManager.sol`

Execution/cashflow-related emitters now supported:

| Contract | Events |
| --- | --- |
| `OptionMatchingEngine` | `OptionTradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)` |
| `MarginEngine` | `TradeExecuted(address,address,uint256,uint128,uint128)` |
| `MarginEngine` | `TradingFeeCharged(address,address,address,uint256,bool,uint256,uint256,uint256,uint256,uint256,bool)` |
| `MarginEngine` | `CollateralDeposited(address,address,uint256)` |
| `MarginEngine` | `CollateralWithdrawn(address,address,uint256,uint256)` |
| `CollateralVault` | `InternalTransfer(address,address,address,uint256)` |
| `CollateralVault` | `Deposited(address,address,uint256)` |
| `CollateralVault` | `Withdrawn(address,address,uint256)` |
| `CollateralVault` | `Synced(address,address,uint256)` |
| `FeesManager` | `FeeBpsCapSet(uint16,uint16)` |
| `FeesManager` | `DefaultFeesSet(uint16,uint16,uint16,uint16)` |
| `FeesManager` | `MerkleRootSet(bytes32,bytes32,uint64)` |
| `FeesManager` | `TierClaimed(address,uint8,uint64,uint64)` |
| `FeesManager` | `OverrideSet(address,uint16,uint16,uint16,uint16,uint64,bool)` |

`OptionPositionUpdated` is still not present in the Solidity tree and is
not decoded.

## Config

New option-event-indexer emitter config:

| Env key | Required when enabled | Notes |
| --- | --- | --- |
| `OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS` | yes | Falls back to `OPTION_MATCHING_ENGINE_ADDRESS` when unset. |
| `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS` | yes | Falls back to `MARGIN_ENGINE` when unset. |
| `OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS` | yes | Falls back to `COLLATERAL_VAULT` when unset. |
| `OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS` | no | Falls back to `FEES_MANAGER` when unset; validated if configured. |

Existing indexer controls remain unchanged:

- `OPTION_EVENT_INDEXER_ENABLED`
- `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS`
- `OPTION_EVENT_INDEXER_FROM_BLOCK`
- `OPTION_EVENT_INDEXER_BATCH_BLOCKS`
- `OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS`
- `OPTION_EVENT_INDEXER_REQUIRE_RPC`
- `RPC_URL`

When disabled, missing emitter addresses are safe. When enabled, startup
requires persistence, required nonzero emitter addresses, and `RPC_URL`
when `OPTION_EVENT_INDEXER_REQUIRE_RPC=true`.

## DB Behavior

The existing `option_execution_events` schema remains sufficient:

- idempotency is preserved by `(chain_id, tx_hash, log_index)`;
- `contract_address` records the exact emitting contract;
- events from different contracts in the same transaction are linked to
  the same `option_execution_transaction` by `tx_hash`;
- `onchain_intent_id` is populated only where the actual event includes it;
- raw topics/data and decoded JSON are persisted for every supported event.

The indexer still does not mutate trade confirmation or reconciliation
status. It does not insert generic `execution_transactions`.

## Admin Endpoint

`GET /admin/options/events` now includes:

- `emitter_contracts`: configured emitter role/address pairs;
- `counts_by_event_name`;
- `counts_by_contract_address`;
- existing `counts` alias for event-name counts;
- recent events, latest tick, and cursor state.

`GET /admin/config` exposes the same sanitized emitter contract config under
`options.event_indexer`. RPC URLs and private keys are never returned.

## V1S Expected Coverage

For tx `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`,
expected multi-emitter coverage is:

- `OptionMatchingEngine`: one `OptionTradeExecuted`.
- `MarginEngine`: one `TradeExecuted`, two `TradingFeeCharged`.
- `CollateralVault`: three `InternalTransfer`, plus `Synced` balance-write
  events.
- `FeesManager`: no execution cashflow event expected for this tx.

Manual live backfill remains operator-controlled: configure the emitter
addresses and set `OPTION_EVENT_INDEXER_FROM_BLOCK` before enabling the
indexer against a dev/local database.

## Limitations

- This is an event ledger only. It does not perform reconciliation.
- It does not compare on-chain fee totals to backend fee-ledger rows.
- Settlement and liquidation reconciliation are deferred.
- It does not broadcast, retry, submit transactions, create intents, or
  create production option/generic transaction rows.

## Deferred Work

- Reconciliation worker for option event coverage.
- On-chain fee reconciliation against backend fee ledger state.
- Operator tooling for dry-run V1S backfill reports without cursor mutation.
