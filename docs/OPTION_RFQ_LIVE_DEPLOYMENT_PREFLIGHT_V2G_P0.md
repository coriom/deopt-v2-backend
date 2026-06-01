# V2G-P0 — OPTION RFQ Live Deployment Preflight

## Status

- Milestone: **V2G-P0** — sets up the offline deployment + signing
  surface for the V2G-O OPTION RFQ wiring, **without** broadcasting
  any transaction or restarting any live service.
- Date: 2026-06-01.
- Outcome:
  - **Live state audit pinned.** Base Sepolia deployment manifest
    (`deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`)
    confirms `OptionMatchingEngine = null` (never deployed). The live
    V2-enabled MarginEngine `0x287Cef479be5889eEfCa847F9e73C860898f48Cc`
    predates V2G-O and therefore does not expose the new
    `applyRfqTrade` selector. The live legacy MarginEngine
    `0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8` is also pre-V2G-O.
  - **Greenfield finding.** No OPTION RFQ — and in fact no live
    OPTION matching — exists on Base Sepolia today. The
    `option_series` in the manifest are
    `registry_active: false, activation_state: 0`. The first live
    OPTION ORDERBOOK + RFQ trade will therefore both land directly
    on V2G-O bytecode. There is **no state migration** for OPTION.
  - **Recommended strategy: A.** Redeploy MarginEngine V2G-O +
    first-deploy OptionMatchingEngine V2G-O in a single coordinated
    operator window, wire via the existing
    `RewireMarginEngineV2.s.sol` flow, do not flip
    `useFeesManagerV2` until smoke is green.
  - **Backend RFQ signing surface implemented** in
    `src/options/execution.rs`:
    - `OPTION_RFQ_TRADE_TYPE` const + `option_rfq_trade_typehash()`
      pinned to the on-chain `RFQ_TRADE_TYPEHASH`
      `0x6c660d979559d8526032a642d665ecefe15ca18cf062c24b6cd36058f98a123b`;
    - `OptionRfqTrade` `alloy_sol_types` struct + `executeRfqTrade`
      function definition;
    - `option_rfq_trade_digest`, `option_rfq_trade_digest_bytes`,
      `option_rfq_trade_hash` helpers — reuse the existing
      `OptionTradePayload` since the field layout is identical to
      the ORDERBOOK trade;
    - `encode_option_execute_rfq_trade_calldata` calldata builder;
    - `option_execute_rfq_trade_selector` and its expected sibling.
  - **8 new Rust unit tests** in `src/options/execution.rs::tests`
    pin: RFQ typehash matches contract, RFQ typehash ≠ ORDERBOOK
    typehash, RFQ digest ≠ ORDERBOOK digest for identical payload
    (cross-flow-replay defense), RFQ digest deterministic, RFQ
    selector matches signature, RFQ selector ≠ ORDERBOOK selector,
    calldata first 4 bytes are RFQ selector, calldata decodes the
    expected fields.
  - **Script gaps identified, not implemented.** The new forge
    scripts to be created next are
    `RewireOptionMatchingEngineForRfqV2.s.sol` (if reusing the V2D-L
    rewire pattern, this is actually subsumed by the existing
    `RewireMarginEngineV2.s.sol` since OptionMatchingEngine
    construction itself wires the MarginEngine) and
    `SmokeOptionRfqV2Fees.s.sol` (read-only RFQ-fee preflight checker).
  - **Backend signing CLI not extended** (no `--rfq` flag on the
    existing tooling) — V2G-P0 ships only the library surface; the
    operator-facing CLI extension is deferred to **V2G-P1**.
  - **Soak preserved.** Backend PID 56199 + 4-container compose
    stack remained healthy across all validations.
- Hard gates respected: no broadcast, no transaction submission, no
  redeploy, no chain mutation, no backend restart, no compose touch,
  no Prometheus reset, no `.env` edit, no private-key handling, no
  governance/timelock action, no soak interruption.

## Phase 1 — Live state audit

