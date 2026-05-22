# Option First Live Broadcast

Date: 2026-05-22

Scope: first live option execution broadcast on Base Sepolia through the option-specific endpoint only. `/executor/broadcast` was not called, no generic executor broadcast endpoint was used, no Solidity/frontend/deployment/commit/push action was performed, and no private keys were printed.

## Inputs

- `TEST_START_MS`: `1779445529961`
- OptionMatchingEngine: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- Active on-chain optionId: `24145907678156652148089862289363692212069910767044828147380657249455352740183`
- Backend option series: `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d`
- Buyer: `0xc0A76c2A6c6b70C0B065A05E64417886416cc976`
- Seller: `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3`
- Executor: `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27`

## Environment And Authorization

Required live environment values were present in the process environment without printing values:

- `RPC_URL`
- `BUYER_PRIVATE_KEY`
- `SELLER_PRIVATE_KEY`
- `EXECUTOR_PRIVATE_KEY`

Public addresses were derived locally from the private keys and matched the expected buyer, seller, and executor addresses above.

Read-only executor authorization check:

```text
OptionMatchingEngine.isExecutor(0xc35F7A8A103A9A4464adfaa76B9B514093D23C27) = true
```

## Backend Runtime Flags

The backend ran on `127.0.0.1:8080` with these live broadcast gates:

| Flag | Value |
| --- | --- |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | `true` |
| `EXECUTION_ENABLED` | `true` |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `true` |
| `EXECUTOR_DRY_RUN` | `false` |
| `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` | `true` |
| `OPTION_EXECUTION_SIGNATURE_MODE` | `strict` |
| `OPTION_NONCE_SYNC_ENABLED` | `true` |
| `OPTION_NONCE_SYNC_REQUIRE_RPC` | `true` |
| `OPTION_NONCE_SYNC_STRICT` | `true` |
| `OPTION_EXECUTION_SIMULATION_ENABLED` | `true` |
| `OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION` | `true` |
| `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` | `0` |
| `EXECUTOR_FROM_ADDRESS` | derived executor address |
| `EXECUTOR_CHAIN_ID` | `84532` |
| `INDEXER_ENABLED` | `false` |
| `RECONCILIATION_ENABLED` | `false` |
| `CONFIRMATION_ENABLED` | `false` |
| `SIMULATION_ENABLED` | `false` |
| `PERP_NONCE_SYNC_ENABLED` | `false` |
| `MM_GATEWAY_ENABLED` | `false` |

`/health` returned `ok=true`. `/admin/config` showed `option_execution_broadcast_enabled=true`, `execution_enabled=true`, `real_broadcast_enabled=true`, `executor_private_key=true`, `rpc=true`, strict option execution signatures, strict option nonce sync, and option simulation enabled. The config response exposed booleans for configured secrets and did not expose raw private keys or the raw RPC URL.

## Intent And Signatures

Created one crossing option orderbook fill on the active option series:

- Sell order: `2cd4f021-37cc-482f-8cf0-1df2efb7de3f`
- Buy order: `10ccff89-ee83-438a-83d4-bc813e719b2b`
- Fill: `a8d46003-c144-43b4-b422-c922ff21135d`

Generated option execution intent:

- Intent ID: `4075afe3-fe42-457d-a9ca-eb0907d09a74`
- On-chain intent ID: `0x18c8c98825599abf10ce99d0e6f12c9215fc6ecbd497784ba37aff433909493b`
- Option ID: `24145907678156652148089862289363692212069910767044828147380657249455352740183`
- Quantity: `1`
- Premium per contract native: `1000000`
- Buyer nonce: `0`
- Seller nonce: `0`

Signing payload:

