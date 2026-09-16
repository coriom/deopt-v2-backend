# PERPS_BASE_SEPOLIA_CLOSED_TEST_COSIGN_ROUTE_V1

Closed-test-only co-sign flow that produces the exact 10-field
`PerpTrade` required by the deployed Base Sepolia PME V1 at
`0x774d96E5739bffadEE91508b4D3D74F5BE29F165`.

## Deployed contract truth (authoritative)

| Field | Value |
|---|---|
| PME address | `0x774d96E5739bffadEE91508b4D3D74F5BE29F165` |
| Chain | Base Sepolia (chainId 84532) |
| Settlement fn | `executeTrade(PerpTrade, bytes buyerSig, bytes sellerSig)` |
| Selector | `0x7a708c4c` |
| Struct | **10-field** `PerpTrade` — NO `maxExecutionPrice1e8` / `minExecutionPrice1e8` |
| `TRADE_TYPEHASH` (on-chain) | `0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293` |
| Domain | `name="DeOptV2-PerpMatchingEngine", version="1", chainId=84532, verifyingContract=PME` |

**Sol repo HEAD contains a WIP 12-field V2 (`maxExecutionPrice1e8` / `minExecutionPrice1e8` inserted) which is NOT deployed. Do not mix them.**

Existing `perp_trade_digest` in `src/execution/perp_trade.rs` computes the 12-field digest and would produce signatures that would NOT verify against the deployed PME. That helper remains for future V2 use but is NOT used by the co-sign route.

The new `perp_trade_v1_digest` and `perp_trade_v1_digest_bytes` compute the correct 10-field digest matching the deployed contract. Their typehash is pinned:

- `PERP_TRADE_V1_TYPE` = `"PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)"`
- `PERP_TRADE_V1_TYPEHASH_HEX` = `0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293`

## Two-phase co-sign flow

### Phase A — Prepare

Route (recommended): `POST /perps/closed-test/trades/prepare`

Core logic: `perps_cosign::prepare_trade_core(state, req)`.

Gates:
- `PERPS_CLOSED_TEST_ENABLED = true` (else `PerpsNotLive`)
- `PERPS_PUBLIC_TRADING_ENABLED = false` (else `PerpsNotLive`)
- Both buyer + seller in `PERPS_CLOSED_TEST_ALLOWLIST`
- `buyer != seller`
- Well-formed EVM addresses
- Non-zero `sizeDelta1e8` and `executionPrice1e8`

Actions:
1. Generate a fresh UUID v4 (backend-owned).
2. Derive `intentId = keccak256(uuid.to_string().as_bytes())` (canonical hyphenated RFC-4122 string bytes).
3. Freeze `buyerNonce = 0`, `sellerNonce = 0` (matches fresh trader-fixture state; future extension: read `PME.nonces(x)` via RPC).
4. Freeze `deadline = now_ms + PREPARE_DEADLINE_TTL_MS (default 1h)`.
5. Build the 10-field `PerpTrade` payload.
6. Compute the EIP-712 digest.
7. Return `{ uuid, intentId, digest, typedData, trade }`.

### Phase B — Cosign

Route (recommended): `POST /perps/closed-test/trades/{uuid}/cosign`

Core logic: `perps_cosign::cosign_verify_core(payload, domain, req)`.

Body:
```json
{ "buyerSignature": "0x…", "sellerSignature": "0x…" }
```

Actions:
1. Recompute the frozen EIP-712 digest server-side (never trust client-supplied digest).
2. `ecrecover(digest, buyer_signature)` must equal `payload.buyer`.
3. `ecrecover(digest, seller_signature)` must equal `payload.seller`.
4. On success, persist `{ buyer_sig, seller_sig }` via `PgRepository::upsert_execution_intent_signatures(intent_id, …)`.
5. The intent's `execution_intent_signatures` row is now `calldata_ready`. BroadcastPolicy can consume it — but real broadcast still requires the independent executor gates.

## Signature verification failure classes

- Malformed 65-byte hex → `PerpsIntentSignatureInvalid`
- Recovery yields address ≠ `payload.buyer` / `payload.seller` → `PerpsIntentTraderMismatch`
- Swapped signatures (buyer_sig belongs to seller and vice-versa) → `PerpsIntentTraderMismatch`
- Signed against wrong `verifyingContract` / `chainId` → recovery yields different address → `PerpsIntentTraderMismatch`
- Tampered `PerpTrade` fields (e.g. size delta ± 1) → digest differs → `PerpsIntentTraderMismatch`

## Identity invariant

The `intentId` bytes32 threaded through the system is derived from ONE UUID:

```
Uuid::new_v4()
  ↓ intent_id.to_string() (RFC-4122 hyphenated)
  ↓ keccak256(uuid_string.as_bytes())
  = PerpTrade.intentId
  = TradeExecuted.topic1 (on-chain event)
  = expected_intent_hash_from_uuid(uuid) (backend receipt verifier)
```

The `PerpTradePayload.intent_id` and `ExecutionIntent.intent_id` MUST both be derivations of the same UUID. Test `y_receipt_identity_uses_uuid_keccak` pins this.

