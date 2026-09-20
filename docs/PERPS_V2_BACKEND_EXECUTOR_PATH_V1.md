# PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — design + tests

Second milestone in the `PERPS_V2_BACKEND_COMPATIBILITY` sequence.
Builds on `PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1` (foundation) by
adding the **non-broadcast** V2 executor path: prepare → typed data
→ digest → cosign → ABI-level V2 calldata → readiness preflight.

**What this milestone delivers**

| Umbrella § | Deliverable                                                                                       | Status |
|------------|---------------------------------------------------------------------------------------------------|--------|
| §1         | Exact V2 12-field `PerpTrade` wire shape frozen (typehash, field order, wire strings)             | ✅     |
| §14        | V2 `executeTrade` calldata encoder + independent decode round-trip proof                          | ✅     |
| §9         | V2 prepare dispatches domain + digest + typed-data + reader targets on active version             | ✅     |
| §10        | V2 PME nonces read fresh (not migrated from V1)                                                   | ✅     |
| §11        | V2 typed data / digest byte-equality across HTTP typedData, cosign, runtime paths                 | ✅     |
| §12        | V2 cosign dispatches on `intent.protocol_version` (not runtime active)                            | ✅     |
| §13        | Active-version flip preserves persisted intent version (retargeting is impossible)                | ✅     |
| §§5-8      | V2 on-chain reader traits + preflight primitives (mock-tested)                                    | ✅     |
| §§17-19    | Fail-closed preflight aggregator with 11 distinct denial branches                                 | ✅     |
| §20        | V1/V2 target matrix — cross-version replay refused at every layer                                 | ✅     |
| §21        | Intent `protocol_version` surfaced via `#[derive(Serialize)]` on `ExecutionIntent`                | ✅     |
| §22        | DB round-trip / serde default for pre-migration rows                                              | ✅     |
| §23        | 39 unit tests + 18 integration tests, all green                                                   | ✅     |

**What this milestone does NOT deliver**

| Umbrella § | Deferred to                                                                                                                                 |
|------------|---------------------------------------------------------------------------------------------------------------------------------------------|
| §15-16     | Real local `eth_call` simulation against a deployed local V2 fixture. Requires extending `script/DeployPerpsE2E.s.sol` + `tests/perps_closed_test_e2e_harness.rs`. Sub-milestone `PERPS_V2_BACKEND_ANVIL_E2E_V1`. |
| §16        | Backend production `RpcV2EngineReader` / `RpcV2PmeReader` / `RpcV2VaultReader` impls (the trait interfaces + mock-based tests are done here; the real live-RPC impl lands with the E2E harness).                     |
| §17        | Live migration-open negative simulation. Trait-level refusal is proven; live wire-level refusal against a running Anvil V2 goes in `PERPS_V2_BACKEND_ANVIL_E2E_V1`.                                                    |
| §16-18     | Broadcast / `sendRawTransaction` / LocalKeystore. No milestone in this umbrella covers these until `PERPS_V2_BACKEND_ANVIL_E2E_V1` succeeds — broadcast requires that E2E to pass first.                                |

---

## §1 — V2 wire shape (frozen)

```
V2 PerpTrade (12-field), typehash 0x9ccd368c748c5e85df8e96f94ac1d47316abde07a2d78c4f1b10b91cb98942c3:

    bytes32 intentId
    address buyer
    address seller
    uint256 marketId
    uint128 sizeDelta1e8
    uint128 executionPrice1e8
    uint128 maxExecutionPrice1e8   ← V2-only
    uint128 minExecutionPrice1e8   ← V2-only
    bool    buyerIsMaker
    uint256 buyerNonce
    uint256 sellerNonce
    uint256 deadline

V1 PerpTrade (10-field, deployed Base Sepolia), typehash 0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293:

    bytes32 intentId
    address buyer
    address seller
    uint256 marketId
    uint128 sizeDelta1e8
    uint128 executionPrice1e8
    bool    buyerIsMaker
    uint256 buyerNonce
    uint256 sellerNonce
    uint256 deadline
```

