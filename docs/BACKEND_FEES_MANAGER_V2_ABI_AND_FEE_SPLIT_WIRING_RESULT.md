# BACKEND-FEES-MANAGER-V2-ABI-AND-FEE-SPLIT-WIRING — result

**Status:** SHIPPED 2026-06-09 (Phase G close-out).

> **Addendum (2026-06-10, follow-on `BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE`):**
> the `FeeSplitSummary.effective_maker_ppm` / `.effective_taker_ppm`
> fields are now also surfaced via
> `BroadcastObservabilitySnapshot.last_effective_maker_ppm` /
> `.last_effective_taker_ppm`, populated by a single call-site update
> in `src/options/service.rs` immediately after
> `record_econ_data_available`. The recorder is guarded by
> `if let Some(fee_split) = inputs.fee_split.as_ref()` so boundary
> mode never records fake `(0, 0)`. The JSON `/executor/health/v2`
> endpoint reports these values verbatim — by construction, the policy
> gate reads the same numbers. See
> `docs/BACKEND_LIVE_PROVIDER_EFFECTIVE_PPM_CACHE_RESULT.md`.

**Scope:** wire `IFeesManagerV2.quoteFees(...)` + `rebateBudget(asset)`
read-only ABI calls into the broadcast policy data provider, populate
`fee_split` + `fm_v2_rebate_budget_asset`, and flip
`econ_data_available = true` at the policy ctx builder when ALL three
economic reads (FM_V2 quote, FM_V2 budget, PFV reserve) have landed.
**No mainnet tx. No live broadcast. No `.env` edit.**

---

## 1. Files changed

### New (1)

- `deopt-v2-backend/docs/BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`
  — this close-out doc.

### Modified (3)

- `src/options/broadcast_policy_data.rs` — **FeesManagerV2 ABI codec**
  (selector, encode call, decode 12-field `FeeQuote` static tuple,
  signed-i32 ABI decode with sign-extension validation, u128 overflow
  guard, isRebate / appliedPpm consistency cross-check). New
  `FeeQuoteRaw` type. New `aggregate_fee_split(maker, taker, asset)`
  helper. New `BroadcastPolicyInputs::fm_v2_rebate_budget_asset` field.
  `LiveBroadcastPolicyDataProvider` extended with a `quote_fees_call`
  RPC helper + the maker/taker pair invocation + `FM_V2.rebateBudget`
  read.
- `src/options/service.rs` — `run_should_broadcast_policy` now flips
  `econ_data_available = true` only when **all three** reads landed
  (`fee_split.is_some() AND fm_v2_rebate_budget_asset.is_some() AND
  pfv_rebate_reserve_asset.is_some()`); wires
  `inputs.fm_v2_rebate_budget_asset` into the rebate-budget asset cap.
  3 new integration tests on the broadcast call site.
- `src/options/mod.rs` — re-export the new public surface
  (`aggregate_fee_split`, `decode_fee_quote`, `encode_quote_fees_call`,
  `quote_fees_selector_bytes`, `FeeQuoteRaw`, `FM_V2_QUOTE_FEES_SELECTOR`).

## 2. ABI calls wired

| Contract function                                                   | Selector (keccak vector verified by unit test) | Returns                                |
| ------------------------------------------------------------------- | ---------------------------------------------- | -------------------------------------- |
| `quoteFees(address,uint8,uint8,bool,address,uint256)` (view, read-only) | `quote_fees_selector_bytes()`                  | `FeeQuote` static tuple (12 fields, 384 B) |
| `rebateBudget(address)` (view, read-only)                            | `keccak256("rebateBudget(address)")[0..4]`     | `uint256`                              |

Both calls are issued via the existing `EthCallProvider::eth_call` with
`from = 0x0`, `value = 0`, no gas limit. No state mutation, no
transaction sending.

Per intent the provider issues **2 × `quoteFees` + 1 × `rebateBudget` =
3 read-only RPC calls** when the FM_V2 address is configured.

## 3. Fee split mapping

For each broadcast attempt:

1. **`basis_amount`** = `intent.premium_per_contract_native × intent.quantity_contracts` (saturating multiplication; option fee basis is `PREMIUM` per `IFeesManagerV2.productFeeBasis(OPTION)`).
2. **maker / taker selection** from `intent.buyer_is_maker`:
   - `buyer_is_maker = true` → maker = buyer, taker = seller.
   - `buyer_is_maker = false` → maker = seller, taker = buyer.
