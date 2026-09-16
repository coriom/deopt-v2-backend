# PERPS_BASE_SEPOLIA_BACKEND_RECEIPT_IDENTITY_BINDING_V1

Closes the "correct emitter + correct topic0 but wrong execution
identity" gap. Confirmation now requires cryptographic proof that the
PerpMatchingEngine emitted the event for exactly the persisted
execution being finalized.

## Solidity event ABIs (authoritative)

From
[`src/matching/PerpMatchingEngine.sol`](../../deopt-v2-sol/src/matching/PerpMatchingEngine.sol):

```solidity
event TradeExecuted(
    bytes32 indexed intentId,
    address indexed buyer,
    address indexed seller,
    uint256 marketId,
    uint128 sizeDelta1e8,
    uint128 executionPrice1e8,
    bool buyerIsMaker,
    uint256 buyerNonce,
    uint256 sellerNonce
);

event TradeExecutedFromIntents(
    bytes32 indexed buyerIntentHash,
    bytes32 indexed sellerIntentHash,
    uint256 marketId,
    uint128 size1e8,
    uint128 executionPrice1e8,
    uint256 timestamp
);
```

Topic layout:

| Event | topic0 | topic1 | topic2 | topic3 |
|---|---|---|---|---|
| `TradeExecuted` | `0x5018a0a7…dfedb3f80` | `intentId` (bytes32) | `buyer` (address) | `seller` (address) |
| `TradeExecutedFromIntents` | `0x560ebd5f…24342440` | `buyerIntentHash` (bytes32) | `sellerIntentHash` (bytes32) | — |

## Backend → on-chain identity mapping

For the **pre-matched** path (`executeTrade`, driven by the current
`BroadcastPolicy`):

```
backend intent.intent_id: Uuid
    → intent_id.to_string() (RFC-4122 hyphenated hex)
    → keccak256(uuid_string.as_bytes()) : [u8; 32]
    → PME `TradeExecuted.topic[1]`
```

Helper: `expected_intent_hash_from_uuid(intent_id: Uuid) -> [u8; 32]`
(mirrors the on-chain `intent_id_to_b256` used by
`build_perp_execution_call_from_intent`).

For the **intent-based** path (`executeTradeFromIntents`, not
currently exercised by `BroadcastPolicy`), the identity is a pair of
EIP-712 `PerpOrderIntent` digests (buyer + seller). Requires BOTH
hashes to match `topic[1]` + `topic[2]` of `TradeExecutedFromIntents`.

## Strict receipt verification algorithm

```
verify_pme_event_in_receipt(receipt, expected_pme, expected_identity):
  IF receipt.logs is empty  → BroadcastRejected("no logs")
  FOR each log in receipt.logs:
    IF log.address != expected_pme  → skip
    IF log.topics[0] != expected_topic0(expected_identity)  → skip
    IF log.topics.len() < min_topics(expected_identity)  → mark saw_emitter_topic0, skip
    match expected_identity:
      PreMatchedIntent { intent_id }:
        IF topic_matches_bytes32(log.topics[1], intent_id)  → return Ok(())
      FromIntents { buyer_intent_hash, seller_intent_hash }:
        IF topic_matches_bytes32(log.topics[1], buyer_intent_hash)
           AND topic_matches_bytes32(log.topics[2], seller_intent_hash)  → return Ok(())
    mark saw_emitter_topic0
  IF saw_emitter_topic0  → BroadcastRejected("identity mismatch")
  ELSE  → BroadcastRejected("no matching PME event")
```

Key properties:

- **Emitter check is strict**: only logs from `config.perp_matching_engine_address` count.
- **Topic0 check is strict**: only the event selector for the identity variant counts.
- **Identity check is strict**: `topic[1]` (and `topic[2]` for FromIntents) MUST decode to exactly the expected value.
- **Multi-event tolerant**: a single tx may emit multiple PME events; we require ≥1 EXACT match.
- **Malformed-input safe**: short topic lists / non-hex chars return `false` instead of panicking.
- **Deterministic**: same input → same verdict every time (reconciler restart safety).

## Failure taxonomy

| Scenario | Verdict | Reason string contains |
|---|---|---|
| No logs at all | BroadcastRejected | `no logs` |
| No log from PME with matching topic0 | BroadcastRejected | `no matching PME event` |
| Log from PME with matching topic0 but wrong identity | BroadcastRejected | `identity mismatch` |
| Short/malformed topic list | BroadcastRejected | `no matching PME event` OR `identity mismatch` (never panic) |
| ≥1 log with exact match | Ok — advances to Confirmed |

## Wire-up in finalize_receipt

`BroadcastPolicy::finalize_receipt` (composite check):

1. `receipt.tx_hash == expected_tx_hash` (else mark Failed)
2. `receipt.status == 1` (else mark Failed)
3. IF `verify_pme_event == true`:
   - build `ExpectedExecutionIdentity::PreMatchedIntent { intent_id: expected_intent_hash_from_uuid(intent_id) }`
   - call `verify_pme_event_in_receipt(receipt, config.perp_matching_engine_address, &identity)`
   - IF verifier returns Err → mark Failed with reason `"semantic_event_verification: …"`
4. Only when ALL of (1), (2), (3) pass → mark Confirmed with receipt block.

Confirmed/Failed monotonicity (from V1 durability milestone) prevents
regressing after this call.

## Test matrix

Isolated verifier tests (13 in `src/execution/broadcast_policy.rs`
under `#[cfg(test)] mod tests`):

| # | Test | Assertion |
|---|---|---|
| A | `identity_a_correct_intent_id_confirms` | correct emitter + topic0 + intent_id → Ok |
| B | `identity_b_wrong_intent_id_fails` | correct emitter + topic0, wrong intent_id → Err(identity mismatch) |
| C | `identity_c_wrong_emitter_fails` | correct topic0 + intent_id but wrong emitter → Err(no matching PME event) |
| D | `identity_d_wrong_topic0_fails` | correct emitter + intent_id but wrong topic0 → Err(no matching PME event) |
| E | `identity_e_multi_event_one_match_confirms` | 2 PME logs, one exact → Ok |
| F | `identity_f_multi_event_none_match_fails` | 2 PME logs, none exact → Err(identity mismatch) |
| G | `identity_g_malformed_topics_fails_safely` | short topic list AND non-hex topic → Err (no panic) |
| H | `identity_h_status_zero_isolated_semantics` | verifier is status-agnostic (composite check gates status) |
| I | `identity_i_verifier_ignores_tx_hash` | verifier scope excludes tx_hash |
| J | `identity_j_verifier_deterministic` | same input → same verdict twice |
| K | `identity_k_from_intents_requires_both_hashes` | FromIntents identity variant matches topic1 + topic2 |
| L | `identity_l_topic_matches_bytes32_safety` | case-insensitive hex, wrong length, non-hex all handled safely |

Plus 5 T-series composite tests exercising the full
`broadcast_intent → finalize_receipt` path.

## Access-gate posture (unchanged)

- **Public Perps remains OFF.**
- **Funding remains OFF.**
- **Collateral activation unchanged.**
- **Base mainnet untouched.**

## Follow-on work

1. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`** — fund
   closed-test traders with mUSDC collateral. See READ-ONLY overview.
2. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_SMOKE_ORDER_V1`** — end-to-end
   real trade against Base Sepolia PME.
3. **Intent-based path exercise** — when `BroadcastPolicy` gains an
   `executeTradeFromIntents` code path, thread the
   `ExpectedExecutionIdentity::FromIntents` variant through
   `finalize_receipt`.
