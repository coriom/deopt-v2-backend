# V2G-R — ProtocolFeeVault Production Design Spec

## Status

- Milestone: **V2G-R** — design-spec only. No code shipped, no
  Solidity sources edited, no scripts added, no live state mutated.
- Date: 2026-06-01.
- Scope: a **single** production fee-treasury module — `ProtocolFeeVault` —
  that replaces the two-EOA pattern in production today (`feeRecipient` +
  `rebateFundingAccount` both pointing at the `RiskGovernor` /
  `ProtocolTimelock` address `0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588`).
- Outcome:
  - Architecture decided: **one module, internal buckets** rather than
    two separate `FeeCollector` / `RebateReserve` contracts.
  - Integration with `FeesManagerV2` defined: vault is the new
    `feeRecipient` AND the new `rebateFundingAccount`. Fee math stays
    in `FeesManagerV2`. `rebateBudget` accounting stays in
    `FeesManagerV2`.
  - `CollateralVault` interaction model defined: the vault holds its
    bookkeeping in `CollateralVault.balances[vault][asset]` exactly
    like every other internal account, so existing
    `transferBetweenAccounts` plumbing works without changes.
  - Admin surface defined: `allocateToRebateReserve`, `withdrawRevenue`,
    `pauseRebates` / `unpauseRebates`, `sweepUnsupported`,
    `setRevenueReceiver`.
  - Events, invariants, security considerations, migration plan,
    monitoring metric changes, and test plan all sketched below.
- Hard gates respected: no broadcast, no deploy, no chain mutation,
  no backend restart, no compose touch, no Prometheus reset, no
  `.env` edit, no implementation source written.

## Goals + non-goals

### Goals

1. **Single contract surface** for the operator/finance team to reason
   about: gross fees collected, rebates paid, rebate-reserve balance,
   net revenue, all per settlement asset.
2. **Drop-in compatibility** with the deployed `FeesManagerV2` — the
   integration switch is one-call (`setFeeRecipient(vault)` +
   `setRebateFundingAccount(vault)`).
3. **Cleaner monitoring**: the V2 fee Grafana dashboard becomes
   `vault.grossFees - vault.rebatesPaid = vault.netRevenue` per asset
   instead of inferring it from two EOA balances + a separate
   `rebateBudget` mapping.
4. **Tight rebate-cap independence**: the FeesManagerV2-side
   `rebateBudget` mapping remains the gating ceiling; the vault's
   `rebateReserve` is a *funding pool*, not a re-implementation of the
   cap. The vault can be empty and the system stays correct (just
   reverts on the next rebate consumption, same as today).
5. **Reversible**: ownership transfer + setter calls only; the vault
   never holds keys, never holds private state that can't be reread
   off chain, never embeds business logic that should live in
   `FeesManagerV2`.

### Non-goals

- Re-implementing fee math (`_effectiveRatePpm`, RFQ discount,
  tier resolution) — those stay in `FeesManagerV2`.
- Holding the rebate cap (`rebateBudget`) — stays in `FeesManagerV2`.
- Decoupling option vs perp accounting at the vault layer — both
  products share `feeBalance[asset]` / `rebateReserve[asset]`. Per-product
  reporting is a view-time aggregation on top of `FeeChargedV2` /
  `FeeRebatedV2` event streams.
- Implementing yield strategies for the rebate reserve (V2H+ work).
- Cross-chain treasury bridging (V2H+ work).

## One module vs two — why one wins at launch

| Concern | Single `ProtocolFeeVault` | Separate `FeeCollector` + `RebateReserve` |
|---|---|---|
| Operator deploy cost | 1 contract, 1 wiring step | 2 contracts, 2 wiring steps, 2 ownership transfers |
| Atomicity of fund moves between buckets | Internal write — trivially atomic | Cross-contract transfer + 2-sided event coordination |
| Audit surface | One ABI, one storage layout, one set of invariants | Two ABIs, two storage layouts; integration risk between the two |
| Monitoring | `vault.netRevenue(asset)` is a single view | Indexer must JOIN the two contracts' balances |
| Rebate-reserve top-up flow | `allocateToRebateReserve(asset, amount)` — pure internal book entry | `feeCollector.transferTo(rebateReserve, asset, amount)` — full `transferBetweenAccounts` round trip |
| Per-asset accounting | One mapping read | Two contracts to query and reconcile |
| Risk of bucket-mismatch bug | Constrained to one contract | Two contracts can drift |
| Future split-out path | Can split later by promoting one bucket to its own contract; storage layout is forward-compatible if buckets are typed cleanly | Already split — but the operator paid the split cost up front |
| Governance complexity | One owner / one timelock target | Two owners or one owner with two targets — more surface for misconfig |
| Reversibility of design | High — easy to add a sibling contract later | Hard to merge back into one |

