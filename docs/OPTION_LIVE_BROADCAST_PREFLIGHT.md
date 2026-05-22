# Option Live Broadcast Preflight V1L

Date: 2026-05-21 (live env rerun)

Scope: preflight only. No live broadcast was performed, no transaction was submitted, `/executor/broadcast` was not called, and no Solidity, frontend, deployment, commit, or push action was performed.

## Inputs

- OptionMatchingEngine: `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- Option ID: `24145907678156652148089862289363692212069910767044828147380657249455352740183`
- Test start timestamp: `1779382203740`

## Process Environment Snapshot

The rerun used the live process environment. Required values are present without being printed.

| Variable | Status |
| --- | --- |
| `RPC_URL` | present (value not printed) |
| `DATABASE_URL` | present |
| `BUYER_PRIVATE_KEY` | present (value not printed) |
| `SELLER_PRIVATE_KEY` | present (value not printed) |
| `EXECUTOR_PRIVATE_KEY` | present (value not printed) |
| `BUYER_ADDRESS` | present |
| `SELLER_ADDRESS` | present |
| `EXECUTOR_FROM_ADDRESS` | present |
| `OPTION_EXECUTION_SIMULATION_FROM` | injected at backend launch from the derived executor address |

Private keys were read only via `cast wallet address --private-key "$VAR"` and the local `sign_option_execution_intent` binary, which consumes the key from the process environment and never echoes it.

## Derived Public Addresses

| Role | Address |
| --- | --- |
| Buyer | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` |
| Seller | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` |
| Executor | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` |

## Authorization Check

```text
cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  "isExecutor(address)(bool)" \
  0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 \
  --rpc-url "$RPC_URL"
```

Result: `true`. The derived executor is authorized on `OptionMatchingEngine`. No `cast send` was performed.

## Backend Runtime Configuration

The backend was started locally on `127.0.0.1:18080` with persistence enabled and the broadcast surface forced off:

| Flag | Value |
| --- | --- |
| `PERSISTENCE_ENABLED` | `true` |
| `OPTIONS_ENABLED` | `true` |
| `OPTION_RFQ_ENABLED` | `true` |
| `OPTION_EXECUTION_ENABLED` | `true` |
| `OPTION_EXECUTION_REQUIRE_PERSISTENCE` | `true` |
| `OPTION_MATCHING_ENGINE_ADDRESS` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `OPTION_EXECUTION_SIGNATURE_MODE` | `strict` |
| `OPTION_EXECUTION_CHAIN_ID` | `84532` |
| `OPTION_EXECUTION_EIP712_NAME` | `DeOptV2-OptionMatchingEngine` |
| `OPTION_EXECUTION_EIP712_VERSION` | `1` |
| `OPTION_EXECUTION_DEFAULT_SETTLEMENT_DECIMALS` | `6` |
| `OPTION_NONCE_SYNC_ENABLED` | `true` |
| `OPTION_NONCE_SYNC_REQUIRE_RPC` | `true` |
| `OPTION_NONCE_SYNC_STRICT` | `true` |
| `OPTION_EXECUTION_SIMULATION_ENABLED` | `true` |
| `OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION` | `true` |
| `OPTION_EXECUTION_SIMULATION_FROM` | derived executor address |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | `false` |
| `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` | `true` |
| `OPTION_EXECUTION_BROADCAST_GAS_LIMIT` | `0` |
| `EXECUTION_ENABLED` | `false` |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `false` |
| `EXECUTOR_DRY_RUN` | `true` |
| `MM_GATEWAY_ENABLED` | `false` |
| `INDEXER_ENABLED` | `false` |
| `RECONCILIATION_ENABLED` | `false` |
| `CONFIRMATION_ENABLED` | `false` |
| `SIMULATION_ENABLED` | `false` |
| `PERP_NONCE_SYNC_ENABLED` | `false` |
| `RFQ_ENABLED` | `false` |

`/admin/config` reflected this configuration: `option_execution_broadcast_enabled=false`, `real_broadcast_enabled=false`, `execution_enabled=false`, `option_execution_simulation_enabled=true`, `option_nonce_sync_enabled=true`, `configured.rpc=true`, `configured.executor_private_key=true`, `configured.database=true`. No raw secrets, no raw private keys, no raw database URL appeared in the response.

## Option Series, Orders, and Intent

The backend option series mapped to the live on-chain optionId was already present in Postgres and was reused via the idempotent path:

| Field | Value |
| --- | --- |
| `option_series_id` | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` |
| `onchain_series_id` | `24145907678156652148089862289363692212069910767044828147380657249455352740183` |
| `underlying` | `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` |
| `settlement_asset` | `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| `expiry` | `1893456000` |
| `strike_1e8` | `300000000000` |
| `is_call` | `true` |
| `contract_size_1e8` | `100000000` |
| `status` | `active` |

