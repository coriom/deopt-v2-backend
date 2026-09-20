# PERPS_V2_BACKEND_DURABLE_BOUNDS_AND_RPC_READERS_V1 — supplement

Supplement to `PERPS_V2_BACKEND_EXECUTOR_PATH_V1` (commit `407ade4`).
Closes the two REAL correctness gaps identified in
`PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1`:

1. **§§1-2 durable V2 signed bounds** — `max/min_execution_price_1e8`
   were hardcoded to `0` in `ExecutionIntent::perp_trade_payload()`,
   silently dropping any V2 trader-signed price bounds after DB
   reload. This would produce an on-chain signature verification
   failure the first time a V2 intent with non-trivial bounds was
   cosigned.
2. **§5 real RPC-backed V2 readers** — the trait-based abstractions
   in `PERPS_V2_BACKEND_EXECUTOR_PATH_V1` had only mock impls. The
   live-chain implementations (`RpcV2EngineReader`,
   `RpcV2MatchingEngineReader`, `RpcV2VaultReader`) now exist with
   selectors derived from Solidity source via `alloy_sol_types::sol!`.

## Deferred to next milestone

Umbrella §§4, 6-11 (extending `script/DeployPerpsE2E.s.sol` with V2
contracts + Anvil `pg_integration` test running the full
non-broadcast lifecycle) are deferred to
`PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1`. That work is genuinely a full
sub-milestone: it requires ~500-line Solidity deploy-script
extension + ~800-line Rust integration test infrastructure. The
mock-based `rpc_end_to_end_preflight_against_mock` test proves the
composition of all real readers behaves exactly as the mock-based
preflight tests did, so the confidence loss from deferring the
Anvil wire-level tests is limited to "the JSON-RPC codec assumptions
against a running V2 chain" — which is precisely what
`PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1` will pin down.

## §1 — V2 signed-field durability audit

| Field                       | Source                                      | Durability path                                    |
|-----------------------------|---------------------------------------------|----------------------------------------------------|
| `intentId`                  | derived `keccak256(uuid)`                   | server UUID column                                 |
| `buyer`                     | persisted DB column                         | `execution_intents.buyer`                          |
| `seller`                    | persisted DB column                         | `execution_intents.seller`                         |
| `marketId`                  | persisted DB column                         | `execution_intents.market_id`                      |
| `sizeDelta1e8`              | persisted DB column                         | `execution_intents.size_1e8`                       |
| `executionPrice1e8`         | persisted DB column                         | `execution_intents.price_1e8`                      |
| `maxExecutionPrice1e8` ⭐   | **persisted DB column (NEW)**                | `execution_intents.max_execution_price_1e8`        |
| `minExecutionPrice1e8` ⭐   | **persisted DB column (NEW)**                | `execution_intents.min_execution_price_1e8`        |
| `buyerIsMaker`              | persisted DB column                         | `execution_intents.buyer_is_maker`                 |
| `buyerNonce`                | persisted DB column                         | `execution_intents.buyer_nonce`                    |
| `sellerNonce`               | persisted DB column                         | `execution_intents.seller_nonce`                   |
| `deadline`                  | persisted DB column (ms → sec conversion)   | `execution_intents.deadline_ms`                    |

⭐ = added in migration `0065_execution_intents_v2_price_bounds.sql`.

**Forbidden mutable reconstructions:** none. Every signed field is
either a persisted immutable column or a deterministic derivation
from a persisted immutable column. Runtime state (marks, oracle
prices, PME nonces, migration state) is NEVER used to reconstruct a
signed field after prepare-time.

## §2 — Migration + schema + model

```sql
ALTER TABLE execution_intents
    ADD COLUMN max_execution_price_1e8 TEXT NOT NULL DEFAULT '0';
ALTER TABLE execution_intents
    ADD COLUMN min_execution_price_1e8 TEXT NOT NULL DEFAULT '0';
```

`DEFAULT '0'` back-fills every pre-migration row to strict-price
semantics — safe because pre-migration rows are all
`protocol_version = 'perp_v1'` (migration `0064`) and V1 signatures
do not commit to bounds.

New fields on `ExecutionIntent`:

```rust
#[serde(default)]
pub max_execution_price_1e8: u128,
#[serde(default)]
pub min_execution_price_1e8: u128,
```

Both use `#[serde(default)]` for backward compat with legacy JSON.

`DbExecutionIntent` mirrors both as `String` (matches
`price_1e8`/`size_1e8` u128-wide TEXT convention).