## Existing `PerpOrderIntent` route unchanged

`POST /perps/orders/signed` continues to accept `PerpOrderIntent` sigs and store them for internal matching / durability. Those signatures **cannot** be used as `PerpTrade` sigs for the deployed PME:

- `PerpOrderIntent` typehash = `0xeeaf370e4195f568ccb783efe23803dd5bf3c859aef9d0c3e3f211c2da2d5d1c` (frozen)
- `PerpTrade` typehash (deployed V1) = `0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293`
- Distinct digests → distinct sigs. Not interchangeable.

Do not merge the two cryptographic domains.

## BroadcastPolicy compatibility

Once both signatures are persisted, the existing BroadcastPolicy path:

```
ExecutionIntent (CalldataReady)
  ↓ build_execution_transaction_request(&config, &intent, &signatures)
  ↓ tx_builder::build_perp_execution_call_from_intent
  ↓ abi::encode_execute_trade_calldata
  → selector 0x7a708c4c + 10-field PerpTrade tuple + buyer_sig + seller_sig
```

Test `x_final_calldata_decodes_to_frozen_trade` proves the first 4 bytes of the calldata are the deployed selector `0x7a708c4c` and that the tuple encodes the frozen trade.

## Security gates preserved

The cosign route creates a **broadcast-eligible** intent. Real broadcast still requires:
- `EXECUTION_ENABLED=true`
- `EXECUTOR_DRY_RUN=false`
- `EXECUTOR_REAL_BROADCAST_ENABLED=true`
- Signer triad validated at startup
- `preflight_static` + on-chain PME state OK
- Reconciler + executor tick loops running

No new secret material is stored. Signatures are public ECDSA outputs. Trader private keys never touch the backend.

## Test matrix (15 tests, `src/api/perps_cosign.rs`)

| # | Predicate | Test |
|---|---|---|
| A | 10-field typehash matches `0xfb345c17…` | `a_typehash_matches_deployed_v1` |
| A' | Type string has exactly 10 fields, no bounds | `a_type_string_has_ten_fields_no_bounds` |
| B | executeTrade selector = `0x7a708c4c` | `b_execute_trade_selector_matches_deployed_v1` |
| C | Digest deterministic + 32-byte | `c_digest_deterministic` |
| D/E | UUID → bytes32 deterministic + distinct | `de_uuid_to_bytes32_deterministic` |
| G | Valid buyer + seller sigs recover correctly | `g_valid_cosign_signatures_recover_correctly` |
| H | Invalid buyer signature → `TraderMismatch` | `h_invalid_buyer_signature_rejects` |
| I | Invalid seller signature → `TraderMismatch` | `i_invalid_seller_signature_rejects` |
| J | Swapped signatures rejected | `j_swapped_signatures_reject` |
| K | Trade tampered after signing → mismatch | `k_tampered_trade_after_signing_rejects` |
| O | Wrong verifying contract → mismatch | `o_wrong_verifying_contract_rejects` |
| P | Wrong chain id → mismatch | `p_wrong_chain_id_rejects` |
| S | buyer == seller gate documented | `s_buyer_equals_seller_gate_documented` |
| X | Final calldata starts with `0x7a708c4c` selector | `x_final_calldata_decodes_to_frozen_trade` |
| Y | Receipt identity == expected UUID hash | `y_receipt_identity_uses_uuid_keccak` |

## Remaining STOP gates before real Base Sepolia settlement

1. Prepare one real Base Sepolia closed-test trade via `prepare_trade_core` — reviews exact typed data.
2. Operator reviews the typed data.
3. Manual co-sign of the SAME frozen `PerpTrade` by Trader A + Trader B (operator TTY password prompt on each keystore).
4. Pinned Base Sepolia fork rehearsal using those REAL signatures — verifies `executeTrade` succeeds against deployed bytecode.
5. Only after fork GREEN: configure real backend runtime (`PERPS_CLOSED_TEST_ENABLED=true`, `EXECUTION_ENABLED=true`, `EXECUTOR_REAL_BROADCAST_ENABLED=true`, signer triad, RPC, gas caps, DB URL).
6. Startup preflight + initial reconciliation must be GREEN before any real broadcast.
7. Only after runtime readiness GREEN: explicit authorization for one real `BroadcastPolicy::broadcast_intent` execution.

## Files changed

- **New**: `src/api/perps_cosign.rs` — core prepare + cosign logic + 15 tests.
- **New**: `docs/PERPS_BASE_SEPOLIA_CLOSED_TEST_COSIGN_ROUTE_V1.md` — this file.
- **Extended**: `src/execution/perp_trade.rs` — added `PERP_TRADE_V1_TYPE`, `PERP_TRADE_V1_TYPEHASH_HEX`, `perp_trade_v1_typehash`, `perp_trade_v1_digest`, `perp_trade_v1_digest_bytes`.
- **Extended**: `src/execution/mod.rs` — re-exports for V1 primitives.
- **Extended**: `src/api/mod.rs` — module registration.