### Read sources (read-only)

- `deopt-v2-sol/deployments/base-sepolia.manifest.draft.json` —
  canonical Base Sepolia deployment manifest.
- `deopt-v2-sol/.env.base-sepolia` (key names only, values redacted).
- `deopt-v2-backend/.env`, `.env.example` (no contract addresses
  configured for live Base Sepolia OPTION matching).

### Findings

| Surface | Live state | Source |
|---|---|---|
| `MarginEngine` (V2 fees-enabled) | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` | manifest `feesManagerV2WiredToMarginEngine` |
| `MarginEngine` (legacy non-V2) | `0x6c5665de05e7314cb63cd77f82dfa86508a5b5f8` | manifest `contracts.MarginEngine` |
| `OptionMatchingEngine` | **`null` — never deployed** | manifest `contracts.OptionMatchingEngine` |
| `FeesManagerV2` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` (enabled, V2 toggle on) | manifest `feesManagerV2`, `feesManagerV2Status` |
| `feeRecipient` (V2) | `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` (Timelock) | manifest |
| `rebateFundingAccount` (V2) | `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` (Timelock) | manifest |
| `rebateBudget_baseCollateral` | `0` (no rebate budget funded yet) | manifest `feesManagerV2PostEnableReads` |
| `merkleRoot` | `0x000…000` (no tier merkle root set) | manifest `feesManagerV2PostEnableReads` |
| option series state | `registry_active: false, activation_state: 0` for both ETH + BTC | manifest `option_series` |
| Backend env wiring | No `OPTION_MATCHING_ENGINE_ADDRESS` / `MARGIN_ENGINE` set in `.env` | grep |
| Live RFQ support | **false** | derived: OptionMatchingEngine missing + MarginEngine predates V2G-O |
| Redeploy required | **yes** for OPTION path | derived |
| State migration risk | **LOW** (greenfield for OPTION) | derived |

### ABI confirmation

V2G-O sources:
- `src/matching/IMarginEngineRfqTrade.sol::applyRfqTrade(IMarginEngineTrade.Trade)` selector: emitted by `forge inspect MarginEngine methods` post-V2G-O; absent on the legacy `0x6c5665…` and the V2-pre-O `0x287Cef…` deployments by construction (this code never compiled on those branches).
- `OptionMatchingEngine.executeRfqTrade(...)` selector: only exists on the V2G-O `out/OptionMatchingEngine.json` artifact.
- `RFQ_TRADE_TYPEHASH = 0x6c660d979559d8526032a642d665ecefe15ca18cf062c24b6cd36058f98a123b` — derived by hashing `OptionRfqTrade(bytes32 intentId,address buyer,address seller,uint256 optionId,address underlying,address settlementAsset,uint64 expiry,uint64 strike1e8,bool isCall,uint128 contractSize1e8,uint128 quantity,uint128 premiumPerContract,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)` via a throwaway forge harness (cleaned up).

ORDERBOOK path remains intact:
- `MarginEngine.applyTrade` (V2G-O bit-equivalent body) — confirmed by `MarginEngineTest::testV2GO_OrderbookApplyTradeBehavesIdenticallyToPreRefactor`.
- `OptionMatchingEngine.executeTrade` — V2G-O leaves `TRADE_TYPEHASH` and the ORDERBOOK signing path untouched.

## Phase 2 — Deployment strategy comparison