Round-trip mappings updated in both directions
(`TryFrom<&ExecutionIntent>`, `TryFrom<DbExecutionIntent>`).

INSERT + all 6 SELECT statements updated to include the new columns.

## §2 — payload builder fix

`ExecutionIntent::perp_trade_payload()` now threads the persisted
bounds instead of hardcoding `0, 0`:

```rust
PerpTradePayload::new(
    intent_id, buyer, seller, market_id, size, price,
    self.max_execution_price_1e8,   // ← was 0
    self.min_execution_price_1e8,   // ← was 0
    buyer_is_maker, buyer_nonce, seller_nonce, deadline_sec,
)
```

V1 intents persist `0, 0` (via DB default) and reproduce V1
strict-price behaviour byte-for-byte. V2 intents persist the
trader-signed values.

## §5 — Real RPC readers

`src/execution/v2_readiness.rs::v2_abi` submodule generates selectors
via `alloy_sol_types::sol!` from the exact Solidity signatures:

```rust
sol! {
    function migrationState() external view returns (uint8);
    function migrationSnapshotHash() external view returns (bytes32);
    function clearingAccount() external view returns (address);
    function isExecutor(address) external view returns (bool);
    function nonces(address) external view returns (uint256);
    function perpEngine() external view returns (address);
    function paused() external view returns (bool);
    function balances(address, address) external view returns (uint256);
}
```

Three live-chain impls:

```rust
pub struct RpcV2EngineReader<R> { rpc: R, engine: AccountId }
pub struct RpcV2MatchingEngineReader<R> { rpc: R, pme: AccountId }
pub struct RpcV2VaultReader<R> { rpc: R, vault: AccountId }
```

Each impls the corresponding trait via `eth_call` against the
configured contract. Decoders fail closed on non-canonical
encodings (upper bytes non-zero on address/bool words, unknown
enum values, wrong return length).

## §3 — DB round-trip proofs

Four tests in `tests/perps_v2_backend_executor_path_v1_tests.rs`:

- `v2_bounds_survive_db_model_round_trip` — non-trivial bounds
  survive `ExecutionIntent → DbExecutionIntent → ExecutionIntent`
  byte-identical
- `v2_digest_survives_db_model_round_trip` — V2 EIP-712 digest
  reconstructed from the reloaded intent equals the pre-DB digest
- `v2_bounds_default_to_zero_for_v1_backfill` — V1 intent with
  zero bounds round-trips lossless
- `active_version_flip_after_reload_preserves_bounds` — persisted
  V2 intent survives simulated "shutdown + reload + runtime flip"

## §5 — Real RPC reader tests

Seven tests in `src/execution/v2_readiness.rs::tests`:

- `rpc_engine_reader_migration_state_sealed`
- `rpc_engine_reader_migration_state_open_and_invalid`
- `rpc_engine_reader_snapshot_hash_and_clearing_account`
- `rpc_pme_reader_is_executor_and_nonces_and_engine_and_paused`
- `rpc_vault_reader_balances`
- `rpc_readers_reject_non_canonical_encodings`
- `rpc_end_to_end_preflight_against_mock` — composes all three
  real readers through the preflight aggregator against a mock
  RPC provider; asserts exact call sequence (7 reads) + happy-path
  outcome

## Regression

```
cargo test --lib          → 1834 passed / 0 failed
cargo test --test perps_v2_backend_executor_path_v1_tests → 22 passed / 0 failed
cargo test --test perps_v2_backend_compat_foundation_v1_tests → 21 passed / 0 failed
```

Delta since the previous executor-path commit:
- Lib tests: 1827 → 1834 (+7 new RPC tests)
- Integration tests: 18 → 22 (+4 durability round-trip)
- Total green: **1877** (was 1866)

## Next milestone

**`PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1`** — extend
`script/DeployPerpsE2E.s.sol` with `PerpClearingAccountV2` +
`PerpEngineV2` + `PerpMatchingEngineV2` + `sealMigration` +
`fundClearing`. Extend `tests/perps_closed_test_e2e_harness.rs` to
configure the backend against local V2 addresses. New
`tests/perps_v2_backend_anvil_live_e2e_pg_integration.rs` running
the full prepare→cosign→REAL eth_call lifecycle against Anvil.
Migration-open negative test. Real-RPC clearing/executor gate tests.
Zero-send proof (Anvil executor nonce unchanged).

After that: `PERPS_V2_BACKEND_RECONCILIATION_V1` per the umbrella
spec.