3. **flow code**:
   - `OptionExecutionSourceType::OptionOrderbookFill` → `flow = 0` (`ORDERBOOK`).
   - `OptionExecutionSourceType::OptionRfqFill` → `flow = 1` (`RFQ`).
   - Product code is always `0` (`OPTION`).
4. **Calls**:
   - `quote_maker = quoteFees(maker_addr, OPTION, flow, isMaker=true, asset, basis_amount)`.
   - `quote_taker = quoteFees(taker_addr, OPTION, flow, isMaker=false, asset, basis_amount)`.
5. **Decode**: each call returns a 384-byte `FeeQuote` static tuple →
   `decode_fee_quote(raw)` validates:
   - sign-extension correctness on int32 (`appliedPpm`).
   - u128 overflow guard on `basisAmount` / `feeAmount`.
   - `isRebate == (appliedPpm < 0)` cross-check (with `appliedPpm == 0`
     treated as fee-only, matching the contract).
   - bool / u8 / address padding.
6. **Aggregate via `aggregate_fee_split(&maker_quote, &taker_quote, asset)`**:
   - `gross_fee_revenue = (maker_quote.fee_amount if !is_rebate) + (taker_quote.fee_amount if !is_rebate)`.
   - `total_rebate_outflow = (maker_quote.fee_amount if is_rebate) + (taker_quote.fee_amount if is_rebate)`.
   - `net_protocol_revenue = (gross as i128).saturating_sub(rebate as i128)`.
   - `effective_maker_ppm = maker_quote.applied_ppm as i64`.
   - `effective_taker_ppm = taker_quote.applied_ppm as i64`.
   - `tier = max(maker_quote.tier, taker_quote.tier)`.
   - `asset = intent.settlement_asset`.

This mirrors the `should_broadcast` consumer view of `FeeSplitSummary`
exactly — no internal-only fields.

## 4. RFQ discount behavior

The FeesManagerV2 contract handles the RFQ-side discount internally —
`quoteFees(..., flow=RFQ, ...)` computes the effective ppm as
`raw_ppm × (1_000_000 - discount_ppm) / 1_000_000` per
`getRfqDiscountProfile(tier, product)` and returns the resulting
`appliedPpm`. The backend does NOT have to recompute the discount; it
simply consumes `appliedPpm` from the quote return. This matches the
launch-invariant verifier's `apply_discount` formula in
`broadcast_policy::verify_launch_invariant`.

For testing: the unit tests for `aggregate_fee_split` cover both flow
shapes via the maker/taker `FeeQuoteRaw` permutations; RFQ-specific
discount clamping is exercised by the existing `verify_launch_invariant`
tests in `broadcast_policy::tests::rfq_discount_clamped_at_1m_ppm`.

## 5. Launch invariant behavior

Two paths now enforce the Cluster 4 launch invariant at the live-read
boundary:

### 5a. Live `should_broadcast §8 step 5` rebate-solvency hard gate

When `econ_data_available = true` AND `total_rebate_outflow > 0`:

- `rebate_budget_asset < total_rebate_outflow` → reject `policy:rebate-budget`.
- `rebate_reserve_asset < total_rebate_outflow` → reject `policy:rebate-reserve`.

Where `rebate_budget_asset = inputs.fm_v2_rebate_budget_asset` (live
read) and `rebate_reserve_asset = inputs.pfv_rebate_reserve_asset` (live
read).

**Cluster 4 launch state — `PFV.rebateReserve = 0`** — guarantees any
rebate-positive intent on mainnet rejects via `policy:rebate-reserve`
even if FM_V2's `rebateBudget` is non-zero.

### 5b. Live `should_broadcast §8 step 4` negative-effective-ppm hard gate

When `mode == Mainnet` AND `econ_data_available = true` AND either
`effective_maker_ppm < 0` OR `effective_taker_ppm < 0` → reject
`policy:negative-effective-ppm`.

This complements the existing `verify_launch_invariant` operator-side
sweep with a per-broadcast-attempt enforcement.

### 5c. Live `should_broadcast §8 step 4` no-econ-content

When `gross_fee_revenue == 0 AND total_rebate_outflow == 0` → reject
`policy:no-econ-content`. Live FM_V2 returning a zero-fee quote (e.g.
mis-tiered trader with effective ppm = 0) now correctly fires this.

