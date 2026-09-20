# PERPS_V2_BACKEND_ANVIL_LIVE_E2E_SCAFFOLDING_V1 — scaffolding

Scaffolding for the umbrella `PERPS_V2_BACKEND_ANVIL_LIVE_E2E_V1`
directive. Delivers the two safety-critical pieces that can be
proven WITHOUT running the full Anvil integration:

1. **§0 V1 bounds invariant** — every reconstruction path refuses a
   V1 intent that carries non-zero V2 price bounds.
2. **§§1-4 V2 deploy script** — `DeployPerpsV2E2E` compiles cleanly
   at Sol HEAD `2e9ad6f`, deploying the full production V2 stack:
   CollateralVault + PerpMarketRegistry + OracleRouter +
   PerpEngineV2 + PerpMatchingEngineV2 + PerpClearingAccountV2 +
   script-local `IPerpRiskModule` + Base-Sepolia-parity A/B seed +
   `sealMigration` + `fundClearing`.

## §§ deferred to `PERPS_V2_BACKEND_ANVIL_LIVE_E2E_INTEGRATION_V1`

- §§5-11 pg_integration test spawning Anvil + `forge script` + backend + HTTP prepare/cosign
- §§8, 12-14 Real RPC readers exercised against live Anvil V2 + `simulate_ok`
- §§15-19 Negative real-RPC tests (migration-open, clearing mismatch, clearing floor, executor-auth, PME-engine linkage)
- §§20-22 Zero-send + unchanged-state + API proofs against real chain state

The scaffolding leaves the follow-up milestone with pure integration
work: no new Rust struct fields, no new Solidity contracts, no new
migrations. The path is: extend `tests/perps_closed_test_e2e_harness.rs`
with a `spawn_v2` method that invokes the deploy script we ship
here, then write a single `perps_v2_backend_anvil_live_e2e_pg_integration.rs`
that runs the full non-broadcast lifecycle.

## §0 — V1 bounds invariant

Because backend uses a single 12-field `PerpTradePayload` for both
V1 and V2, a V1 intent with non-zero `max/min_execution_price_1e8`
is unrepresentable (V1 EIP-712 typehash does not commit to bounds).
Silently reinterpreting such a row would produce a V1 digest that
verified against nothing.

New method (`src/execution/intent.rs`):

```rust
pub fn validate_version_invariants(&self) -> Result<()> {
    if self.protocol_version == V1
        && (self.max_execution_price_1e8 != 0 || self.min_execution_price_1e8 != 0) {
        return Err(BackendError::Config("V1 intent has non-zero V2 bounds …"));
    }
    Ok(())
}
```

Called from `perp_trade_payload()` on EVERY reload — no bypass path.

Five tests in `src/execution/intent.rs::tests`:
- `v1_with_zero_bounds_is_valid`
- `v1_with_nonzero_max_bound_is_refused`
- `v1_with_nonzero_min_bound_is_refused`
- `v2_with_nonzero_bounds_is_valid`
- `v2_with_zero_bounds_is_valid`

All PASS.

## §§1-4 — V2 deploy script

`script/DeployPerpsE2E.s.sol` extended with:
- `contract DeployPerpsV2E2E is Script` — full V2 stack deployer
- `contract _PerpsV2E2EMockRisk is IPerpRiskModule` — script-local
  ABI-compatible mock risk module (no production PerpRiskModuleV2
  exists at Sol HEAD `2e9ad6f`)

Input env:
```
PERPS_V2_E2E_DEPLOY_ENABLED   (bool, required=true to run)
PERPS_V2_E2E_MANIFEST_PATH    (path, required)
DEPLOYER_PRIVATE_KEY          (Anvil #0)
PERPS_V2_E2E_TRADER_A         (Anvil #1)
PERPS_V2_E2E_TRADER_B         (Anvil #2)
PERPS_V2_E2E_EXECUTOR         (Anvil #3)
PERPS_V2_E2E_INITIAL_PRICE_1E8   (default 246_831_000_000)
PERPS_V2_E2E_CLEARING_FUND_RAW   (default 1_000_000_000_000 = 1M mUSDC)
PERPS_V2_E2E_SEAL_MIGRATION      (default true; set false for §15 negative fixture)
```