| Strategy | Touched contracts | Admin calls | State migration impact | Backend env changes | Monitoring impact | Rollback | Risk |
|---|---|---|---|---|---|---|---|
| **A. Redeploy MarginEngine + first-deploy OptionMatchingEngine.** | MarginEngine, OptionMatchingEngine | DeployMarginEngineV2 → DeployOptionMatchingEngine → RewireMarginEngineV2 → WireFeesManagerV2Option | OPTION side: **none** (no live positions). MarginEngine V2 fee tier state lives in `FeesManagerV2`, **not** in MarginEngine — so the redeploy does not lose tier data. | Add `OPTION_MATCHING_ENGINE_ADDRESS` to backend `.env`; update `MARGIN_ENGINE` to the new address. | Indexer will see a new `MarginEngine` address — must update Prometheus `match[]` filters and Grafana variables. | Existing legacy MarginEngine `0x6c5665…` and pre-V2G-O `0x287Cef…` remain on chain; can re-wire dependents back if needed. | **LOW**. Greenfield OPTION + V2 fee state in separate contract. |
| **B. Redeploy only OptionMatchingEngine; keep `0x287Cef…` MarginEngine.** | OptionMatchingEngine | DeployOptionMatchingEngine | Rejected: `0x287Cef…` does not implement `IMarginEngineRfqTrade.applyRfqTrade`. `OptionMatchingEngine.executeRfqTrade` would revert at the cast. | none | none | n/a — strategy is non-viable. | n/a |
| **C. Deploy a separate RFQ adapter calling `applyTrade` with FlowKind impossible.** | new RFQ adapter contract | DeployRfqAdapter | Rejected: cannot reach `consumeFees(FlowKind.RFQ)` without the MarginEngine-side flow-parameterised helper; an adapter would have to *route through* the MarginEngine and would land at the same hardcoded `FlowKind.ORDERBOOK`. | n/a | n/a | n/a | High — defeats V2G-O Design Option A. |
| **D. Defer live RFQ until a future deployment wave.** | none | none | n/a | n/a | n/a | n/a | Acceptable as fallback only — leaves a written-but-unexposed feature, increases drift risk between source and live ABI. |

**Selected: Strategy A.** No live OPTION positions to migrate. Single coordinated redeploy carries both the new MarginEngine and the first OptionMatchingEngine.

## Phase 3 — Script gap analysis

### Existing scripts (sufficient for Strategy A)

| Script | Purpose | Re-usable for V2G-P? |
|---|---|---|
| `DeployMarginEngineV2.s.sol` | Safe-by-default MarginEngine deploy with `DEPLOY_MARGIN_ENGINE_V2_CONFIRM` gate | **Yes** — V2G-O code is in-tree; rebuilding picks it up automatically. |
| `DeployOptionMatchingEngine.s.sol` | First-deploy OptionMatchingEngine pointing at given MarginEngine | **Yes** — V2G-O code is in-tree; rebuilding picks it up automatically. |
| `RewireMarginEngineV2.s.sol` | Re-points dependents (vault, risk, matching, insurance, governor) at the new MarginEngine; `REWIRE_MARGIN_ENGINE_V2_CONFIRM` gate | **Yes** — already handles the OptionMatchingEngine via `setMarginEngine`. |
| `WireFeesManagerV2Option.s.sol` | Register the new MarginEngine as a FeesManagerV2 consumer for OPTION | **Yes** |
| `ConfigureMarkets.s.sol` | Activate OPTION product series | **Yes** |
| `SmokeOptionV2Rebate.s.sol` | Read-only preflight checker for the OPTION rebate path | **Re-usable as-is for ORDERBOOK smoke**. Does NOT cover RFQ. |
| `SmokeOptionV2RebateExecute.s.sol` | Broadcast smoke ORDERBOOK rebate trade | **Re-usable for ORDERBOOK**. Does NOT cover RFQ. |

### Script gaps (queued, not implemented in V2G-P0)

| New script | Purpose | Hard rule | Owner |
|---|---|---|---|
| `SmokeOptionRfqV2Fees.s.sol` | Read-only preflight: confirms the new OptionMatchingEngine exposes `executeRfqTrade(...)`, the new MarginEngine exposes `applyRfqTrade(...)`, and the RFQ discount profile for the configured maker/taker tiers matches the V2G-N canonical table. **No broadcast.** | abort unless `SMOKE_OPTION_RFQ_V2_CONFIRM=true` | V2G-P1 |
| `SmokeOptionRfqV2FeesExecute.s.sol` | Operator-only broadcast of one canonical RFQ trade (Tier 2 taker 94 ppm or Tier 4 maker -50 ppm preserved). | abort unless `SMOKE_OPTION_RFQ_V2_EXECUTE_CONFIRM=true` + dry-run pass | V2G-P2 |
| `PreflightOptionRfqEntryPoints.s.sol` | Pure ABI/selector probe — reads the bytecode of a given MarginEngine/OptionMatchingEngine and confirms the `applyRfqTrade` and `executeRfqTrade` selectors are present (i.e. the live deployment is V2G-O or later). | no broadcast | V2G-P0/P1 — could be added inline next session |

