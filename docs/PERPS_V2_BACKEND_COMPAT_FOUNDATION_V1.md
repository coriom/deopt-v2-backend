# PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — design + runbook

Scope: **foundation layer** of the larger
`PERPS_V2_BACKEND_COMPATIBILITY` initiative. This milestone adds the
minimal set of primitives required to safely coexist V1 (deployed)
and V2 (future) Perps stacks in one backend process, **without**
enabling any V2 execution path yet.

Follow-up milestones will layer on the remaining sections of the
umbrella spec:

| Follow-up milestone                             | Umbrella §§ | Scope                                                    |
|-------------------------------------------------|-------------|----------------------------------------------------------|
| `PERPS_V2_BACKEND_EXECUTOR_PATH_V1`             | §§6-11      | V2 migration-state reads; clearing-liquidity preflight; V2 prepare/cosign/simulation/executor preflight dispatch |
| `PERPS_V2_BACKEND_RECONCILIATION_V1`            | §§12-14     | Version-aware receipt/emitter binding; durable reconciliation version-binding; indexer multi-emitter |
| `PERPS_V2_BACKEND_ANVIL_E2E_V1`                 | §§15-18     | Anvil V2 deployment fixture; full E2E close reproducing 244_274 mUSDC; migration-open + V1-legacy negative tests |

This foundation milestone is a **pure additive layer**: no
runtime behavior changes when `PERPS_ACTIVE_ENGINE_VERSION` is not
set. Every existing intent, receipt, and reconciliation continues
to route through the V1 code path unchanged.

---

## §1 — V1-coupling inventory (delivered)

Full report in the conversation record; concise summary:

**Address / config**
- `PERP_ENGINE_ADDRESS` → `ExecutionConfig::perp_engine_address` (`src/execution/config.rs:47`)
- `PERP_MATCHING_ENGINE_ADDRESS` → `ExecutionConfig::perp_matching_engine_address` (`src/execution/config.rs:46`)
- Consumed by `RpcNonceReader`, `RpcMarkPriceReader`, `simulate_execution_intent`, executor preflight
- No version selector — active routing is a single address

**EIP-712**
- Deployed V1 (Base Sepolia `0x774d96…F165`): 10-field `PerpTrade`, domain `("DeOptV2-PerpMatchingEngine", "1")`, TYPEHASH `0xfb345c17…8293`
- Future V2 (`PerpMatchingEngineV2.sol` at sol HEAD `2e9ad6f`): 12-field `PerpTrade`, domain `("DeOptV2-PerpMatchingEngine", "2")`, TYPEHASH `0x9ccd368c…42c3`
- Backend already ships both typehashes: `PERP_TRADE_V1_TYPEHASH_HEX` (10-field) and `PERP_TRADE_TYPEHASH_HEX` (12-field) in `src/execution/perp_trade.rs`

**Persistence**
- `execution_intents` table (migration `0001_init.sql`): no version column pre-foundation
- Intent lifecycle: `Pending → CalldataReady → SimulationOk → Prepared → Submitted → Confirmed` (`src/execution/intent.rs:9-36`)
- UUIDs generated server-side (client-supplied UUID rejected for closed-test)

**Runtime**
- Executor address env var: `HV2_EXECUTOR_ADDRESS`
- Preflight: `hybrid_v2/execution/preflight.rs` checks readiness snapshot; NO explicit `PME.isExecutor(runtime)` today
- Simulation: `hybrid_v2/execution/simulator.rs` eth_call against `plan.target`
- Receipt correlation: `hybrid_v2/execution/broadcast_indexer_correlation.rs` matches by `tx_hash`; emitter address bound at manifest-config time
- Reconciliation: `hybrid_v2/execution/broadcast_outbox.rs:771-841` — 14-phase state machine persisted in `hybrid_v2_broadcast_state`

**Arming gate**
- `PERPS_CLOSED_TEST_BROADCAST_ARMED` + `PERPS_CLOSED_TEST_BROADCAST_INTENT_ID` (`src/execution/config.rs:85-96`)

---

## §2 — Versioned contract config (delivered)

New env vars (all optional; default `None`):