Both typehashes are pinned in `src/execution/perp_trade.rs` and regression-tested against the wire string in `tests/perps_v2_backend_executor_path_v1_tests.rs::v2_typehash_matches_solidity_perp_matching_engine_v2`.

## §§2-4 — Strongly-typed V2 payload

**Decision:** kept the existing single-struct `PerpTradePayload` with 12 fields (V1 zeros the bounds slots). Rejected the enum refactor `enum PerpTradePayload { V1, V2 }` for this milestone — the tag-based dispatchers introduced in the foundation (`perp_trade_digest_for_version`, `encode_execute_trade_calldata_for_version`) already prevent cross-version confusion without paying the invasive refactor cost. The `#[serde(default)]` on `ExecutionIntent.protocol_version` + version-aware config resolvers + dispatch-refusing digest helpers close the "impossible to compute V1 digest from V2 payload accidentally" invariant equivalently.

If a future milestone needs to carry V2-only fields that CANNOT be represented as "V1 field with zero default" (e.g. an entirely new struct member), that will force the enum split. The current 12-field shape does not require it.

## §§5-8 — V2 on-chain readers

`src/execution/v2_readiness.rs`:

```rust
pub trait V2EngineReader {
    fn read_migration_state(&self) -> ReaderFuture<'_, MigrationState>;
    fn read_migration_snapshot_hash(&self) -> ReaderFuture<'_, [u8; 32]>;
    fn read_clearing_account(&self) -> ReaderFuture<'_, AccountId>;
}

pub trait V2MatchingEngineReader {
    fn read_is_executor(&self, runtime: &AccountId) -> ReaderFuture<'_, bool>;
    fn read_v2_nonce(&self, trader: &AccountId) -> ReaderFuture<'_, u128>;
    fn read_configured_engine(&self) -> ReaderFuture<'_, AccountId>;
    fn read_paused(&self) -> ReaderFuture<'_, bool>;
}

pub trait V2VaultReader {
    fn read_clearing_settlement_balance(&self, clearing: &AccountId, asset: &AccountId)
        -> ReaderFuture<'_, u128>;
}
```

Real RPC impls (`RpcV2EngineReader` etc.) are deferred to `PERPS_V2_BACKEND_ANVIL_E2E_V1` where a running local V2 stack can drive the wire-level codec tests.

## §§17-19 — Fail-closed preflight aggregator

`v2_preflight_check(config, intent_version, runtime_executor, settlement_asset, engine, pme, vault) -> V2PreflightReport`

Denial branches (short-circuits at the first failure, RPC calls skipped after):

1. `IntentIsV1` — this preflight is V2-only
2. `ActiveVersionNotV2` — runtime not on V2
3. `ConfigMissing(...)` — any of the three V2 addresses unset or zero
4. `MigrationOpen` — `PerpEngineV2.migrationState() != Sealed`
5. `SealedButSnapshotHashZero` — Solidity-side invariant violation
6. `ClearingAccountMismatch` — `engine.clearingAccount() != config.PERP_CLEARING_ACCOUNT_V2_ADDRESS`
7. `PmeEngineLinkageMismatch` — `pme.perpEngine() != config.PERP_ENGINE_V2_ADDRESS`
8. `ExecutorNotAuthorized` — `pme.isExecutor(runtime) == false`
9. `PmePaused` — `pme.paused() == true`
10. `ClearingBalanceBelowFloor` — vault settlement balance held by clearing < `PERPS_V2_CLEARING_MIN_BALANCE_RAW`
11. `UpstreamRpcError` — any of the reads failed

Each denial has a dedicated unit test that mocks the exact upstream failure and asserts the outcome enum matches. `MigrationState::parse_u8` fails closed on any wire value ∉ {0, 1}.