- Digest: `0x9e4e1b3087b13f785fe9da302a0b1846e7f121edfa7b8dcf7b691c467e381523`
- Domain: `DeOptV2-OptionMatchingEngine`, version `1`, chain ID `84532`, verifying contract `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- Primary type: `OptionTrade`

Buyer and seller signatures were produced locally with `sign_option_execution_intent`; strict signature submission returned `status=calldata_ready`, `buyer_signature_present=true`, `seller_signature_present=true`, `calldata_ready=true`, and `missing_signatures=false`.

## Fresh Simulation

Fresh live simulation was run immediately before broadcast:

```json
{
  "intent_id": "4075afe3-fe42-457d-a9ca-eb0907d09a74",
  "simulation_status": "simulation_ok",
  "block_number": 41838777,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1779445842538,
  "submitted": false,
  "confirmed": false
}
```

Pre-broadcast DB counts since `TEST_START_MS`:

| Table | Count |
| --- | ---: |
| `option_execution_transactions` | 0 |
| `execution_transactions` | 0 |

## Broadcast Result

Exactly one live broadcast call was made:

```text
POST /options/execution-intents/4075afe3-fe42-457d-a9ca-eb0907d09a74/broadcast
```

Response:

```json
{
  "intent_id": "4075afe3-fe42-457d-a9ca-eb0907d09a74",
  "status": "broadcast_submitted",
  "tx_hash": "0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c",
  "to": "0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b",
  "from": "0xc35f7a8a103a9a4464adfaa76b9b514093d23c27",
  "transaction_id": "204a3070-50b1-4b89-865d-ad183752b1e8",
  "broadcast_enabled": true,
  "submitted": true,
  "duplicate": false,
  "confirmed": false
}
```

No automatic retry was attempted.

## DB Evidence

Post-broadcast counts since `TEST_START_MS`:

| Table | Count |
| --- | ---: |
| `option_execution_transactions` | 1 |
| `execution_transactions` | 0 |

Persisted option transaction row:

| Field | Value |
| --- | --- |
| `transaction_id` | `204a3070-50b1-4b89-865d-ad183752b1e8` |
| `intent_id` | `4075afe3-fe42-457d-a9ca-eb0907d09a74` |
| `onchain_intent_id` | `0x18c8c98825599abf10ce99d0e6f12c9215fc6ecbd497784ba37aff433909493b` |
| `sender` | `0xc35f7a8a103a9a4464adfaa76b9b514093d23c27` |
| `target` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `value_wei` | `0` |
| `gas_limit` | `1000000` |
| `tx_hash` | `0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c` |
| `status` | `submitted` |
| `created_at_ms` | `1779445862372` |
| `updated_at_ms` | `1779445862372` |

Persisted intent row after broadcast:

| Field | Value |
| --- | --- |
| `status` | `broadcast_submitted` |
| `error` | `NULL` |
| `simulation_status` | `simulation_ok` |
| `simulation_block_number` | `41838777` |
| `simulated_at_ms` | `1779445842538` |

The broadcast evidence rows were preserved and were not cleaned up.

## Receipt

Read-only receipt check:

| Field | Value |
| --- | --- |
| `transactionHash` | `0xe832365b11ead105e020b65a25570516e87ab1af2b1225b698561f090eff8b7c` |
| `blockNumber` | `41838788` |
| `from` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |
| `to` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `status` | `0 (failed)` |
| `gasUsed` | `982941` |
| `effectiveGasPrice` | `6000000` |
| `logs` | `[]` |

No confirmation or reconciliation state was written.

## No Forbidden Mutation Proof

- `/executor/broadcast` was not called.
- The generic executor broadcast endpoint was not used.
- `execution_transactions` rows since `TEST_START_MS`: `0`.
- Exactly one option-specific transaction row was created since `TEST_START_MS`.
- No second option broadcast/idempotency call was made, to avoid any chance of submitting more than the one allowed transaction.
- No fake transaction hash was created; the persisted hash is the provider-returned hash.
- No confirmed/reconciled status was written.
- No Solidity, frontend, deployment, commit, or push action was performed.

## Code Patch

A minimal backend patch was required before the run because startup rejected the requested live flag matrix `EXECUTION_ENABLED=true` with `EXECUTOR_DRY_RUN=false`. The patch allows that startup state for manual broadcast paths and prevents the legacy dry-run executor loop from starting unless `EXECUTOR_DRY_RUN=true`. The generic executor manual tick still rejects non-dry-run real execution.

## Next Steps

- Diagnose why the mined transaction failed despite the fresh `simulation_ok`.
- Add option execution receipt/indexer/confirmation/reconciliation flow in a later task.
- Keep the preserved `option_execution_transactions` row as broadcast evidence.
