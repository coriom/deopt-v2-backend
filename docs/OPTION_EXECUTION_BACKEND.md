# Option Execution Backend V1D

Backend Option Execution Intents V1D turns option fills into signing/calldata artifacts for Solidity `OptionMatchingEngine` and adds safe `eth_call` simulation for calldata-ready intents.

It is disabled by default:

```text
OPTION_EXECUTION_ENABLED=false
OPTION_EXECUTION_REQUIRE_PERSISTENCE=true
OPTION_MATCHING_ENGINE_ADDRESS=
OPTION_EXECUTION_SIGNATURE_MODE=disabled
OPTION_EXECUTION_CHAIN_ID=84532
OPTION_EXECUTION_EIP712_NAME=DeOptV2-OptionMatchingEngine
OPTION_EXECUTION_EIP712_VERSION=1
OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS=6
OPTION_EXECUTION_SIMULATION_ENABLED=false
OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true
OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0
OPTION_EXECUTION_SIMULATION_FROM=
```

When enabled, startup requires `OPTIONS_ENABLED=true`. If `OPTION_EXECUTION_REQUIRE_PERSISTENCE=true`, startup also requires `PERSISTENCE_ENABLED=true`. `OPTION_MATCHING_ENGINE_ADDRESS` must be a valid nonzero EVM address and is used as the EIP-712 verifying contract.

## Intent Creation

The backend creates option execution intents from:

- option orderbook fills when a new option order crosses resting liquidity
- option RFQ fills when an option RFQ quote is accepted

Creation is idempotent by `(source_type, source_id)`. The stored source types are `option_orderbook_fill` and `option_rfq_fill`.

The source option series must have an on-chain option id in `onchain_series_id` or `onchain_product_id`. `underlying` and `settlement_asset` must be nonzero EVM addresses. Fill size must be a whole number of contracts: `quantity = size_1e8 / 100000000`. Premium is converted to settlement-native units per contract: `premiumPerContract = price_1e8 * 10^OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS / 100000000`.

## EIP-712 Payload

`GET /options/execution-intents/:intent_id/signing-payload` returns `primaryType=OptionTrade`, the option execution domain, the message, and the digest.

The type string matches Solidity:

```text
OptionTrade(bytes32 intentId,address buyer,address seller,uint256 optionId,address underlying,address settlementAsset,uint64 expiry,uint64 strike1e8,bool isCall,uint128 contractSize1e8,uint128 quantity,uint128 premiumPerContract,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)
```

`intentId` is `keccak256(bytes(option_execution_intent_uuid_string))`. V1C sets buyer and seller nonces to `0` and deadline to `0`; on-chain nonce synchronization is deferred.

## Signatures And Calldata

`POST /options/execution-intents/:intent_id/signatures` accepts `buyer_signature` and/or `seller_signature`. In disabled signature mode, supplied signatures are shape-validated as 65-byte `0x` hex. In strict mode, the backend recovers each signer from the EIP-712 digest and checks it against the intent buyer or seller.

When both signatures are stored, the backend builds `OptionMatchingEngine.executeTrade(OptionTrade,bytes,bytes)` calldata and marks the option execution intent `calldata_ready`. `GET /options/execution-intents/:intent_id/calldata` returns the stored calldata or builds it from stored signatures.

## Simulation

`POST /options/execution-intents/:intent_id/simulate` is a manual V1D `eth_call` safety check for calldata-ready option execution intents. It is disabled unless `OPTION_EXECUTION_SIMULATION_ENABLED=true`.

Simulation requires:

- existing option execution intent
- buyer and seller signatures
- stored calldata
- nonzero `OPTION_MATCHING_ENGINE_ADDRESS`
- `RPC_URL` when `OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION=true`

The call uses:

- `to = OPTION_MATCHING_ENGINE_ADDRESS`
- `data = intent.calldata`
- `value = 0`
- optional `gas = OPTION_EXECUTION_SIMULATION_GAS_LIMIT` when nonzero
- `from = OPTION_EXECUTION_SIMULATION_FROM` when set, otherwise `EXECUTOR_FROM_ADDRESS`

`OptionMatchingEngine.executeTrade` is executor-gated. If the configured `from` address is not an allowed executor on-chain, simulation is expected to fail with a revert.

Results are stored directly on `option_execution_intents`:

- `simulation_status`: `simulation_pending`, `simulation_ok`, `simulation_failed`, or `simulation_unavailable`
- `simulation_error`
- `simulation_block_number`
- `simulation_revert_data`
- `simulation_revert_selector`
- `simulated_at_ms`

`GET /options/execution-intents/:intent_id/simulation` returns the persisted result, or `simulation_pending` before a simulation has been run.

## Safety Boundary

V1D does not broadcast. Simulation uses `eth_call` only. It does not call `/executor/broadcast`, create `execution_transactions`, fabricate transaction hashes, require private keys, require live RPC for normal tests, or set submitted/confirmed option statuses. Option execution broadcast, indexer, reconciliation, and confirmation remain deferred.

Deferred items include option nonce sync from `OptionMatchingEngine`, option broadcast, option indexer/reconciliation/confirmation, on-chain submission, settlement, exercise, and frontend surfaces.