```
PERP_ENGINE_V2_ADDRESS=<0x…>            # PerpEngineV2 deployment
PERP_MATCHING_ENGINE_V2_ADDRESS=<0x…>   # PerpMatchingEngineV2 deployment (EIP-712 verifyingContract)
PERP_CLEARING_ACCOUNT_V2_ADDRESS=<0x…>  # PerpClearingAccountV2 deployment
PERPS_ACTIVE_ENGINE_VERSION=v1|v2       # runtime active version (default v1)
PERPS_V2_CLEARING_MIN_BALANCE_RAW=0     # optional operational floor
```

New `ExecutionConfig` fields:

```rust
pub perp_engine_v2_address: Option<AccountId>
pub perp_matching_engine_v2_address: Option<AccountId>
pub perp_clearing_account_v2_address: Option<AccountId>
pub perps_active_engine_version: PerpsProtocolVersion  // default V1
pub perps_v2_clearing_min_balance_raw: u128
```

New `ExecutionConfig::validate_startup` guardrails:
- `PERPS_ACTIVE_ENGINE_VERSION=v2` refused unless all three V2
  addresses are populated + non-zero
- V1/V2 addresses forbidden from colliding (`PERP_ENGINE_V2_ADDRESS == PERP_ENGINE_ADDRESS` refused; same for PME)