At launch we expect:
- 1 settlement asset (mUSDC),
- a single timelock (`RiskGovernor`),
- a small operator team.

The split-out gains (clear separation of concerns, mutual
isolation) buy us nothing at this size. We pay the split cost up
front and ship two contracts that will move the same money 99% of
the time. **One module wins.** If V2H needs separation (e.g. a
dedicated rebate-reserve yield vault), the buckets in this design
are independent storage slots — we can detach `rebateReserve[asset]`
into a sibling contract with one migration script.

## Module shape

### Storage layout (sketch — design only)

```solidity
contract ProtocolFeeVault {
    /// @notice Trusted on-chain collateral ledger. Vault holds all
    ///         settlement-asset funds inside this vault's internal
    ///         account: `_collateralVault.balances[address(this)][asset]`.
    ICollateralVault public immutable collateralVault;

    /// @notice The FeesManagerV2 instance the vault is paired with.
    ///         Used for `rebateBudget` reads (informational, not gating).
    IFeesManagerV2 public immutable feesManagerV2;

    /// @notice Owner — expected to be the protocol timelock.
    address public owner;

    /// @notice EOA / contract that receives `withdrawRevenue` payouts.
    address public revenueReceiver;

    /// @notice Per-asset internal buckets. Sum invariants below.
    mapping(address => uint256) public feeBalance;        // unallocated positive-fee proceeds
    mapping(address => uint256) public rebateReserve;     // funds reserved for paying rebates
    mapping(address => uint256) public grossFeesCollected; // monotonic running total of inbound fees
    mapping(address => uint256) public rebatesPaid;        // monotonic running total of outbound rebates
    mapping(address => uint256) public netRevenue;         // grossFeesCollected - rebatesPaid (cached for cheap reads)

    /// @notice Rebate pause flag. When true the vault refuses to
    ///         honour `consumeFees` rebate withdrawals by reverting
    ///         on the `transferBetweenAccounts` callback. FeesManagerV2
    ///         then bubbles the revert up to the consuming engine.
    bool public rebatesPaused;
}
```

### Why the vault holds money in `CollateralVault` rather than as ERC20

`FeesManagerV2.consumeFees` currently moves value via
`CollateralVault.transferBetweenAccounts(asset, from, to, amount)`.
Both `from` and `to` are pure indices into `CollateralVault.balances`
— they do NOT need to be EOAs or even contracts. By holding the
vault's funds in `CollateralVault.balances[vault][asset]`, the
integration is a zero-byte change at the `FeesManagerV2` site: the
vault is *just another internal account* from the CollateralVault's
point of view.

This also means the vault inherits the CollateralVault's safety:
- `nonReentrant`,
- `whenInternalTransfersNotPaused`,
- `onlyMarginEngine` authorization at the transfer site,
- all the existing CollateralVault pause/seizure semantics.

The vault does NOT hold raw ERC20 tokens (no `IERC20.transferFrom`,
no allowance mgmt at the vault layer). To withdraw revenue, the
vault calls `CollateralVault.withdrawFor(revenueReceiver, asset,
amount)` (or equivalent — see CollateralVault interaction model
below). This means a single audited path for moving asset value
in/out of the protocol.

## Integration with FeesManagerV2

### Recipient wiring

| Setter | Old target | New target |
|---|---|---|
| `FeesManagerV2.setFeeRecipient(addr)` | `0xa67f…3b588` (Timelock EOA) | `address(protocolFeeVault)` |
| `FeesManagerV2.setRebateFundingAccount(addr)` | `0xa67f…3b588` (Timelock EOA) | `address(protocolFeeVault)` |

Both setters live on `FeesManagerV2`, are `onlyOwner`, emit
`FeeRecipientSet` / `RebateFundingAccountSet` — no changes to those
contracts. The vault's authority over its own balances follows
from being an internal `CollateralVault` account; it does not need
custom access to `FeesManagerV2`.

### Money flow — positive fees (consumeFees → vault)