Crossing orders were created via `POST /options/orders`:

- Seller `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3`, side `sell`, `price_1e8=100000000`, `size_1e8=100000000`.
- Buyer `0xc0A76c2A6c6b70C0B065A05E64417886416cc976`, side `buy`, `price_1e8=100000000`, `size_1e8=100000000`.

This produced one `option_orderbook_fill` and the corresponding option execution intent:

- `intent_id`: `7e52d08f-26b3-4c25-a246-59c25277951e`
- `onchain_intent_id`: `0x6da769f7f066b192aefc13b271232002a569b9433e458531fa2740470b1bb928`
- `onchain_option_id`: `24145907678156652148089862289363692212069910767044828147380657249455352740183`

## Signing and Strict Submission

Buyer and seller EIP-712 signatures were produced locally via `cargo run --bin sign_option_execution_intent` using `BUYER_PRIVATE_KEY` and `SELLER_PRIVATE_KEY` from the process environment.

- Signing payload digest: `0xf1c71916b24f9e0d8acf31a90713d8e6a27b22ffc6438cc7957f955aacb64799`
- `domain.name`: `DeOptV2-OptionMatchingEngine`, `version=1`, `chainId=84532`, `verifyingContract=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`
- `primaryType`: `OptionTrade`

Signatures were submitted to `POST /options/execution-intents/<intent_id>/signatures` with `OPTION_EXECUTION_SIGNATURE_MODE=strict`. Response: `status=calldata_ready`, `buyer_signature_present=true`, `seller_signature_present=true`, `calldata_ready=true`, `missing_signatures=false`, and calldata returned (function selector `0x031f77b3` for `executeOption`).

## Live Simulation

```text
POST /options/execution-intents/7e52d08f-26b3-4c25-a246-59c25277951e/simulate
```

Response:

```json
{
  "intent_id": "7e52d08f-26b3-4c25-a246-59c25277951e",
  "simulation_status": "simulation_ok",
  "block_number": 41807063,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "simulated_at_ms": 1779382416250,
  "submitted": false,
  "confirmed": false
}
```

`simulation_ok` was reached against Base Sepolia with the authorized executor, valid buyer/seller EIP-712 signatures, and the live on-chain optionId. No transaction was submitted (`submitted=false`).

## Disabled Broadcast Endpoint Check

Endpoint called while `OPTION_EXECUTION_BROADCAST_ENABLED=false`:

```text
POST /options/execution-intents/7e52d08f-26b3-4c25-a246-59c25277951e/broadcast
```

Response:

```text
HTTP/1.1 400 Bad Request
{"error":"configuration error: option execution broadcast is disabled"}
```

This is the expected clean disabled rejection. `ensure_option_execution_broadcast_enabled` rejects before duplicate-tx lookup, executor key access, transaction signing, or any `option_execution_transactions` insert.

## Transaction Table Counts

Read-only Postgres counts after the disabled endpoint check:

| Table | Filter | Count |
| --- | --- | ---: |
| `option_execution_transactions` | `created_at_ms >= 1779382203740` | 0 |
| `execution_transactions` | `created_at_ms >= 1779382203740` | 0 |
| `option_execution_transactions` | total | 0 |
| `execution_transactions` | total | 1 (pre-existing, predates TEST_START_MS) |

No `option_execution_transactions` or `execution_transactions` rows were created during this preflight.

## No Forbidden Mutation

The backend log contained no `eth_sendRawTransaction`, no `/executor/broadcast` call, and no live submission trace. Only `/options/orders`, `/options/execution-intents/.../signing-payload`, `/options/execution-intents/.../signatures`, `/options/execution-intents/.../simulate`, and the disabled `/options/execution-intents/.../broadcast` request were issued. The disabled-broadcast guard fired at the configuration check, before any provider, key, or insert path.

## Code Patch

No backend code patch was needed for this preflight. The existing option broadcast implementation already fails closed when `OPTION_EXECUTION_BROADCAST_ENABLED=false`, and the simulation/signing/strict-submit paths produced the expected `simulation_ok` end-to-end against Base Sepolia.

Relevant behavior:

- `broadcast_option_execution_intent` calls `ensure_option_execution_broadcast_enabled` before provider construction.
- `broadcast_option_execution_intent_with_provider` calls the same guard before intent lookup, duplicate transaction lookup, key access, signing, or transaction insert.
- The disabled guard returns `configuration error: option execution broadcast is disabled`.

## Future Real Broadcast Flag Matrix

Use this only for a later authorized live broadcast run. Do not use it for preflight-only runs.