Version-aware address resolvers (used by follow-up milestones):
- `ExecutionConfig::active_perp_engine_address()` — for NEW intents
- `ExecutionConfig::active_perp_matching_engine_address()` — for NEW intents
- `ExecutionConfig::perp_engine_address_for(version)` — for RECONCILIATION reads (intent's persisted version is authoritative)
- `ExecutionConfig::perp_matching_engine_address_for(version)` — mirror

---

## §3 — EIP-712 versioning (delivered)

**`PerpTradeDomain` constructors:**

```rust
PerpTradeDomain::new(chain_id, addr)               // legacy alias for new_v1
PerpTradeDomain::new_v1(chain_id, addr)            // version = "1"
PerpTradeDomain::new_v2(chain_id, addr)            // version = "2"
PerpTradeDomain::for_version(v, chain_id, addr)    // version-dispatched
```

**V2 digest helpers (new):**

```rust
perp_trade_v2_digest(&payload, &domain) -> Result<String>
perp_trade_v2_digest_bytes(&payload, &domain) -> Result<[u8; 32]>
```

Both refuse a V1-versioned domain with an explicit `BackendError::Config` — prevents the class of bug where a caller reconstructs the V2 digest against a V1 domain and gets a hash that verifies against nothing.

**Version-aware dispatchers (new):**

```rust
perp_trade_digest_for_version(&payload, &domain, version) -> Result<String>
perp_trade_digest_bytes_for_version(&payload, &domain, version) -> Result<[u8; 32]>
```

Both refuse mismatched `(version, domain.version)` pairs.

---

## §4 — Intent protocol_version (delivered)

**Enum** (`src/execution/perp_trade.rs`):

```rust
pub enum PerpsProtocolVersion { V1, V2 }

impl PerpsProtocolVersion {
    pub const fn as_persisted_str(self) -> &'static str;   // "perp_v1" / "perp_v2"
    pub const fn domain_version_str(self) -> &'static str; // "1" / "2"
    pub fn parse(raw: &str) -> Result<Self>;               // accepts perp_v1/v1/1 + perp_v2/v2/2
}
impl Default for PerpsProtocolVersion { fn default() -> Self { Self::V1 } }
```

**Field** on `ExecutionIntent`:

```rust
pub protocol_version: PerpsProtocolVersion  // default V1 via #[serde(default)]
```

**DB migration** `0064_execution_intents_protocol_version.sql`:

```sql
ALTER TABLE execution_intents
    ADD COLUMN protocol_version TEXT NOT NULL DEFAULT 'perp_v1'
    CHECK (protocol_version IN ('perp_v1', 'perp_v2'));

CREATE INDEX idx_execution_intents_protocol_version
    ON execution_intents (protocol_version);
```

Pre-migration rows deterministically back-fill to `perp_v1` (the deployed V1 stack).

**Immutability invariant** (documented in code + migration COMMENT): flipping `PERPS_ACTIVE_ENGINE_VERSION` MUST NOT retarget a persisted intent. Enforced by `perp_engine_address_for(intent.protocol_version)` at read time; runtime `active_perp_engine_address()` is used ONLY at NEW-intent creation.

**Runtime pin at prepare time** (`src/api/perps_cosign.rs`): a new intent's `protocol_version` is stamped from `state.execution_config.perps_active_engine_version`.

---

## §5 — Cross-version replay isolation tests (delivered)

`tests/perps_v2_backend_compat_foundation_v1_tests.rs` — 20 tests:

1. V1 and V2 typehashes distinct
2. V1/V2 domain separators distinct (same addr, distinct version)
3. V1/V2 domain separators distinct (distinct addr, realistic)
4. V1 digest bytes ≠ V2 digest bytes for same payload
5. `perp_trade_v2_digest` refuses V1 domain
6. `perp_trade_v2_digest_bytes` refuses V1 domain
7. Dispatcher(V1, domain_v2) fails closed
8. Dispatcher(V2, domain_v1) fails closed
9. Dispatcher bytes (V2, domain_v1) fails closed
10. Dispatcher(V1, domain_v1) matches direct V1 digest
11. Dispatcher(V2, domain_v2) matches direct V2 digest
12-14. `PerpsProtocolVersion::parse` — canonical, aliases, rejection
15-16. `for_version` selects right version string
17-18. Persisted wire form stable (`perp_v1` / `perp_v2`)
19. Default is V1
20-21. Domain-version-string invariant (V1="1", V2="2")

---

## §22 — Corrected Base Sepolia deployment/cutover runbook

**Ordering rationale change vs the prior sketch:** the old runbook froze V1 first, then deployed V2. This left the shared Vault authorised only for V1 during the deploy window — any V2 configuration failure meant redoing the freeze. The corrected order pre-stages V2 with NO downtime, then executes a tight cutover.

### PHASE A — pre-stage V2 (NO downtime)

**Precondition:** V1 continues to serve trading and reconciliation exactly as today. Backend `PERPS_ACTIVE_ENGINE_VERSION` is unset or `v1`. No user traffic hits any V2 surface.

1. **Deploy V2 contracts** in one atomic sequence:
   - `PerpClearingAccountV2(vault)` → address `C`
   - `PerpEngineV2(owner, registry, vault, oracle)` → address `E`
   - `PerpMatchingEngineV2(owner, engine=E)` → address `P`
2. **Verify bytecode** via `cast code` against the artifact built at sol HEAD `2e9ad6f`.
3. **Configure V2 dependencies:**
   - `E.setGuardian(OWNER)`
   - `E.setMatchingEngine(P)`
   - `E.setRiskModule(risk)`
   - `E.setClearingAccount(C)`
   - `registry.setMaxExecutionDeviationBps(marketId, ...)`
4. **DO NOT authorise V2 on shared Vault yet.** V2 must remain economically inert until cutover step 17.
5. **Migration stays OPEN** — no seeding, no seal.
6. **DO NOT route users/backend to V2.**
   - Backend env: keep `PERPS_ACTIVE_ENGINE_VERSION=v1`.
   - `PERP_ENGINE_V2_ADDRESS`, `PERP_MATCHING_ENGINE_V2_ADDRESS`, `PERP_CLEARING_ACCOUNT_V2_ADDRESS` populated but inert (they only feed reads/preflight, not routing, while active=v1).

**Exit criterion:** V1 unchanged; V2 deployed but authority-less and empty.

### PHASE B — cutover (tight window)

7. **Freeze V1 new trading/matching** at the trading engine layer (existing runtime arming gate; do NOT touch V1 contracts).
8. **Confirm no durable/backend/on-chain V1 in-flight execution:**
   - Backend: no rows in `execution_intents.status IN ('pending', 'calldata_ready', 'simulation_ok', 'prepared', 'submitted')`
   - `hybrid_v2_broadcast_state`: no rows in `Broadcasting`, `SubmissionUnknown`, `Pending`, `Submitted`
   - On-chain: no pending V1 executor tx in mempool
9. **Pin final snapshot block** — read chain HEAD, record `snapshot_block`.
10. **Construct canonical snapshot manifest** — per §22 of `PERPS_V2_MIGRATION_SEED_HOOK_V1.md`.
11. **Verify snapshot twice independently** (different operators, different RPC endpoints).
12. **Fund clearing** through canonical path: `C.fundClearing(usdc, amount)` — separate operation, NOT via `adminSeed*`.
13. **Seed market funding** — for each market: `E.adminSeedMarketFunding(marketId, cumulativeFundingRate, lastFundingTimestamp)`.
14. **Seed positions / residual bad debt** — for each open position: `E.adminSeedPosition(trader, marketId, size, openNotional, lastCumulativeFundingRate)`. For any residual bad debt trader: `E.adminSeedResidualBadDebt(trader, amount)`.
15. **Verify V2 state vs snapshot** — every seeded position matches; aggregate OI matches; clearing balance matches expected funding amount.
16. **Seal migration:** `E.sealMigration(snapshotHash)` — irreversible.
17. **Transition Vault authority from V1 to V2** with the smallest possible dual/no-authority window:
    - `vault.setAuthorizedEngine(E)` (V2 gains authority; V1 loses it in the same tx if the Vault permits — otherwise use a Timelock/Safe atomic bundle)
    - If atomic swap is impossible, prefer NO-AUTHORITY window over DUAL-AUTHORITY window (users experience temporary inability to open new positions rather than double-write risk)
18. **Prove V1 can no longer mutate collateral** — attempt `V1_engine.applyTrade(...)` from any authorized runtime; MUST revert `NotAuthorizedEngine` or equivalent.
19. **Enable V2 backend/runtime routing:**
    - Backend env: `PERPS_ACTIVE_ENGINE_VERSION=v2`
    - Restart with `validate_startup` re-check
    - Backend preflight: `E.migrationState() == SEALED`, `E.clearingAccount() == C`, `P.isExecutor(runtime) == true`, chain-id 84532
20. **Only NOW create the first V2 real intent.** Match the current live A/B state (§18 of migration doc): closes at `249_273_743_964`, realised transfer of `244_274` raw mUSDC (not `488_548`).

**Ordering invariants (MUST hold):**
- Step 2 → 10 → 15 → 16 → 18 strictly monotonic (deploy before snapshot before seal before V1 revocation)
- Dual-authority window between steps 17 and 19 requires V1 to remain FROZEN (matched by the step 7 freeze)
- Any failure between steps 12-16 rolls back cleanly: delete the V2 contracts, keep V1 running

**Rollback:** at every step through 16, rollback = "delete V2, unfreeze V1". After 16 (seal), rollback requires deploying a NEW V2 with corrected state (the sealed V2 is irreversible).

---

## §22 (informational) — current A/B future seed payload

For the current live Base Sepolia state, the eventual seed calls will be:

```
E.adminSeedMarketFunding(
    1,
    0,        // cumulativeFundingRate1e18
    0         // lastFundingTimestamp
)

E.adminSeedPosition(
    0xff287410852B9328437eaC353720e5476bC5F837,  // trader A
    1,                                            // marketId
    +1_000_000,                                   // size1e8
    +2_468_310_000,                               // openNotional1e8
    0                                             // lastCumulativeFundingRate1e18
)

E.adminSeedPosition(
    0x66858286fEEA78a05eA093673EA1535E0A52002d,  // trader B
    1,
    -1_000_000,
    -2_468_310_000,
    0
)

E.sealMigration(0x<canonical_snapshot_hash>)
```

Vault balances at snapshot (informational; re-read at actual cutover):
- Trader A: 9_992_595 raw mUSDC
- Trader B: 9_998_765 raw mUSDC
- Fee sink: 8_684 raw mUSDC
- Clearing V2: 0 (undeployed)

PME V1 nonces at snapshot: A=1, B=1. V2 PME nonces start fresh at 0 (safe due to EIP-712 domain separator distinctness — see §5 tests).

---

## Not in this milestone (deferred to follow-ups)

- §6 — V2 `migrationState` / `clearingAccount` on-chain reads
- §7 — Clearing-liquidity preflight (address + balance floor)
- §8-11 — V2 prepare/cosign/simulation/executor preflight paths
- §12 — Receipt/emitter binding version-aware
- §13 — Durable reconciliation version-binding
- §14 — Indexer multi-emitter
- §15 — Anvil V2 deployment fixture (extend `script/DeployPerpsE2E.s.sol`)
- §16 — Full E2E close on Anvil (reproduce 244_274 mUSDC transfer)
- §17 — Migration-open negative test
- §18 — V1 legacy safety test (`ACTIVE_VERSION=v2` refuses new V1 intents)

Each of the above requires new tests + new RPC callsites + Anvil orchestration. Attempting them in one milestone alongside the foundation would produce partial coverage. They are sequenced as the follow-up milestones listed in the table at the top of this document.