**Clearing floor is operational, not exact.** The exact per-trade solvency proof lives in `PerpMatchingEngineV2._executeSingle` → `PerpEngineV2.applyTrade`, which the backend validates via `eth_call` simulation. The backend floor is an operator safety net for "gross under-funding" — not a duplication of the on-chain economics.

## §§9-13 — V2 prepare / cosign dispatch

**Prepare (`src/api/perps_cosign.rs::prepare_trade_core`)** dispatches on the RUNTIME active version:

```rust
let active_version = state.execution_config.perps_active_engine_version;
let verifying_contract = state.execution_config.active_perp_matching_engine_address()?.clone();
let domain = PerpTradeDomain::for_version(active_version, chain_id, verifying_contract);
let digest_hex = match active_version {
    V1 => perp_trade_v1_digest(&payload, &domain)?,
    V2 => perp_trade_v2_digest(&payload, &domain)?,
};
let typed_data = match active_version {
    V1 => build_typed_data_v1(...),
    V2 => build_typed_data_v2(...),
};
```

**Cosign (`src/api/perps_cosign.rs::cosign_load_and_verify` + `cosign_verify_core_for_version`)** dispatches on the PERSISTED `intent.protocol_version`. Runtime active version is IRRELEVANT to cosign — this is the invariant that prevents a cutover flip from retargeting an already-signed intent.

Nonce reader construction in `src/api/routes.rs` uses the version-aware config resolvers `active_perp_matching_engine_address()` / `active_perp_engine_address()`. Under active=v1 these resolve to V1 addresses (backward-compat); under active=v2 they resolve to V2.

## §21 — API delta

`ExecutionIntent` is `#[derive(Serialize)]` with `protocol_version` serialised as lowercase `"v1"` / `"v2"` (via `#[serde(rename_all = "snake_case")]` on the enum). Any endpoint that returns an `ExecutionIntent` in JSON now surfaces the field. Backward compat: deserialise from legacy JSON without the field defaults to V1 (`#[serde(default)]`).

## §22 — DB round-trip

Migration `0064_execution_intents_protocol_version.sql` (delivered in foundation) added the column. `DbExecutionIntent.protocol_version: String` maps via `TryFrom<DbExecutionIntent>` for `ExecutionIntent` and back through `TryFrom<&ExecutionIntent>` for `DbExecutionIntent`. Round-trip lossless — proved by `execution_intent_serde_round_trips_v2_protocol_version`. Pre-migration rows deserialise to V1 (proved by `execution_intent_serde_defaults_missing_protocol_version_to_v1`).

## §23 — Test results

```
cargo test --lib
1827 passed; 0 failed; 0 ignored

cargo test --test perps_v2_backend_executor_path_v1_tests
18 passed; 0 failed; 0 ignored

cargo test --test perps_v2_backend_compat_foundation_v1_tests
21 passed; 0 failed; 0 ignored
```

Delta since foundation:
- Lib tests: 1805 → 1827 (+22: 14 v2_readiness + 8 abi V2)
- Integration tests: 21 → 39 (+18 executor-path)
- Total green: 1866 (was 1826)

---

## Next milestone

`PERPS_V2_BACKEND_ANVIL_E2E_V1` — spec §§15-18. Extend `script/DeployPerpsE2E.s.sol` to deploy V2 (`PerpClearingAccountV2` + `PerpEngineV2` + `PerpMatchingEngineV2` + seal migration). Extend `tests/perps_closed_test_e2e_harness.rs` to configure the backend against the deployed V2 addresses. Write a new pg_integration test that runs the full prepare→cosign→simulation lifecycle on Anvil with a REAL `eth_call` against the deployed V2 PME. Add the migration-open negative case (deploy but do NOT seal → preflight refuses).

That milestone completes the "V2 non-broadcast" path (spec §§16-18). Broadcast (§§19-27 in the umbrella) is `PERPS_V2_BACKEND_BROADCAST_V1` — deliberately deferred until Anvil E2E proves the read/simulate loop is correct.
