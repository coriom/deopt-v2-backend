# V2G-P1 — OPTION RFQ Operator Packet + Forge Preflight Scripts

## Status

- Milestone: **V2G-P1** — ships the operator-facing artifacts
  (read-only forge probes + a safe-by-default backend packet
  generator) that V2G-P0 documented as "queued."
- Date: 2026-06-01.
- Outcome:
  - **3 new forge scripts** (all safe-by-default, none broadcasts):
    - `script/PreflightOptionRfqEntryPoints.s.sol` — read-only
      bytecode-scan probe (`PUSH4 <selector>`); reports
      `applyRfqTrade` + `executeRfqTrade` presence vs ORDERBOOK
      reference selectors. Supports env-driven `run()` and
      address-driven `probe(margin, optionMatching)` for tests.
    - `script/SmokeOptionRfqV2Fees.s.sol` — read-only RFQ-fee
      preflight; calls `FeesManagerV2.quoteFees` with
      `FlowKind.RFQ`, asserts flow kind, asserts expected
      maker/taker fees if env-supplied, asserts rebate-budget
      sufficiency if the maker quote is a rebate.
    - `script/SmokeOptionRfqV2FeesExecute.s.sol` — execute
      **scaffold**; runs the same preflight and reconstructs the
      EIP-712 digest the operator must sign offline. Refuses
      without `SMOKE_OPTION_RFQ_V2_FEES_EXECUTE_CONFIRM=true`.
      Even when confirmed, does NOT broadcast — the broadcast
      block is intentionally absent in V2G-P1; V2G-P2 wires it.
  - **4 new Solidity tests** in
    `test/unit/scripts/PreflightOptionRfqEntryPoints.t.sol`
    exercise the probe against EOA, zero-address, and stubbed
    legacy/V2G-O contracts.
  - **1 new backend module** `src/options/rfq_operator_packet.rs`
    exposing:
    - `OptionRfqOperatorPacketInputs<'_>` — payload + domain
      + optional signatures.
    - `OptionRfqOperatorPacket` — digest_hex, digest_bytes,
      function_selector_hex, calldata_hex (only when signatures
      attached), payload_summary, broadcast_ready, broadcast
      confirm-env name.
    - `build_option_rfq_operator_packet(...)` — computes the
      EIP-712 RFQ digest and, when signatures are present, the
      ABI-encoded `executeRfqTrade(...)` calldata.
    - `require_option_rfq_broadcast_confirm(env)` — operator
      gate; accepts *only* the literal string `"true"`.
    - `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV` constant.
    - `OptionRfqOperatorBroadcastError` enum.
  - **10 new Rust unit tests** covering: digest-only packet,
    signed packet, RFQ vs ORDERBOOK selector, RFQ vs ORDERBOOK
    digest, broadcast-confirm refusal under absent / `false` /
    `1` / `TRUE` (only literal `"true"` is accepted), no
    signature leakage in `payload_summary`, canonical typehash
    field count.
  - **ORDERBOOK scripts unchanged** — neither
    `SmokeOptionV2Rebate.s.sol` nor `SmokeOptionV2RebateExecute.s.sol`
    were modified.
  - **Soak preserved.** PID 56199 + 4-container compose stack
    remained healthy throughout V2G-P1.
- Hard gates respected: no broadcast, no transaction submission, no
  redeploy, no chain mutation, no backend restart, no compose touch,
  no Prometheus reset, no `.env` edit, no private-key handling, no
  governance/timelock action, no soak interruption.

## What V2G-P1 deliberately does NOT do

- **No broadcast.** No script and no Rust function in this milestone
  submits a transaction. `SmokeOptionRfqV2FeesExecute.s.sol` is a
  scaffold that produces an offline-signable digest; the actual
  broadcast (signing → executor → V2 fee events landing in
  `/admin/fees/onchain`) is V2G-P2 work.
- **No live deployment.** The V2G-O bytecode still sits in `out/`.
  V2G-P (the broadcast milestone) is still pending.
- **No backend restart.** The new module is in the codebase but the
  live PID 56199 (V2G-G era binary) does not see it. New tests run
  against the freshly-built `target/` artifacts; the running soak
  binary is untouched.
- **No private-key handling.** The operator packet builder takes
  pre-signed signature bytes as input — it never sees a key. The
  EOA signing happens out-of-band (hardware wallet, V2G-D2 signing
  CLI, etc.).

## Forge scripts

### `script/PreflightOptionRfqEntryPoints.s.sol`

| Knob | Value |
|---|---|
| Reads | `MARGIN_ENGINE`, `OPTION_MATCHING_ENGINE` |
| Writes | none |
| Probe technique | Solc-emitted `PUSH4 selector` bytecode scan |
| Selectors probed | `applyRfqTrade=0x1ccdd23f`, `applyTrade=0xb022e608`, `executeRfqTrade=0xb52ce6f5`, `executeTrade=0x031f77b3` |
| Public API | `run() returns (Report)` env-driven, `probe(address, address) returns (Report)` address-driven (for tests) |
| Failure modes | none — returns `Report` with `SelectorStatus` per probe; missing env → `NotConfigured`, EOA → `TargetHasNoCode`, code w/o selector → `NotExposed`, code w/ selector → `Exposed` |