Deploy sequence:
1. Mock USDC (6-dec) + WETH (18-dec)
2. Dual `MockPriceSource` + `OracleRouter`
3. `CollateralVault(deployer)` + `PerpMarketRegistry(deployer)`
4. `PerpEngineV2(deployer, registry, vault, router)`
5. `_PerpsV2E2EMockRisk(usdc, 6)` — script-local
6. `PerpMatchingEngineV2(deployer, engine)` + `setExecutor(runtime, true)`
7. Vault: `setCollateralToken(usdc)` + `setAuthorizedEngine(engine)`
8. Registry: `setSettlementAssetAllowed(usdc)` + `createMarket` + `setMaxExecutionDeviationBps`
9. Engine: `setMatchingEngine` + `setRiskModule` + `setClearingAccount`
10. Mark A/B/clearing accounts trivially healthy via `MockRisk.setHealthy`
11. Mint USDC to deployer + `PerpClearingAccountV2.fundClearing(usdc, amount)` — canonical funding path
12. Seed Base-Sepolia-parity state via `adminSeedMarketFunding` + `adminSeedPosition(A|B)`
13. `sealMigration(deterministic hash)` (unless `SEAL_MIGRATION=false`)

Manifest fields written for the Rust harness to consume:
```
chainId, marketId, initialPrice1e8, clearingFundRaw, sealed, sealHash,
deployer, usdc, weth, primarySource, secondarySource, oracleRouter,
vault, perpMarketRegistry, perpEngineV2, perpMatchingEngineV2,
perpClearingAccountV2, risk, traderA, traderB, executor
```

`forge build` on the sol repo compiles the new contract at
`out/DeployPerpsE2E.s.sol/DeployPerpsV2E2E.json` and
`out/DeployPerpsE2E.s.sol/_PerpsV2E2EMockRisk.json` cleanly.

## Regression

```
cargo test --lib intent::tests
28 passed; 0 failed; 0 ignored
```

- +5 new V1 bounds invariant tests
- Zero regressions
- No cargo build cycle needed (lib test only recompiles intent.rs)

## Next milestone

**`PERPS_V2_BACKEND_ANVIL_LIVE_E2E_INTEGRATION_V1`** — pure Rust
integration work:

1. Extend `tests/perps_closed_test_e2e_harness.rs` with:
   - `V2DeployedAddresses` struct mirroring the manifest fields
   - `E2eEnv::spawn_v2(...)` method that sets the V2 env vars +
     invokes `forge script DeployPerpsV2E2E` + parses the manifest
   - Backend config extension: `PERPS_ACTIVE_ENGINE_VERSION=v2` +
     the V2 contract addresses

2. New `tests/perps_v2_backend_anvil_live_e2e_pg_integration.rs`:
   - `#[ignore]` unless `PERPS_E2E_ENABLE=1` (existing convention)
   - Spawn PG + Anvil + V2 stack + backend
   - HTTP prepare V2 with non-trivial bounds
   - Sign with ephemeral Anvil trader keys
   - HTTP cosign after simulated DB reload (drop repository handle
     between prepare and cosign)
   - HTTP simulate → `simulation_status == simulation_ok`
   - Independent calldata decode via
     `decode_execute_trade_v2_calldata`
   - Assertions: executor nonce unchanged, positions unchanged,
     `execution_transactions` empty, `broadcasts` empty

3. Five negative tests re-using the deploy script's
   `PERPS_V2_E2E_SEAL_MIGRATION=false` mode + per-test config
   perturbations (wrong clearing address, insufficient balance,
   executor auth false, PME-engine linkage mismatch).

Estimated 8-15 hours of focused work with 1-2 cargo debug cycles.