| Variable | Required value / requirement |
| --- | --- |
| `PERSISTENCE_ENABLED` | `true` |
| `DATABASE_URL` | set |
| `OPTIONS_ENABLED` | `true` |
| `OPTIONS_REQUIRE_PERSISTENCE` | `true` |
| `OPTION_EXECUTION_ENABLED` | `true` |
| `OPTION_EXECUTION_REQUIRE_PERSISTENCE` | `true` |
| `OPTION_MATCHING_ENGINE_ADDRESS` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` |
| `OPTION_EXECUTION_SIGNATURE_MODE` | `strict` |
| `OPTION_EXECUTION_CHAIN_ID` | `84532` |
| `OPTION_EXECUTION_EIP712_NAME` | `DeOptV2-OptionMatchingEngine` |
| `OPTION_EXECUTION_EIP712_VERSION` | `1` |
| `OPTION_NONCE_SYNC_ENABLED` | `true` |
| `OPTION_NONCE_SYNC_REQUIRE_RPC` | `true` |
| `OPTION_NONCE_SYNC_STRICT` | `true` |
| `OPTION_EXECUTION_SIMULATION_ENABLED` | `true` |
| `OPTION_EXECUTION_REQUIRE_RPC_FOR_SIMULATION` | `true` |
| `OPTION_EXECUTION_REQUIRE_SIMULATION_OK` | `true` |
| `OPTION_EXECUTION_BROADCAST_ENABLED` | `true` |
| `EXECUTION_ENABLED` | `true` |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | `true` |
| `EXECUTOR_DRY_RUN` | `true` |
| `EXECUTOR_PRIVATE_KEY` | set |
| `EXECUTOR_FROM_ADDRESS` | set to the derived executor address, if configured explicitly |
| `EXECUTOR_CHAIN_ID` | `84532` |
| `EXECUTOR_MAX_GAS_LIMIT` | nonzero |
| `EXECUTOR_MAX_FEE_PER_GAS_WEI` | set |
| `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` | set |
| `RPC_URL` | set to Base Sepolia RPC |
| `MM_GATEWAY_ENABLED` | `false` unless the separate gateway is intentionally needed |

Before enabling `OPTION_EXECUTION_BROADCAST_ENABLED=true`, verify the derived executor address is authorized:

```text
cast wallet address --private-key "$EXECUTOR_PRIVATE_KEY"
cast call 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b "isExecutor(address)(bool)" "$EXECUTOR_ADDRESS" --rpc-url "$RPC_URL"
```

Do not print or log the private key.

## Manual Live Broadcast Procedure

For a later authorized live broadcast window:

1. Export the full real-broadcast flag matrix above in the process environment.
2. Derive the executor public address locally and verify `isExecutor(executor) == true`.
3. Start the backend with persistence enabled.
4. Create or select the option execution intent.
5. Fetch the signing payload and sign it with valid buyer and seller keys.
6. Submit signatures and verify calldata is ready.
7. Run option simulation and require `simulation_ok`.
8. Recheck that no submitted option execution transaction already exists for the intent.
9. Call only the option endpoint:

```text
POST /options/execution-intents/<intent_id>/broadcast
```

10. Record the returned transaction hash and monitor confirmation.

## Cleanup

After the run, the backend was stopped and its runtime rows since `TEST_START_MS=1779382203740` were removed:

- `DELETE FROM option_execution_intents WHERE intent_id = '7e52d08f-26b3-4c25-a246-59c25277951e';` — 1 row removed.
- `DELETE FROM option_fills WHERE fill_id = '4043678c-aab2-4a42-ae4c-933f284f8e15';` — 1 row removed.
- `DELETE FROM option_orders WHERE order_id IN ('616eb211-e69a-43ba-bac6-86fa490c97f3','e2b98518-8fea-4730-ba03-dc0a1e1c0140');` — 2 rows removed.

Post-cleanup counts since `TEST_START_MS`: `option_execution_intents=0`, `option_fills=0`, `option_orders=0`, `option_execution_transactions=0`, `execution_transactions=0`. The pre-existing intent `6ac7db54-f30c-4964-a863-c8484fcf3b11` and the pre-existing series row were preserved.

## Remaining Blocker

None for V1L preflight. The live-env preflight is complete:

- env presence verified without printing values
- buyer/seller/executor public addresses derived
- `isExecutor(executor) == true` on Base Sepolia
- `OPTION_EXECUTION_BROADCAST_ENABLED=false`, `EXECUTION_ENABLED=false`, `EXECUTOR_REAL_BROADCAST_ENABLED=false` enforced
- live `simulation_ok` reproduced end-to-end with valid signatures
- broadcast endpoint cleanly rejected with HTTP 400 `configuration error: option execution broadcast is disabled`
- no `option_execution_transactions` or `execution_transactions` rows created
- no `/executor/broadcast` call; no `eth_sendRawTransaction`

A real on-chain broadcast remains gated by an explicit authorized flag flip to `OPTION_EXECUTION_BROADCAST_ENABLED=true` (and the rest of the real-broadcast matrix above), which is out of scope for this preflight.