### 5d. `econ_data_available` precondition

All three checks above gate on `econ_data_available = true`. Missing
any of `fee_split` / `fm_v2_rebate_budget_asset` /
`pfv_rebate_reserve_asset` keeps `econ_data_available = false` →
boundary mode (existing Sepolia rehearsal preserved; mainnet still
fail-closed via the chain-state gates from the prior milestone).

## 6. Broadcast integration

Order at the wired call site (unchanged from the prior milestone, but
with live economic data now flowing through):

1. `ensure_option_execution_broadcast_enabled`.
2. `get_option_execution_intent`.
3. `data_provider.gather_inputs(state, &intent).await` — now issues
   FM_V2 `quoteFees(maker)` + `quoteFees(taker)` + `rebateBudget(asset)`
   in addition to the chain-state reads. **Mainnet error → BroadcastFailed
   with structured `policy:policy-internal:<error>` reason; no signer
   call.**
4. Authoritative dedupe re-check (intent status + tx-row).
5. `run_should_broadcast_policy(state, &intent, &inputs)` — now applies
   `econ_data_available` based on the 3-way conjunction, AND maps the
   live FM_V2 budget into the policy.
6. Reject → `BroadcastFailed` + `policy:<code>:...` + warn log. **No
   signer call.**
7. Approve → existing signer + send path (unchanged).

## 7. Tests added (17 new)

### Unit (14 in `options::broadcast_policy_data::tests`)

ABI codec:
- `quote_fees_selector_matches_keccak_vector` — independent keccak of
  the canonical signature equals the helper output.
- `encode_quote_fees_call_has_exact_size_and_selector_prefix` — 4 +
  6×32 byte layout + selector prefix + last-word big-endian check.
- `decode_fee_quote_positive_ppm_round_trip`.
- `decode_fee_quote_zero_ppm_treated_as_non_rebate`.
- `decode_fee_quote_negative_ppm_is_rebate`.
- `decode_fee_quote_short_payload_rejected`.
- `decode_fee_quote_inconsistent_negative_sign_rejected` — `appliedPpm<0`
  but `isRebate=false` malformed contract output.
- `decode_fee_quote_inconsistent_positive_sign_rejected` —
  `appliedPpm>0` but `isRebate=true`.
- `decode_fee_quote_overflow_high_basis_rejected` — `basisAmount` over
  `u128::MAX` cap rejected as encoding error.
- `decode_fee_quote_malformed_sign_extension_rejected` — partial
  sign-extension yields encoding error.

Fee-split aggregation:
- `aggregate_fee_split_fee_only_path_sums_both_sides` — both sides
  positive ppm; gross = sum, rebate = 0, net = gross.
- `aggregate_fee_split_maker_rebate_taker_fee` — maker negative-ppm
  rebate + taker positive-ppm fee; net = fee - rebate.
- `aggregate_fee_split_tier_is_max_of_two_sides` — tier = max(maker, taker).
- `aggregate_fee_split_both_rebate_negative_net_revenue` — both sides
  rebate; gross = 0, rebate = sum, net < 0.

### Integration (3 in `options::service::tests`)

- `fee_split_populated_fee_only_intent_approves` — `fee_split` +
  `fm_v2_rebate_budget_asset` + `pfv_rebate_reserve_asset` all `Some(_)`
  → `econ_data_available = true` → fee-only intent approves; signer
  called once; chain send fires.
- `fee_split_rebate_positive_with_zero_reserve_rejects` — rebate-positive
  intent + `pfv_rebate_reserve_asset = 0` → reject
  `policy:rebate-reserve`; signer NOT called.
- `fee_split_rebate_positive_with_insufficient_budget_rejects` —
  rebate-positive intent + `fm_v2_rebate_budget_asset < total_rebate_outflow`
  → reject `policy:rebate-budget`; signer NOT called.

## 8. Tests run

```
cargo fmt --all -- --check                                          : ok
cargo clippy --all-targets --all-features -- -D warnings            : ok
cargo test --all-targets --all-features --no-fail-fast              :
  lib                                                                : 612 / 612 ✓ (was 595)
  integration suites (api / nonce_sync / fees / rfq / options / signatures / mm-protocol) : 256 ✓
  grand total                                                        : 868 / 868 ✓ (was 851)
forge fmt / forge build / forge test                                 : not re-run; no sol source touched
```