### `script/SmokeOptionRfqV2Fees.s.sol`

| Knob | Value |
|---|---|
| Reads | `FEES_MANAGER_V2_ADDRESS`, `MARGIN_ENGINE`, `OPTION_MATCHING_ENGINE`, `MAKER_ACCOUNT`, `TAKER_ACCOUNT`, `SETTLEMENT_ASSET`, `RFQ_PREMIUM_NATIVE`, `EXPECTED_FLOW_KIND_RFQ`, `EXPECTED_MAKER_FEE_NATIVE` (optional), `EXPECTED_TAKER_FEE_NATIVE` (optional), `MIN_REBATE_BUDGET` (optional) |
| Writes | none (`view`-only) |
| Asserts | MarginEngine uses FeesManagerV2 ✓ MarginEngine is fee consumer ✓ Code present on both target contracts ✓ `quoteFees(... FlowKind.RFQ ...).flow == RFQ` for both legs ✓ Expected fee amounts match if env-supplied ✓ Rebate budget covers maker rebate if quote is a rebate |
| Failure mode | Custom error with operator-readable name (`RfqMakerQuoteFlowKindMismatch`, `RfqTakerFeeMismatch`, `RebateBudgetTooLowForMakerRebate`, etc.) |

### `script/SmokeOptionRfqV2FeesExecute.s.sol`

| Knob | Value |
|---|---|
| Reads | All of the smoke env plus `OPTION_ID`, `UNDERLYING`, `OPTION_EXPIRY`, `OPTION_STRIKE_1E8`, `OPTION_IS_CALL`, `OPTION_CONTRACT_SIZE_1E8`, `OPTION_QUANTITY`, `OPTION_PREMIUM_PER_CONTRACT`, `OPTION_BUYER_IS_MAKER`, `OPTION_DEADLINE_SECONDS`, `OPTION_BUYER_NONCE`, `OPTION_SELLER_NONCE`, `OPTION_INTENT_ID`, `SMOKE_OPTION_RFQ_V2_FEES_EXECUTE_CONFIRM` |
| Writes | none (intentionally `view`) |
| Refuses | unless confirm flag set; unless chain != Base mainnet (8453); unless V2 wiring complete; unless rebate budget ≥ `MIN_REBATE_BUDGET` |
| Output | EIP-712 RFQ digest (operator signs offline) + payload echo |
| Broadcast | **explicitly disabled** in V2G-P1 |

## Sol-side tests

`test/unit/scripts/PreflightOptionRfqEntryPoints.t.sol` — 4 tests:

| Test | Asserts |
|---|---|
| `test_probe_reportsNotConfiguredWhenAddressesAreZero` | `address(0)` ⇒ `NotConfigured` for all four selectors. |
| `test_probe_reportsTargetHasNoCodeForEoaAddress` | EOA (no code) ⇒ `TargetHasNoCode`. |
| `test_probe_reportsExposedForV2GOMatchingEngineStub` | V2G-O-shaped stub (implements both ORDERBOOK + RFQ selectors) ⇒ `Exposed` for all four. |
| `test_probe_reportsNotExposedForLegacyMatchingEngineStub` | Legacy-shaped stub (only ORDERBOOK selectors) ⇒ `Exposed` for ORDERBOOK + `NotExposed` for RFQ. Pins the operational claim that the probe distinguishes V2G-O from pre-V2G-O bytecode. |

## Backend operator packet module

### File

`src/options/rfq_operator_packet.rs` (new), re-exported from
`src/options/mod.rs`.

### Public surface

```rust
pub const OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV: &str =
    "OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM";

pub struct OptionRfqOperatorPacketInputs<'a> {
    pub payload: &'a OptionTradePayload,
    pub domain: &'a Eip712Domain,
    pub signatures: Option<&'a OptionTradeSignatureBundle>,
}

pub struct OptionRfqOperatorPacket {
    pub digest_hex: String,
    pub digest_bytes: [u8; 32],
    pub function_selector_hex: String,
    pub calldata_hex: Option<String>,
    pub payload_summary: String,
    pub broadcast_confirm_env: &'static str,
    pub broadcast_ready: bool,
}

pub fn build_option_rfq_operator_packet(
    inputs: OptionRfqOperatorPacketInputs<'_>,
) -> Result<OptionRfqOperatorPacket>;

pub fn require_option_rfq_broadcast_confirm(
    env: &HashMap<String, String>,
) -> std::result::Result<(), OptionRfqOperatorBroadcastError>;
```

### Backend tests

`src/options/rfq_operator_packet.rs::tests` — 10 tests:

