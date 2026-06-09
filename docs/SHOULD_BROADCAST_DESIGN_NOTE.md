# SHOULD_BROADCAST — design note

**Status:** DRAFT (Phase B of `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`).
**Posture:** decision/design only; no source modification in this note.
**Spec:** `BACKEND_GAS_FEES_REBATES_POLICY_V1.md §8` + `MAINNET_CUSTODY_CLUSTER_4_RESOLUTION_REDACTED.md §2.3` (launch invariant).
**Closes:** gap-list C-4 / W-3; auditor Q-34 anchor.

---

## 1. Where the gate plugs in

- **New module:** `src/options/broadcast_policy.rs`.
- **Re-export:** `pub use broadcast_policy::*` from `src/options/mod.rs`.
- **Call site:** `src/options/service.rs::broadcast_option_execution_intent_with_provider`,
  immediately AFTER `ensure_option_execution_broadcast_enabled(state)` and
  `get_option_execution_intent`, and BEFORE the dedupe-check on
  `find_submitted_option_execution_transaction`.
  Rationale: the policy gate is the canonical pre-broadcast decision; existing
  per-field validators (`validate_broadcast_intent`, `perform_option_broadcast_gas_safety_check`)
  remain in place but are subsumed by the policy at the structured-reason level.

## 2. Public surface

```rust
pub enum ShouldBroadcastDecision {
    Approve(ApprovalReason),
    Reject(RejectReason),
}

pub enum ApprovalReason {
    Profitable,
    AtCost,
    Subsidisable(SubsidyReason),
}

pub enum SubsidyReason {
    PromotionalLaunch,
    LiquidityBootstrap,
}

pub enum RejectReason {
    Dupe,
    BuyerSigInvalid,
    SellerSigInvalid,
    Expired,
    NonceConsumed,
    ProductUnlisted,
    StaleRm,
    OmePaused,
    BeNotExec,
    BeLowBal,
    NoBuyerMargin,
    NoSellerMargin,
    SimRevert(String),
    GasCap,
    Wash,
    QuotaBreach,
    AttackPattern,
    NoEconContent,
    RebateBudget,
    RebateReserve,
    NegativeEffectivePpm,
    ChainIdMismatch,
    SelectorNotAllowed,
    TargetNotAllowed,
    SimulationNotOk,
    PolicyInternal(String),
}

pub struct BroadcastContext<'a> {
    pub chain_id: u64,
    pub now_ms: i64,
    pub mode: BroadcastMode,           // Sepolia | Mainnet
    pub options_config: &'a OptionsConfig,
    pub execution_config: &'a ExecutionConfig,
    pub be_address: &'a AccountId,
    pub be_balance_wei: u128,
    pub fund_floor_wei: u128,
    pub ome_paused: bool,
    pub ome_is_executor: bool,
    pub buyer_has_margin: bool,
    pub seller_has_margin: bool,
    pub product_listed: bool,
    pub rm_snapshot_age_ms: u64,
    pub rm_snapshot_max_age_ms: u64,
    pub dedupe_hit: bool,
    pub simulation: SimulationSummary,
    pub fee_split: FeeSplitSummary,    // gross_fee_revenue + total_rebate_outflow + effective_ppms
    pub rebate_budget_asset: u128,
    pub rebate_reserve_asset: u128,
    pub gas_units: u64,
    pub hard_gas_cap: u64,
    pub eth_price_1e8: u128,
    pub pnl_floor_native: i128,
    pub safety_margin_bps: u32,
    pub subsidy_budget: SubsidyBudgetView,
}

pub fn should_broadcast(
    intent: &OptionExecutionIntent,
    context: &BroadcastContext<'_>,
) -> ShouldBroadcastDecision;
```

- Decision is **pure**: no I/O, no clock read, no chain call. All inputs are
  injected through `BroadcastContext`. This makes the gate trivially testable
  with table-driven unit tests.
- Side effects (status update, structured log, persistence) live in the
  call site, not in the policy.

## 3. Cluster 4 launch invariant verifier

Separate fn, callable independently:

```rust
pub struct LaunchInvariantReport {
    pub all_profiles_non_negative: bool,
    pub profiles: Vec<EffectiveProfileEntry>,
    pub rebate_reserve: u128,
    pub rebate_reserve_zero: bool,
    pub overall_pass: bool,
}

pub struct EffectiveProfileEntry {
    pub tier: u8,
    pub product: ProductKind,
    pub flow: FeeFlow,
    pub effective_maker_ppm: i64,
    pub effective_taker_ppm: i64,
    pub non_negative: bool,
}

pub fn verify_launch_invariant(
    profiles: &[ActiveFeeProfile],
    rebate_reserve_asset: u128,
    mode: BroadcastMode,
) -> LaunchInvariantReport;
```

- `overall_pass = all_profiles_non_negative AND (mode == Sepolia OR rebate_reserve_zero)`.
- Inputs are also pure data; the caller (operator CLI, startup-invariant
  hook, `should_broadcast` step-5 backstop) supplies the snapshot.

## 4. Mode awareness

- `BroadcastMode::Mainnet` → fail-closed default; `negative_effective_ppm`
  AND `rebate_reserve != 0` are both hard rejects.
- `BroadcastMode::Sepolia` → fee-only intents proceed even if a *future*
  rebate profile is present (Sepolia rehearsal harness is fee-only by
  construction); rebate-positive candidates still reject if budget/reserve
  is insufficient.
- Mode is derived from `context.chain_id`:
  - `8453` → Mainnet
  - `84532` → Sepolia
  - anything else → Mainnet (conservative; reject by default).

## 5. Integration with existing env gate

The policy does NOT replace `OPTION_EXECUTION_BROADCAST_ENABLED`.
Order at the broadcast call site:

1. `ensure_option_execution_broadcast_enabled(state)` — existing env gate.
2. `get_option_execution_intent(state, intent_id)` — existing fetch.
3. `should_broadcast(&intent, &ctx)` — NEW.
4. On `Approve` → existing dedupe + sign + broadcast path (unchanged).
5. On `Reject` → write `broadcast_failed` with reason; structured log; return
   `BackendError::BroadcastRejected("policy:<code>:<details>")`.

## 6. Startup invariant

On startup, if `chain_id == 8453` AND `execution_broadcast_enabled == true`,
the operator-side launch-invariant verifier MUST be exercised against an
operator-provided snapshot file (path supplied via env or CLI flag) before
the process accepts broadcast traffic. If the report `overall_pass == false`,
the backend exits non-zero with a structured error (no secrets printed).
This is the chain-side backstop teeth for Cluster 4.

For this build task, the startup invariant is gated behind a feature
flag `mainnet_launch_invariant_required` (default OFF) so that Sepolia
rehearsal and local dev are unaffected. Operator turns it ON at mainnet
cutover.

## 7. Out of scope (deferred PRs)

- Liquidation carve-out (§8 step 8) — `LiquidationCarveOut` left as
  `LiquidationCarveOut::OutOfScope` reject; followup tracked under gap-list
  C-13/C-14.
- Advanced anti-griefing heuristics (attack pattern, quota breach) — start
  with simple address-equality wash check; quota/pattern detectors can land
  separately.
- Persistent dedupe cache — Phase 1 uses in-memory hash set scoped to the
  call site (caller wires it); persistent dedupe lives in a follow-up.

## 8. Test surface

Unit tests are table-driven inside `broadcast_policy::tests`; integration
tests drive the wired broadcast call site with a stub provider. See
`BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md` (closeout) for the
full test matrix.