V2G-P0 deliberately does NOT implement the broadcast variants. The
non-broadcast ABI probe is small and safe but is queued together
with the read-only smoke so the operator can review them as one
diff in V2G-P1.

## Phase 4 — Backend signing readiness

### Implemented in V2G-P0 (library surface only)

`deopt-v2-backend/src/options/execution.rs`:

- `OPTION_RFQ_TRADE_TYPE` const (= `OptionRfqTrade(...)` with V2G-O field order).
- `OPTION_EXECUTE_RFQ_TRADE_SIGNATURE` const.
- `OptionRfqTrade` `alloy_sol_types::sol!` struct + `executeRfqTrade` function definition.
- `option_rfq_trade_typehash() -> [u8; 32]`.
- `option_rfq_trade_digest(payload, domain) -> Result<String>`.
- `option_rfq_trade_digest_bytes(payload, domain) -> Result<[u8; 32]>`.
- `option_rfq_trade_hash(payload) -> Result<[u8; 32]>` (internal).
- `option_trade_hash_with_typehash(payload, typehash) -> Result<[u8; 32]>` (shared internal).
- `option_execute_rfq_trade_selector() -> [u8; 4]`.
- `expected_option_execute_rfq_trade_selector() -> [u8; 4]`.
- `encode_option_execute_rfq_trade_calldata(payload, signatures) -> Result<Vec<u8>>`.

The existing `OptionTradePayload` is reused verbatim — the
RFQ struct has identical field layout to `OptionTrade`. The only
caller-visible difference is the typehash inside the digest and the
function selector inside the calldata.

### Tests added

`src/options/execution.rs::tests`:

| Test | Asserts |
|---|---|
| `option_rfq_trade_typehash_matches_contract` | `option_rfq_trade_typehash()` == `0x6c660d979559d8526032a642d665ecefe15ca18cf062c24b6cd36058f98a123b` (drift guard between backend + contract). |
| `option_rfq_typehash_differs_from_orderbook_typehash` | RFQ typehash ≠ ORDERBOOK typehash. |
| `option_rfq_trade_digest_differs_from_orderbook_for_identical_payload` | Cross-flow replay defense: same `OptionTradePayload` produces a different EIP-712 digest under each typehash. |
| `option_rfq_trade_digest_is_deterministic` | Sanity — repeated calls on the same input agree. |
| `option_execute_rfq_trade_selector_matches_signature` | `executeRfqTrade(...)` selector matches keccak(signature). |
| `option_execute_rfq_trade_selector_differs_from_orderbook` | Function selectors do not collide. |
| `option_execute_rfq_trade_calldata_has_rfq_selector` | Calldata first 4 bytes are RFQ selector; negative invariant against ORDERBOOK. |
| `option_execute_rfq_trade_calldata_decodes_expected_trade_fields` | Round-trips through `executeRfqTradeCall::abi_decode`. |

### Deferred to V2G-P1

| Item | Notes |
|---|---|
| `--rfq` flag on the existing OPTION signing CLI | Wire `option_rfq_trade_digest` + `encode_option_execute_rfq_trade_calldata` from the existing operator tooling. Requires no further library work — pure plumbing. |
| Address-derivation sanity check pinning the RFQ signer per V2G-D2 EOA registry | Reuse the existing pattern but switch typehash. |
| Explicit confirm gate (`OPTION_RFQ_V2_BROADCAST_CONFIRM=true`) | Mirror the V2D-S broadcast gate; default to dry-run. |
| Backend restart with V2G-O code | Hard-gated by V2G-K day-1 24h soak (still reserved for `2026-06-01T17:38Z`) and the V2G-M endpoint pickup blocker. |