| Test | Asserts |
|---|---|
| `builds_packet_without_signatures_for_offline_signing` | Digest produced; calldata absent; summary readable. |
| `builds_packet_with_signatures_attaches_calldata` | Calldata first 4 bytes match `function_selector_hex`. |
| `calldata_carries_rfq_selector_not_orderbook_selector` | Cross-flow replay defense at the calldata level. |
| `digest_differs_from_orderbook_digest_for_identical_payload` | Cross-flow replay defense at the digest level. |
| `require_broadcast_confirm_refuses_when_flag_absent` | Safe-by-default. |
| `require_broadcast_confirm_refuses_when_flag_is_not_true` | Pins acceptance to literal `"true"` (rejects `false`, `1`, `TRUE`). |
| `require_broadcast_confirm_accepts_only_literal_true` | Positive case. |
| `packet_summary_does_not_expose_private_key_or_signature` | Static check that summary contains neither key markers nor the canary `0xaa…`/`0xbb…` signature bytes. |
| `option_rfq_trade_type_constant_referenced` | Keeps the canonical type-string linked from the operator module. |
| `payload_field_count_matches_canonical_type_string` | Drift guard — `OPTION_RFQ_TRADE_TYPE` must remain 16 fields. |

### Soak-safe usage

The packet module never touches the DB, never opens an RPC client,
never spawns a tokio task. It is pure compute over caller-supplied
inputs. The operator tooling that consumes it (the V2G-P2 broadcast
flow) will be the first piece that may require a backend restart.

## ORDERBOOK preservation

| Surface | Touched in V2G-P1? |
|---|---|
| `OptionMatchingEngine.executeTrade` | no |
| `MarginEngineTrading.applyTrade` | no |
| `script/SmokeOptionV2Rebate.s.sol` | no |
| `script/SmokeOptionV2RebateExecute.s.sol` | no |
| Backend `encode_option_execute_trade_calldata` | no |
| Backend `option_trade_digest` | no |

## Validations

Solidity (`~/DEOPT/deopt-v2-sol`):

| Command | Result |
|---|---|
| `forge fmt` | clean |
| `forge fmt --check` | ✅ |
| `forge build` | ✅ (warnings: pre-existing lint suggestions only) |
| `forge test --no-match-path 'test/fork/*'` | ✅ +4 V2G-P1 tests on top of V2G-O's 222 |

Backend (`~/DEOPT/deopt-v2-backend`):

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ +10 V2G-P1 tests on top of V2G-P0 baseline |

Frontend: untouched in V2G-P1 scope.

## Monitoring soak preservation

| Check | State at V2G-P1 close |
|---|---|
| Backend PID 56199 alive | ✅ (no restart) |
| `/health` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ (15h+ uptime carried across V2G-O/P0/P1) |
| No `docker compose down` | ✅ |
| No Prometheus reset | ✅ |
| No backend restart | ✅ |
| No `.env` edit (real secrets) | ✅ |
| Day-1 24h gate `2026-06-01T17:38Z` | reserved — not yet ticked |

## Remaining blockers

1. **V2G-K day-1 24h soak gate** still reserved for `2026-06-01T17:38Z`. V2G-P (the broadcast milestone) should not run before that tick clears.
2. **`SmokeOptionRfqV2FeesExecute.s.sol` is a scaffold, not a broadcaster.** V2G-P2 must add the maker/taker signing-key-on-keystore branch + the explicit broadcast block.
3. **Operator CLI plumbing.** The packet module is library-level. The actual operator-facing executable that calls `build_option_rfq_operator_packet` + collects signatures + invokes the broadcast is V2G-P2.
4. **No live `OptionMatchingEngine` on Base Sepolia.** The V2G-O bytecode redeploy itself is the V2G-P broadcast milestone — must precede any RFQ smoke trade.
5. **V2G-M endpoint pickup requires backend restart.** Carried over — PID 56199 is the V2G-G era binary; the V2G-P0+P1 library symbols are present in `target/` but will not bind until the running process is replaced.

## Next recommended milestone

**V2G-P2 — Operator broadcast plumbing + first live RFQ smoke.**

1. Add a small operator binary or admin endpoint that consumes
   `OptionRfqOperatorPacketInputs`, performs the broadcast against
   `executeRfqTrade(...)`, and refuses without
   `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM=true`.
2. Add the maker/taker keystore-signing branch to
   `SmokeOptionRfqV2FeesExecute.s.sol`; pin it behind a second
   confirm flag (`SMOKE_OPTION_RFQ_V2_FEES_BROADCAST_CONFIRM=true`).
3. After V2G-P (the redeploy) lands and the V2G-K day-1 gate
   clears, run the end-to-end RFQ smoke: preflight → execute
   scaffold (digest) → operator signs → broadcast → verify
   `FeeChargedV2.flowKind=RFQ` lands in `/admin/fees/onchain`.