Previously-green tests preserved end-to-end:
- 8 chain-state-reads tests (prior milestone).
- 5 data-provider integration tests (prior milestone).
- 4 signer integration tests.
- 31 broadcast_policy unit tests.
- 16 remote_signer unit tests.
- 7 config-startup-guard tests.
- 47+ option-service tests.

## 9. Remaining limitations

- **`getProfile(tier, product)` direct read NOT wired** — the existing
  policy uses `quoteFees` (which internally consults the profile + tier
  + discount); a direct `getProfile` call isn't needed for the
  broadcast decision. The operator-side `verify_launch_invariant` sweep
  (already shipped) walks profiles independently.
- **`currentTier(account)` direct read NOT wired** — the `tier` field
  in the returned `FeeQuote` is the contract-side authoritative tier;
  the backend uses that value via the maker/taker max.
- **`hashTierLeaf` not wired** — the merkle-tier-claim path isn't part
  of the broadcast decision; it's an operator / dashboard concern.
- **Pending intents not pre-quoted** — `quoteFees` is only invoked at
  broadcast attempt time, not for indexer / lifecycle endpoints. A
  follow-on track may add cached quotes for orderbook display once the
  monitoring spec wiring lands.
- **Subsidy-budget view, risk-manager snapshot freshness, gas
  oracle, and pnl floor remain placeholder** (per prior milestone §10);
  these gates are not the launch-critical path.

## 10. Forbidden / Hard stops respected

- No mainnet broadcast attempted.
- No `.env` edited.
- No KMS / vendor / Treasury Safe creation.
- No `EXECUTOR_PRIVATE_KEY` value / RPC URL / DATABASE_URL / admin token
  printed in code, tests, docs, or commit messages.
- No real KMS sandbox key used in tests (mock provider only).
- No sol/ source touched.
- No DB schema migration (re-uses existing tx + status columns).
- No rebate reserve allocation. No PFV withdrawal. No fund movement.
- No fallback that allows mainnet local-key signing — signer still
  remains uncontacted under any policy reject.
- ABI decode is **strictly fail-closed**: malformed return data → `None`
  fee_split → `econ_data_available = false` → mainnet rejects with
  structured codes via the chain-state gates.
- Rebate-capable profiles cannot cause broadcast with reserve = 0 on
  mainnet — covered by integration test `fee_split_rebate_positive_with_zero_reserve_rejects`.

## 11. Cross-references

- Predecessor milestones: `WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md`,
  `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md`,
  `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md`.
- Spec source: `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §3 + §4 + §8`.
- Cluster anchor: `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3`.
- Custody principle: `~/DEOPT/MAINNET_CUSTODY_POLICY.md §10.1` (chain-side
  backstop for accounting).
- Contract source of truth: `deopt-v2-sol/src/fees/IFeesManagerV2.sol`
  + `FeesManagerV2.sol`.
- Auditor anchors strengthened: **Q-34 (Cluster 4 launch invariant)** —
  per-broadcast live read of the FM_V2 quote + PFV.rebateReserve now
  enforces the §8 step 5 hard gate at the boundary; **Q-26..Q-29** (chain-
  side allowlist correctness) — backend now consumes the same
  `appliedPpm` the contract emits.

## 12. Next milestone recommendation

**Primary backend-side:** `BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING`
— plumb `kms_request_id` + signer `signer:<code>` reject-rate metrics +
new `policy-data:rpc:…` metrics + the new `policy:rebate-budget` /
`policy:rebate-reserve` / `policy:negative-effective-ppm` /
`policy:no-econ-content` reject codes into the existing alerts spec
(`BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 + §7.1`).

**Parallel operator-side (unchanged):** `MAINNET-KMS-VENDOR-SELECTION`
(Q-CD-5; once it lands the
`MAINNET-KMS-VENDOR-ADAPTER-IMPLEMENTATION` follow-on closes the wire
transport for the signer microservice).
`MAINNET-AUDIT-EXT-KICKOFF`. `MAINNET-TREASURY-SAFE-CREATION-PACKET`.

The C-4 economic gate is now **closed end-to-end** at the broadcast
boundary: live FM_V2 quote → policy ctx → `should_broadcast §8 steps
4/5/7` fire with live data. Mainnet rejects on rebate-positive intents
when reserve = 0 (Cluster 4 launch state).