## Phase 7 — Monitoring soak preservation

| Check | State at V2G-P0 close |
|---|---|
| Backend PID 56199 alive | ✅ |
| `/health` | ✅ `{"ok":true,...}` |
| Prometheus `/-/healthy` | ✅ |
| Compose 4/4 containers up | ✅ (15h+ uptime carried across V2G-O + V2G-P0) |
| No `docker compose down` | ✅ |
| No Prometheus reset | ✅ |
| No backend restart | ✅ |
| No `.env` edit (real secrets) | ✅ |

## Phase 8 — Validations

Solidity (`~/DEOPT/deopt-v2-sol`):

| Command | Result |
|---|---|
| `forge fmt` | clean (no diff) |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ |
| `forge test --no-match-path 'test/fork/*'` | ✅ 222/222 (no Solidity changes since V2G-O close) |

Backend (`~/DEOPT/deopt-v2-backend`):

| Command | Result |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ +8 new V2G-P0 tests on top of V2G-N's +1 / V2G-O baseline |

Frontend: untouched in V2G-P0 scope.

## Files changed

### Backend
- `src/options/execution.rs` — added RFQ typehash/digest/calldata helpers + 8 unit tests.

### Docs
- **New:** `docs/OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md` (this file).
- **Updated:** `docs/OPTION_RFQ_FLOW_WIRING_V2G_O.md` — backend signing-surface implementation note.
- **Updated:** `docs/OPTION_RFQ_FEE_DISCOUNTS_V2G_N.md` — V2G-P0 link in the integration gap section.

### Solidity
- None — V2G-O source remains the single ABI delivery.

## Remaining blockers

1. **No live OPTION matching engine on Base Sepolia.** This is not a regression — V2G-P0 is the first deployment plan. Operator must broadcast `DeployMarginEngineV2` (with V2G-O bytecode) + `DeployOptionMatchingEngine` + `RewireMarginEngineV2` in a single window during the V2G-P deploy session.
2. **`SmokeOptionRfqV2Fees.s.sol` + `PreflightOptionRfqEntryPoints.s.sol` not yet written.** Queued for V2G-P1; the safe-by-default pattern from `SmokeOptionV2Rebate.s.sol` is the template.
3. **Operator signing CLI `--rfq` flag not yet wired.** Library surface in place; plumbing queued for V2G-P1.
4. **Canonical V2G-K day-1 24h gate** still reserved for `2026-06-01T17:38Z`. Today's date is `2026-06-01`. The 24h gate elapses at the next tick — operator should hold the deploy session until the soak gate clears.
5. **V2G-M endpoint pickup requires backend restart.** Carried over — PID 56199 is the V2G-G era binary. The new V2G-P0 library symbols are present in `target/` but will not bind until the running process is replaced.

## Next recommended milestone

**V2G-P1 — Write the safe-by-default smoke + ABI-probe scripts and extend the operator signing CLI.**

- `script/PreflightOptionRfqEntryPoints.s.sol` — read-only, no env confirm needed (pure ABI probe).
- `script/SmokeOptionRfqV2Fees.s.sol` — read-only fee-quote preflight; `SMOKE_OPTION_RFQ_V2_CONFIRM=true` gate.
- Extend `tools/sign_option_execution_intent` (or equivalent) with `--rfq` flag wiring `option_rfq_trade_digest` + `encode_option_execute_rfq_trade_calldata`.
- Add Sol-side test that asserts the new MarginEngine bytecode implements both `applyTrade(Trade)` (selector `0x...`) and `applyRfqTrade(Trade)` (selector `0x...`) via `vm.code` reads.

V2G-P proper (broadcast + first live RFQ trade) only after V2G-P1
scripts pass dry-run review and the V2G-K day-1 gate has cleared.