1. Engine calls `FeesManagerV2.consumeFees(trader, OPTION, ORDERBOOK, isMaker, asset, premium)`.
2. FeesManagerV2 computes the positive fee, picks `recipient = feeRecipient = vault`.
3. `CollateralVault.transferBetweenAccounts(asset, trader, vault, feeAmount)` (via the engine's existing path).
4. `FeesManagerV2` emits `FeeChargedV2(consumer, trader, recipient=vault, asset, ...)`.
5. **Vault has no on-chain action here.** The funds land in
   `CollateralVault.balances[vault][asset]`. The vault's
   `feeBalance[asset]` and `grossFeesCollected[asset]` are updated
   lazily by an off-chain indexer OR eagerly via a vault-side
   `accrue(asset)` view that reconciles internal totals against
   the CollateralVault balance — see "Hot-path callback option"
   below.

### Money flow — rebates (consumeFees → trader, debit vault)

1. Engine calls `FeesManagerV2.consumeFees(...)` with a maker / rebate path.
2. `FeesManagerV2` reads `rebateBudget[asset]` and reverts if budget < amount (unchanged).
3. FeesManagerV2 reads `rebateFundingAccount = vault`.
4. `CollateralVault.transferBetweenAccounts(asset, vault, trader, rebateAmount)` (via the engine's existing path).
5. `FeesManagerV2` decrements `rebateBudget[asset]` and emits `FeeRebatedV2`.
6. **Vault has no on-chain action here.** `rebatesPaid[asset]` is
   updated lazily / via the same accrue path.

### Hot-path callback option (recommended after V2G-R review)

To keep `feeBalance`/`rebateReserve`/`grossFeesCollected`/`rebatesPaid`
accurate *on chain*, add a small callback hook the vault implements
and that `FeesManagerV2.consumeFees` calls after the transfer
completes. Two designs:

**Option α — vault-side accrue without hook:**
- Vault exposes `accrue(address asset) external` that reads
  `CollateralVault.balances[vault][asset]` (delta vs cached total),
  classifies the delta against `FeeChargedV2` / `FeeRebatedV2` event
  history (off-chain indexer pushes a hint), and updates
  `feeBalance` / `grossFeesCollected` / etc.
- Pro: no FeesManagerV2 code change.
- Con: requires the off-chain indexer to be live to keep the buckets
  consistent; risk of drift if indexer lags.

**Option β — FeesManagerV2-side notify hook:**
- Vault implements `function onFeeCharged(asset, amount) external onlyFeesManagerV2` and `function onRebatePaid(asset, amount) external onlyFeesManagerV2`.
- `FeesManagerV2.consumeFees` calls these after the transfer.
- Pro: bucket totals are always on-chain consistent without any
  off-chain dependency.
- Con: adds 2 cross-contract calls per `consumeFees`, and requires a
  small additive ABI change on `FeesManagerV2`.

**Recommendation: ship Option β as part of V2G-R deployment.**
The gas cost is bounded (one SLOAD + one SSTORE + one external call
per leg, ~5k gas), the operator clarity gain is significant, and
the FeesManagerV2 hook signature is so small that a single setter
(`setFeeRecipientCallback(address)`) can switch it on or off.

For the design-spec phase we **specify Option β as the launch
configuration** with Option α as a fallback if the FeesManagerV2
redeploy is deferred beyond V2G-P.

### `rebateBudget` ownership

`FeesManagerV2.rebateBudget[asset]` stays the canonical *cap* that
gates per-`consumeFees` rebate consumption. It is **independent**
from the vault's `rebateReserve[asset]` (which is a *funding pool*).
Both can exist; both can be zero; both must be funded for live
rebates to flow:

| State | Behaviour |
|---|---|
| `feesManagerV2.rebateBudget[asset]` ≥ rebate amount AND `vault.rebateReserve[asset]` ≥ rebate amount | rebate succeeds, both decrement |
| `feesManagerV2.rebateBudget[asset]` < rebate amount | FeesManagerV2 reverts `InsufficientRebateBudget` (current behaviour, unchanged) |
| `feesManagerV2.rebateBudget[asset]` ≥ but `vault.rebateReserve[asset]` < rebate amount | Vault refuses the funding transfer (revert at the CollateralVault hop) → bubbles back as FeesManagerV2 revert |
| `rebatesPaused == true` (vault flag) | Vault refuses any rebate funding transfer regardless of reserves |

This dual-gate design is intentional: ops can pause rebates at the
vault layer without touching FeesManagerV2 storage (`rebateBudget`
becomes a *governance* lever and `rebateReserve` becomes an *ops*
lever).

## CollateralVault interaction model

| Vault action | CollateralVault call | Caller |
|---|---|---|
| Receive positive fees | (passive — engine routes them in via `transferBetweenAccounts(asset, trader, vault, amt)`) | n/a |
| Pay rebates | (passive — engine routes them out via `transferBetweenAccounts(asset, vault, trader, amt)`) | n/a |
| Allocate fee proceeds → rebate reserve | Internal vault book-entry only (no CollateralVault touch) | vault owner / timelock |
| Withdraw revenue → revenueReceiver | `CollateralVault.transferBetweenAccounts(asset, vault, revenueReceiver, amount)` *(needs the vault to be authorized as a margin-engine; alternative: `withdrawFor(revenueReceiver, asset, amount)` if CollateralVault exposes a public withdraw path for internal accounts)* | vault owner / timelock |
| Sweep unsupported asset accidentally credited | Vault has no authority to move arbitrary tokens. `sweepUnsupported` is a NOOP placeholder that the design carries for future ERC20-mistake recovery; at launch the vault never holds raw ERC20 so the call reverts `NoUnsupportedAssetToSweep`. | vault owner |

### Authorization question (to be resolved before implementation)

`CollateralVault.transferBetweenAccounts` is gated by
`onlyMarginEngine`. For the vault to **pay out revenue**, it must
either:
1. Be authorized as a margin engine (overloaded but simple), OR
2. Use a different CollateralVault path that authorizes `from ==
   msg.sender` for internal-account transfers initiated by the
   account holder. (Not present today — would need a small
   CollateralVault ABI addition.)

**Recommendation:** add a new
`CollateralVault.transferFromInternalAccount(asset, to, amount)`
function gated by `msg.sender == from` semantics. This is a tiny,
audited surface extension (~15 LOC). The vault becomes the first
caller; the existing margin-engine path is unchanged.

Hold this decision open until the V2G-R implementation milestone.

## Admin functions

### `allocateToRebateReserve(address asset, uint256 amount)`

- Caller: `owner` (timelock).
- Effect: `feeBalance[asset] -= amount; rebateReserve[asset] += amount;`
- Reverts: `InsufficientFeeBalance` if `feeBalance[asset] < amount`.
- Emits: `FeeReserveAllocated(asset, amount)`.
- Note: pure internal book entry. Does NOT touch CollateralVault.
- Operational use: top up the rebate reserve from accumulated fees
  before rotating the `FeesManagerV2.rebateBudget`.

### `withdrawRevenue(address asset, uint256 amount)`

- Caller: `owner` (timelock).
- Effect: transfers `amount` of `asset` from vault to `revenueReceiver`
  via the (proposed) `CollateralVault.transferFromInternalAccount`.
  Decrements `feeBalance[asset]` and `netRevenue[asset]` accordingly.
- Reverts: `InsufficientFeeBalance`, `RevenueReceiverUnset`,
  `RebatesPaused` (if pause flag is set, to prevent withdrawing
  reserve-allocated funds — see invariant 4 below).
- Emits: `RevenueWithdrawn(asset, to, amount)`.

### `pauseRebates()` / `unpauseRebates()`

- Caller: `owner` or `guardian` (TBD — design suggests `guardian`
  for pause, `owner` for unpause, mirroring V2 engine pause patterns).
- Effect: flips `rebatesPaused`. While true, any
  `transferBetweenAccounts(asset, vault, *, *)` initiated by
  FeesManagerV2's rebate path reverts at the vault-side hook
  (Option β) or — if Option α is in force — the off-chain indexer
  raises an alert and ops must withdraw the rebate reserve manually.
- Emits: `RebatesPaused(by)` / `RebatesUnpaused(by)`.
- Operational use: emergency stop on suspicious rebate-claim activity
  without touching FeesManagerV2 storage.

### `sweepUnsupported(address token, address to)`

- Caller: `owner`.
- Effect: `IERC20(token).safeTransfer(to, IERC20(token).balanceOf(address(this)))`
  IFF `token` is NOT a supported settlement asset on CollateralVault.
- Reverts: `TokenIsSupportedSettlementAsset` if it is; the only path to move supported-asset funds is `withdrawRevenue`.
- Emits: `UnsupportedAssetSwept(token, to, amount)`.
- Rationale: launch-time the vault should never receive raw ERC20 by
  design. The sweep is a safety net for the unlikely case someone
  ERC20-transfers tokens directly to the vault address.

### `setRevenueReceiver(address newReceiver)`

- Caller: `owner`.
- Effect: replaces `revenueReceiver`. Non-zero check.
- Emits: `RevenueReceiverSet(old, new)`.

## Events

| Event | Emitted when |
|---|---|
| `FeeReserveAllocated(address indexed asset, uint256 amount)` | `allocateToRebateReserve` |
| `RevenueWithdrawn(address indexed asset, address indexed to, uint256 amount)` | `withdrawRevenue` |
| `RebatesPaused(address indexed by)` / `RebatesUnpaused(address indexed by)` | `pauseRebates` / `unpauseRebates` |
| `UnsupportedAssetSwept(address indexed token, address indexed to, uint256 amount)` | `sweepUnsupported` |
| `RevenueReceiverSet(address indexed previous, address indexed next)` | `setRevenueReceiver` |
| `FeeAccrued(address indexed asset, uint256 amount)` | Option β — when FeesManagerV2 calls `onFeeCharged` |
| `RebateAccrued(address indexed asset, uint256 amount)` | Option β — when FeesManagerV2 calls `onRebatePaid` |
| `OwnershipTransferred(address indexed previous, address indexed next)` | OZ-2step transfer |

All events use `indexed` consistently with V2 fee events so the
existing Grafana indexer dashboard surface is forward-compatible.

## Invariants

1. **Accounting identity.** For every supported asset:
   `grossFeesCollected[asset] - rebatesPaid[asset] == netRevenue[asset]`.
   Enforced by writing `netRevenue` only via the same code path that
   updates the other two counters.
2. **Internal-balance conservation.** For every supported asset:
   `feeBalance[asset] + rebateReserve[asset] == collateralVault.balances[address(this)][asset]`.
   Enforced by writing both buckets only on the same code paths
   that change the underlying CollateralVault balance.
3. **Monotonic counters.** `grossFeesCollected[asset]` and
   `rebatesPaid[asset]` never decrease. Enforced by the absence of
   any `-=` against them in the contract.
4. **Pause safety.** While `rebatesPaused == true`,
   `withdrawRevenue` cannot drain `rebateReserve[asset]` — only
   `feeBalance[asset]` is withdrawable. Prevents the operator from
   accidentally draining the rebate pool while rebates are paused
   for incident response.
5. **No ERC20 dust.** The vault never holds raw ERC20 of a
   supported settlement asset. All settlement-asset funds live in
   `CollateralVault.balances[vault][asset]`. Enforced by NOT having
   any code path that calls `IERC20.transferFrom` into the vault
   for supported assets.
6. **Single source of truth for rebate cap.** The vault NEVER
   replicates or shadows `FeesManagerV2.rebateBudget`. The vault's
   `rebateReserve` is a *funding pool*; the cap remains in
   `FeesManagerV2`.

## Security considerations

- **Reentrancy.** Vault external entry points (`allocateToRebateReserve`,
  `withdrawRevenue`, `pauseRebates`, `unpauseRebates`,
  `sweepUnsupported`, `setRevenueReceiver`, hooks if Option β) are
  `nonReentrant`. The `onFeeCharged` / `onRebatePaid` hooks are
  short and storage-only — no further external calls.
- **Hook auth.** Option β hooks are `onlyFeesManagerV2` gated. The
  vault stores the FM-V2 address as `immutable` so it cannot be
  upgraded silently.
- **Owner = timelock.** Ownership is transferred to
  `ProtocolTimelock` at deployment. No EOA holds owner authority
  in production.
- **Guardian for pause.** A separate, faster-reacting guardian
  EOA holds `pauseRebates` authority for incident response;
  `unpauseRebates` requires the owner / timelock.
- **2-step ownership.** Use OZ `Ownable2Step` to avoid accidental
  transfer.
- **No upgradeability.** Vault is non-upgradeable. To swap, deploy
  a new vault and rotate `FeesManagerV2.setFeeRecipient` +
  `FeesManagerV2.setRebateFundingAccount`. State migration is the
  CollateralVault `transferBetweenAccounts` from old-vault to
  new-vault for each asset.
- **Pause griefing.** Guardian holds pause authority — must be a
  trusted operator-side multisig; pausing rebates blocks live
  rebate trades. Mitigation: timelock can `unpauseRebates` at any
  time, no delay-window limit on the guardian's pause.
- **CollateralVault dependency.** All value moves go through
  `CollateralVault.transferBetweenAccounts` or
  `CollateralVault.transferFromInternalAccount`. The vault inherits
  any CollateralVault pause state (`whenInternalTransfersNotPaused`).
- **Settlement-asset hardening.** `sweepUnsupported` MUST check
  `(bool isSupported,,) = CollateralVault.getCollateralConfig(token)`
  and revert if `isSupported`. Prevents accidental settlement-asset
  drain via the sweep path.
- **No raw ETH.** Vault rejects ETH (`receive() / fallback()`
  revert).

## Migration plan from current `RiskGovernor` recipient

Current Base Sepolia state (per
`deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`):

- `feesManagerV2.feeRecipient = 0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588` (Timelock / RiskGovernor).
- `feesManagerV2.rebateFundingAccount = 0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588`.
- `feesManagerV2.rebateBudget(mUSDC) = 0`.
- `feesManagerV2.merkleRoot = 0x000…`.

### Migration order

1. **Deploy `ProtocolFeeVault` (constructor)** — fix immutables: `collateralVault`, `feesManagerV2`. Pass `owner = ProtocolTimelock`, `guardian = OPS_MULTISIG`, `revenueReceiver = TBD`.
2. **Pre-flight sanity** (offline): assert `vault.collateralVault() == 0x00340c…`, `vault.feesManagerV2() == 0x00dA0B…`, `vault.owner() == 0xa67f8e…`, all four bucket mappings = 0, `rebatesPaused == false`.
3. **(Option β only)** Add the `onFeeCharged` / `onRebatePaid` hook on the upgraded `FeesManagerV2`. This requires a FM-V2 redeploy — fold into V2G-O / V2G-P milestone (the OPTION RFQ broadcast wave).
4. **Drain the old Timelock account.** Use
   `CollateralVault.transferBetweenAccounts(asset, timelock, vault, balance)`
   to move all per-asset balances from the Timelock internal account
   into the vault's internal account. Today this is 0 (`rebateBudget = 0`)
   so this step is a noop, but it's part of the procedure.
5. **Rotate recipients:**
   - `FeesManagerV2.setFeeRecipient(vault)`.
   - `FeesManagerV2.setRebateFundingAccount(vault)`.
6. **Backfill `feeBalance` / `grossFeesCollected`** by replaying
   the V2 fee event stream (the V2G-G indexer has the full history)
   into the vault's storage via a one-shot `bootstrap(asset,
   gross, rebates)` owner-only function. Lock the bootstrap behind
   a `bootstrapped[asset]` flag so it can only run once per asset.
7. **Wire monitoring** (see below).
8. **Confirm health** — first live OPTION/PERP trade post-cutover
   should produce a `FeeChargedV2` event whose `recipient == vault`,
   visible in `/admin/fees/onchain`.

### Rollback plan

If the vault misbehaves before any settlement-asset balance has
been transferred:

- `FeesManagerV2.setFeeRecipient(timelock)` — restore the old recipient.
- `FeesManagerV2.setRebateFundingAccount(timelock)` — restore the old funder.

This is two transactions; the vault is left orphaned but causes no
harm. After balances have been transferred, rollback also requires
`CollateralVault.transferBetweenAccounts(asset, vault, timelock,
balance)` for each asset.

### Compatibility with V2G-P / V2G-O

V2G-O / V2G-P (the OPTION RFQ broadcast) is independent of V2G-R.
The vault deploy can happen before, after, or alongside the V2G-P
redeploy:

- **Before V2G-P:** vault is wired against the current FM-V2; OPTION
  RFQ broadcasts continue to charge fees / pay rebates exactly as
  today, just routed through the vault.
- **After V2G-P:** if V2G-P upgrades FM-V2 (e.g. for Option β
  hooks), the vault constructor's `feesManagerV2` immutable points
  at the new FM-V2 address; this is a fresh-deploy concern, not a
  migration concern.
- **Alongside V2G-P:** combine into a single operator window —
  deploy FM-V2', deploy ProtocolFeeVault, wire FM-V2'.setFeeRecipient(vault),
  then run the V2G-P OPTION redeploy.

The V2G-R design is intentionally orthogonal — it does NOT require
the V2G-P redeploy as a precondition.

## Monitoring metrics / dashboard changes

The V2G-G Grafana dashboard currently surfaces (per asset, via
`/metrics`):

- `deopt_fees_charged_v2_total{product, flow}`
- `deopt_fees_rebated_v2_total{product, flow}`
- `deopt_rebate_budget_balance{asset}` (from FM-V2 `rebateBudget`).

V2G-R adds:

| Metric | Source | Purpose |
|---|---|---|
| `deopt_fee_vault_gross_collected_total{asset}` | `vault.grossFeesCollected(asset)` | Cumulative fees seen by the vault — should match `sum(fees_charged_v2_total)` per asset. Drift alarm. |
| `deopt_fee_vault_rebates_paid_total{asset}` | `vault.rebatesPaid(asset)` | Cumulative rebates — should match `sum(fees_rebated_v2_total)`. Drift alarm. |
| `deopt_fee_vault_net_revenue{asset}` | `vault.netRevenue(asset)` | Live net revenue per asset. |
| `deopt_fee_vault_fee_balance{asset}` | `vault.feeBalance(asset)` | Withdrawable revenue per asset. |
| `deopt_fee_vault_rebate_reserve{asset}` | `vault.rebateReserve(asset)` | Reserve pool — should track ≥ `rebateBudget` per V2G-R policy. |
| `deopt_fee_vault_rebates_paused{}` | `vault.rebatesPaused()` (gauge 0/1) | Critical alert on pause. |
| `deopt_fee_vault_internal_balance{asset}` | `collateralVault.balances(vault, asset)` | Should equal `feeBalance + rebateReserve` per invariant 2. Drift alarm. |

### Grafana panel changes

- **New panel** "Vault revenue per asset" — stacked area of
  `gross_collected_total` minus `rebates_paid_total`.
- **New panel** "Vault buckets" — two-stack bar of `feeBalance` +
  `rebateReserve` per asset.
- **New alert** `FeeVaultDrift` — when
  `gross_collected_total - rebates_paid_total != fee_balance + rebate_reserve`
  for more than 2 evaluation windows.
- **New alert** `FeeVaultRebatesPaused` — `rebates_paused == 1`.
- **New alert** `FeeVaultReserveShortfall` —
  `rebate_reserve(asset) < feesManagerV2.rebateBudget(asset)` for
  more than 5 evaluation windows (the cap is funded by reserve;
  shortfall means future rebates will revert at the CollateralVault
  hop).

### Backend indexer changes

The V2G-G OnchainFeeEvent indexer needs zero changes — it already
indexes `FeeChargedV2.recipient` and `FeeRebatedV2.recipient`. The
admin endpoint `/admin/fees/onchain` will start seeing
`recipient = vault` for both leg types after migration; the
addresses surface in the existing JSON payloads without any decoder
modification.

A *new* indexer for the vault's own events (`FeeReserveAllocated`,
`RevenueWithdrawn`, `RebatesPaused`/`RebatesUnpaused`,
`UnsupportedAssetSwept`, `RevenueReceiverSet`, optionally
`FeeAccrued` / `RebateAccrued` for Option β) ships as part of
V2G-R-backend (separate milestone).

## Test plan

### Unit tests (Solidity, offline)

| Subject | Test |
|---|---|
| Constructor rejects zero `collateralVault` / zero `feesManagerV2` / zero owner | `test_constructorRejectsZeroAddresses` |
| Default bucket state is all zero | `test_initialBucketsAreZero` |
| `rebatesPaused == false` at deploy | `test_initialPauseFlagIsFalse` |
| `allocateToRebateReserve` moves from `feeBalance` to `rebateReserve` | `test_allocateToReserveBalances` |
| `allocateToRebateReserve` reverts on insufficient `feeBalance` | `test_allocateReverts_InsufficientFeeBalance` |
| `withdrawRevenue` decrements `feeBalance` + `netRevenue` and credits `revenueReceiver` (via stubbed CollateralVault) | `test_withdrawRevenueHappyPath` |
| `withdrawRevenue` refuses to withdraw `rebateReserve` while `rebatesPaused == true` | `test_withdrawRevenueBlockedByPause` |
| `pauseRebates` / `unpauseRebates` event + access control | `test_pauseUnpauseGuard` |
| `sweepUnsupported` reverts when token is a supported settlement asset | `test_sweepUnsupportedRefusesSupportedAsset` |
| `sweepUnsupported` succeeds for arbitrary stray ERC20 | `test_sweepUnsupportedHappyPath` |
| `setRevenueReceiver` ownership + non-zero | `test_setRevenueReceiverGuard` |
| Invariant 2 (balance conservation) under random `accrue` mutations | `invariant_balanceConservation` |
| Invariant 1 (accounting identity) holds across all admin actions | `invariant_accountingIdentity` |

### Integration tests (Solidity, offline)

| Subject | Test |
|---|---|
| Full positive-fee path: engine → FM-V2 → vault internal account (option β hook updates buckets) | `integration_positiveFeeAccrual` |
| Full rebate path: engine → FM-V2 → vault internal account (option β hook updates buckets) | `integration_rebatePayout` |
| Rebate path reverts when `rebatesPaused == true` (Option β) | `integration_pausedRebateReverts` |
| Migration step: balance moves from old Timelock account → new vault account, vault.bootstrap registers gross/rebates | `integration_migrationCutover` |
| Rollback: setFeeRecipient back to Timelock + balance moves back | `integration_rollback` |

### Fuzz tests

| Subject | Test |
|---|---|
| Random sequence of `consumeFees` + `allocateToRebateReserve` + `withdrawRevenue` preserves invariant 1 + 2 | `fuzz_vaultStateConsistency` |
| Pause / unpause arbitrary windows do not allow `withdrawRevenue` of `rebateReserve` during pause | `fuzz_pauseSafety` |

### Operator tooling

- New `script/PreflightProtocolFeeVault.s.sol` — read-only sanity
  script for the post-deploy operator window.
- New `script/RewireFeesManagerV2RecipientToVault.s.sol` —
  safe-by-default rewire of FM-V2 → vault, gated by
  `REWIRE_FEE_VAULT_CONFIRM=true`.

## Implementation milestones (not in V2G-R scope)

| Milestone | Deliverable |
|---|---|
| **V2G-R0** (this doc) | Design spec only. **DONE.** |
| **V2G-R1** | Implement `ProtocolFeeVault.sol` + unit + invariant tests offline, no deploy. **DONE — see `deopt-v2-sol/docs/PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md`.** |
| **V2G-R2** | Add `onFeeCharged` / `onRebatePaid` hooks to FM-V2 (or pick Option α). |
| **V2G-R3** | Operator scripts (`Preflight…`, `Rewire…`) + integration tests. |
| **V2G-R4** | Backend metrics + Grafana panels + alerts. |
| **V2G-R5** | Live deploy + cutover + first revenue withdrawal. |

V2G-R1 is small (~300 LOC + tests) and can be folded into the same
operator window as V2G-P if scheduling allows. The four downstream
milestones each independently leave the system in a working state
(the migration can pause between any two if needed).

## Files changed in V2G-R0

- **New:** `deopt-v2-backend/docs/PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md` (this file).

No Solidity sources, no scripts, no backend code, no frontend code,
no docker, no `.env`, no live state touched.

## Soak preservation status

| Check | State at V2G-R0 close |
|---|---|
| Backend PID 56199 alive | ✅ (16h22m+ at the time of the read-only check) |
| `/health` | ✅ `{"ok":true,...}` |
| Compose containers up | ✅ Grafana / Prometheus / Alertmanager / webhook-sink all healthy 16h+ |
| No `docker compose down` | ✅ |
| No backend restart | ✅ |
| No Prometheus reset | ✅ |
| No `.env` edit | ✅ |
| Day-1 24h gate `2026-06-01T17:38Z` | reserved (not yet ticked) |

## Remaining decisions (to resolve before V2G-R1 ships)

1. **Option α vs Option β** for the accrue path. Recommendation:
   Option β. Final call deferred to V2G-R1 PR review.
2. **`transferFromInternalAccount` ABI extension on
   `CollateralVault`** — exact signature, gating (`msg.sender == from`
   vs an allow-list), audit scope.
3. **Guardian** address for `pauseRebates`. Recommendation:
   `OPS_MULTISIG` (the same multisig that holds engine pause
   authority today, per V2D-K registry).
4. **`revenueReceiver`** identity at launch. Recommendation:
   ProtocolTimelock (same as today's recipient EOA) — keeps revenue
   under the same governance for launch, with the option to point
   it at a separate treasury contract later.
5. **Bootstrap policy** — replay V2 fee history into the vault's
   internal counters on cutover, or accept that `grossFeesCollected`
   starts at the cutover-block number and let the indexer expose
   the pre-cutover history out-of-band. Recommendation: bootstrap,
   so the vault is the single source of truth on day 1.
6. **PERP integration scope**. The vault supports PERP fees by
   construction (PERP uses the same `consumeFees(asset, ...)` path).
   No PERP-specific work; flagging only because V2G-R is OPTION-
   adjacent and the operator may assume PERP is out of scope —
   it's not.

## Final report

- **Design summary**: single `ProtocolFeeVault` contract holding
  per-asset buckets (`feeBalance`, `rebateReserve`,
  `grossFeesCollected`, `rebatesPaid`, `netRevenue`) inside its own
  `CollateralVault` internal account. Wired to `FeesManagerV2` via
  the two existing setters (`setFeeRecipient`,
  `setRebateFundingAccount`). Optional FM-V2 hook (Option β)
  notifies the vault on each fee / rebate consumption so the
  buckets stay on-chain consistent.
- **Files changed**:
  `deopt-v2-backend/docs/PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md` (new).
- **Docs updated**: none beyond the new file — the existing
  V2G-N / V2G-O / V2G-P0 / V2G-P1 / V2G-Q docs make no claim about
  fee-recipient identity beyond "Timelock," and the migration
  changes that pointer without invalidating any prior recorded
  result. The existing fee-architecture references in
  `deopt-v2-sol/docs/FEES_MANAGER_V2_DESIGN_SPEC_V2D_A.md` describe
  the FM-V2 math layer and remain accurate.
- **Invariants** (6): accounting identity, internal-balance
  conservation, monotonic counters, pause safety, no ERC20 dust,
  single source of truth for rebate cap.
- **Migration plan**: 8 ordered steps (deploy → preflight → optional
  FM-V2 hook upgrade → drain Timelock → rotate recipients →
  bootstrap counters → wire monitoring → confirm health). Two-tx
  rollback if balances haven't been transferred yet.
- **Remaining decisions** (6): Option α vs β, CollateralVault ABI
  extension, guardian identity, revenue receiver, bootstrap
  policy, PERP scope.
- **Soak preservation**: ✅ PID 56199 alive 16h22m+, Prometheus
  healthy, all 4 compose containers Up 16h+, no live touch.
