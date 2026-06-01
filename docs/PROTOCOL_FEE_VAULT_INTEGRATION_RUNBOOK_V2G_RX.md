# V2G-RX — ProtocolFeeVault Integration Runbook

## Status

- Milestone: **V2G-RX** — operator-facing runbook for the
  V2G-R5 vault cutover. Consolidates V2G-R0 (design) + V2G-R1
  (offline implementation) into a step-by-step plan covering
  FM-V2 hook ABI extension, CollateralVault internal-transfer
  extension, vault bootstrap, recipient rotation, and the
  V2G-R-Monitoring metric/alert spec. **Docs-only.** No deploy,
  no broadcast.
- Date: 2026-06-01.

---

## 1. Pre-broadcast checklist

| Gate | Required | Source of truth |
|---|---|---|
| V2G-R1 offline implementation + 45 tests passing | ✅ | `deopt-v2-sol/docs/PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md` |
| FeesManagerV2 hook ABI extension drafted | this doc, §3 | — |
| CollateralVault `transferFromInternalAccount` extension drafted | this doc, §4 | — |
| Bootstrap data captured from V2G-G event history | yes | indexer DB → `option_execution_events` filtered by `FeeChargedV2`/`FeeRebatedV2` |
| Governance multisig / timelock target | identified | `deployments/base-sepolia.manifest.draft.json::contracts.ProtocolTimelock` |
| Operator EOA funded | yes | — |
| Rollback rehearsal completed offline | yes | this doc, §8 |

---

## 2. Live state recap

| Surface | Current | Target after V2G-R5 |
|---|---|---|
| `FeesManagerV2.feeRecipient` | `0xa67f8e…b588` (Timelock EOA) | `<ProtocolFeeVault address>` |
| `FeesManagerV2.rebateFundingAccount` | `0xa67f8e…b588` (Timelock EOA) | `<ProtocolFeeVault address>` |
| `FeesManagerV2.consumeFees` | charges/rebates land at Timelock | charges/rebates land at vault internal CV account |
| Per-asset gross / rebates / net | inferred from event history | first-class buckets in `ProtocolFeeVault` |
| `CollateralVault.transferFromInternalAccount` | does not exist | extension lands as part of V2G-R3 |

---

## 3. FeesManagerV2 hook integration plan (V2G-R3)

### 3.1 ABI delta

Two new external view-callable hooks on the trusted
`ProtocolFeeVault` instance:

```solidity
interface IProtocolFeeVaultHook {
    function onFeeCharged(address asset, uint256 amount) external;
    function onRebatePaid(address asset, uint256 amount) external;
}
```

(Already implemented in V2G-R1 — `IProtocolFeeVault` interface +
`ProtocolFeeVault.sol` body.)

### 3.2 FM-V2 call-site delta

Inside `FeesManagerV2.consumeFees`, after the existing
`_collateralVault.transferBetweenAccounts(...)`:

```solidity
// V2G-R3 — notify the configured fee-recipient if it is a
// hook-aware contract. Backwards-compatible: legacy EOA
// recipients receive no callback (this is the V2G-R5 cutover
// hook that turns ProtocolFeeVault bucket totals into the
// canonical on-chain record).
address callback = feeRecipientCallback;
if (callback != address(0) && q.feeAmount != 0) {
    if (q.isRebate) {
        IProtocolFeeVaultHook(callback).onRebatePaid(settlementAsset, q.feeAmount);
    } else {
        IProtocolFeeVaultHook(callback).onFeeCharged(settlementAsset, q.feeAmount);
    }
}
```

Setter (owner-only, additive):

```solidity
function setFeeRecipientCallback(address newCallback) external onlyOwner {
    address oldCallback = feeRecipientCallback;
    feeRecipientCallback = newCallback;
    emit FeeRecipientCallbackSet(oldCallback, newCallback);
}
```

### 3.3 Backwards compatibility

| Scenario | Behavior |
|---|---|
| `feeRecipientCallback == address(0)` (default) | No hook called. FM-V2 behaves bit-equivalent to today. |
| `feeRecipientCallback != address(0)` but recipient EOA differs | Hook still fires — but the underlying transfer lands at the EOA, not the vault. **Don't configure this combination** — it's a misconfig that the integration test should catch. |
| `feeRecipientCallback == address(vault)` AND `feeRecipient == address(vault)` AND `rebateFundingAccount == address(vault)` | Canonical V2G-R5 state. Hook updates the vault's accounting buckets in lockstep with the underlying CV transfer. |

### 3.4 Risk

| Risk | Mitigation |
|---|---|
| Hook callback reverts and bricks `consumeFees` | Wrap in `try { } catch { emit HookFailed(...); }` — fee charge succeeds; vault accounting may be temporarily out of sync; ops sees `HookFailed` alert. |
| Hook callback is malicious | `feeRecipientCallback` setter is `onlyOwner` → Timelock controls. Bound by the same trust as `setFeeRecipient`. |
| Re-entrancy via hook | Hook is restricted to internal SSTORE only (per V2G-R1 audit); no external calls. FM-V2 hook call sits *after* the `transferBetweenAccounts` so the vault sees a consistent CV balance. |

### 3.5 Tests to add at V2G-R3 implementation time

| Test | Asserts |
|---|---|
| `testV2GR3_HookFiresOnPositiveFee` | `onFeeCharged(asset, amount)` emitted from vault under `consumeFees`. |
| `testV2GR3_HookFiresOnRebate` | `onRebatePaid(asset, amount)` emitted under rebate consumption. |
| `testV2GR3_NoCallbackWhenRecipientIsEOA` | When `feeRecipientCallback == 0`, no extra event. |
| `testV2GR3_HookRevertDoesNotBlockConsumeFees` | Malicious vault stub throws in `onFeeCharged`; `consumeFees` still completes; `HookFailed` event observed. |
| `testV2GR3_HookOnlyCallableByFmV2` (vault side) | `onFeeCharged`/`onRebatePaid` `onlyFeesManagerV2` modifier (V2G-R1 already pins this). |

---

## 4. CollateralVault internal-transfer extension plan (V2G-R3)

### 4.1 ABI delta

```solidity
interface ICollateralVaultInternalTransfer {
    function transferFromInternalAccount(address asset, address to, uint256 amount) external;
}
```

(Already drafted in `deopt-v2-sol/src/collateral/ICollateralVaultInternalTransfer.sol`
as the V2G-R1 forward-compatible interface — not implemented on
the live CV.)

### 4.2 CV call-site delta

```solidity
// New function on CollateralVault, alongside transferBetweenAccounts.
//
// V2G-R3 — lets an internal-account holder (e.g. ProtocolFeeVault)
// debit its own account and credit an external recipient. Gated by
// `msg.sender == from` semantics — the caller can only move its own
// balance.
function transferFromInternalAccount(address asset, address to, uint256 amount)
    external
    whenInternalTransfersNotPaused
    nonReentrant
{
    if (to == address(0)) revert ZeroAddress();
    if (amount == 0) revert AmountZero();
    if (!_collateralConfigs[asset].isSupported) revert TokenNotSupported();

    // msg.sender debits its own internal account.
    address from = msg.sender;
    _sync(from, asset);

    uint256 fromBal = balances[from][asset];
    if (fromBal < amount) revert InsufficientBalance();
    balances[from][asset] = fromBal - amount;

    // External ERC20 transfer to `to`.
    IERC20(asset).safeTransfer(to, amount);

    emit InternalTransferOut(from, asset, to, amount);
}
```

### 4.3 Backwards compatibility

Pure additive. Existing `transferBetweenAccounts` /
`onlyMarginEngine` path is unchanged.

### 4.4 Risk

| Risk | Mitigation |
|---|---|
| Any caller can drain its own internal balance | By design — the caller can only debit its OWN account (`msg.sender == from`). |
| Non-vault caller drains its account to an attacker | `msg.sender` must already hold the balance — there's no path for a non-vault to accumulate one. |
| Re-entrancy via `to.fallback()` | `nonReentrant` + the external transfer is the LAST step in the function. |
| Paused state | `whenInternalTransfersNotPaused` modifier — operator can pause CV-side moves during incident response. |

### 4.5 Tests to add

| Test | Asserts |
|---|---|
| `testV2GR3_CVInternalTransferDebitsCaller` | `balances[caller][asset]` decremented by `amount`. |
| `testV2GR3_CVInternalTransferCreditsExternal` | `IERC20(asset).balanceOf(to)` increased by `amount`. |
| `testV2GR3_CVInternalTransferRevertsWhenInsufficient` | Caller balance < amount ⇒ `InsufficientBalance`. |
| `testV2GR3_CVInternalTransferRevertsOnPause` | Pause flag set ⇒ revert. |
| `testV2GR3_CVInternalTransferRejectsZeroTo` | `to == 0` ⇒ `ZeroAddress`. |

---

## 5. Bootstrap accounting

After the V2G-R5 cutover, the vault's `grossFeesCollected` and
`rebatesPaid` counters should reflect the V2G-G + V2G-P event
history, not start at zero. Otherwise per-asset net-revenue reads
will be misleading until enough new V2 fees accumulate.

### 5.1 Bootstrap inputs (per asset)

| Field | Source |
|---|---|
| `grossFees` | `SELECT SUM(decoded->>'feeAmount') FROM option_execution_events WHERE event_name='FeeChargedV2' AND chain_id=84532 GROUP BY decoded->>'settlementAsset'` |
| `rebates` | `SELECT SUM(decoded->>'rebateAmount') FROM option_execution_events WHERE event_name='FeeRebatedV2' AND chain_id=84532 GROUP BY decoded->>'settlementAsset'` |
| `feeBalance` | gross - rebates - cumulative-withdrawn (if any) |
| `rebateReserve` | 0 at cutover (top up via `allocateToRebateReserve` post-bootstrap) |

### 5.2 Bootstrap procedure

Per asset, in one tx (broadcast by the Timelock):

```bash
cast send $VAULT 'bootstrap(address,uint256,uint256,uint256,uint256)' \
  $ASSET $GROSS $REBATES $FEEBALANCE $RESERVE \
  --private-key $TIMELOCK_PK
```

The vault enforces (V2G-R1):
- `rebates <= grossFees`
- `feeBalance + rebateReserve <= grossFees - rebates`
- `bootstrapped[asset] == false` → flips to `true` (one-shot per asset)

### 5.3 Verification

```bash
cast call $VAULT 'grossFeesCollected(address)(uint256)' $ASSET
cast call $VAULT 'rebatesPaid(address)(uint256)' $ASSET
cast call $VAULT 'feeBalance(address)(uint256)' $ASSET
cast call $VAULT 'rebateReserve(address)(uint256)' $ASSET
cast call $VAULT 'netRevenue(address)(uint256)' $ASSET
cast call $VAULT 'bootstrapped(address)(bool)' $ASSET
```

---

## 6. Cutover broadcast order (V2G-R5)

```
0. (off-broadcast)   Build vault bootstrap data per asset.
1. Deploy            ProtocolFeeVault with constructor immutables
                     (owner=Timelock, collateralVault=live,
                      feesManagerV2=live).
2. Deploy            FM-V2 + CV extension (per V2G-R3 above).
                     New FM-V2 deployed alongside the old one
                     during the cutover window if the existing FM-V2
                     can't be upgraded.
3. Wire              FM-V2.setFeeRecipientCallback(vault).
4. Wire              FM-V2.setFeeRecipient(vault).
5. Wire              FM-V2.setRebateFundingAccount(vault).
6. Drain old account Move balances Timelock → vault via
                     transferBetweenAccounts.
7. Bootstrap         vault.bootstrap(asset, gross, rebates, feeBalance, 0) per asset.
8. Verify            New fee/rebate tx flows produce identical
                     transfer + new vault.FeeRecorded/RebateRecorded events.
9. Monitor           Wait for the V2G-RX-Monitoring alerts (§7)
                     to land in Prometheus/Grafana and confirm green.
```

### 6.1 Step ordering rationale

- **Callback wiring (Step 3) must precede recipient rotation (Steps 4–5)** so the first fee event after rotation already updates the vault buckets. Otherwise the hook fires on the old recipient (EOA) and contributes nothing.
- **Drain (Step 6) must precede bootstrap (Step 7)** so the vault's internal CV balance matches `feeBalance + rebateReserve` (V2G-R1 invariant 2).
- **Bootstrap must precede verification (Step 8)** so the per-asset counters reflect history before the first new fee lands.

---

## 7. Monitoring metrics + alerts (V2G-RX-Monitoring)

### 7.1 New Prometheus metrics

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_fee_vault_fee_balance` | gauge | `asset` | `vault.feeBalance(asset)` |
| `deopt_fee_vault_rebate_reserve` | gauge | `asset` | `vault.rebateReserve(asset)` |
| `deopt_fee_vault_gross_collected_total` | counter | `asset` | `vault.grossFeesCollected(asset)` |
| `deopt_fee_vault_rebates_paid_total` | counter | `asset` | `vault.rebatesPaid(asset)` |
| `deopt_fee_vault_net_revenue` | gauge | `asset` | `vault.netRevenue(asset)` |
| `deopt_fee_vault_rebates_paused` | gauge (0/1) | — | `vault.rebatesPaused()` |
| `deopt_fee_vault_cv_internal_balance` | gauge | `asset` | `collateralVault.balances(vault, asset)` |

### 7.2 New alerts

| Alert | Condition | Severity |
|---|---|---|
| `FeeVaultDrift` | `gross_collected_total{asset} - rebates_paid_total{asset} != fee_balance{asset} + rebate_reserve{asset} + cumulative_withdrawn{asset}` for 2 evaluation windows | high |
| `FeeVaultRebatesPaused` | `rebates_paused == 1` for any duration | critical |
| `FeeVaultReserveShortfall` | `rebate_reserve{asset} < FeesManagerV2.rebateBudget{asset}` for 5 windows | high |
| `FeeVaultCVBalanceMismatch` | `fee_balance{asset} + rebate_reserve{asset} != cv_internal_balance{asset}` for 1 window | critical (invariant 2 violated) |
| `FeeVaultHookFailed` | `HookFailed` event observed via the indexer | high |

### 7.3 Grafana panels

Add to the V2G-G dashboard (folder `DeOpt`, uid
`deopt-v2g-g-v2-fees`):

| Panel | Type | Query |
|---|---|---|
| Vault: gross vs net revenue per asset | timeseries | `deopt_fee_vault_gross_collected_total{asset}` and `deopt_fee_vault_net_revenue{asset}` |
| Vault: fee balance + rebate reserve (stacked) | timeseries | `deopt_fee_vault_fee_balance + deopt_fee_vault_rebate_reserve` |
| Vault: CV internal balance vs sum-of-buckets | timeseries | Two-line overlay to surface drift visually |
| Vault: rebates paused (gauge) | stat | `deopt_fee_vault_rebates_paused` (0 = green, 1 = red) |

---

## 8. Rollback plan

| When | Action |
|---|---|
| Before Step 4 (recipient rotation) | Take no action — vault sits idle; FM-V2 still uses Timelock EOA. |
| Between Steps 4–5 | `setFeeRecipient(timelock_eoa)` — two-tx revert. |
| After Step 5 (rebate funding rotated) | `setRebateFundingAccount(timelock_eoa)` + `setFeeRecipient(timelock_eoa)` — two-tx revert. |
| After Step 6 (balances drained) | Per asset: `CollateralVault.transferBetweenAccounts(asset, vault, timelock_eoa, balance)`. Then revert §4–5 setters. |
| After Step 7 (bootstrap) | `bootstrap[asset] == true` is permanent (V2G-R1 design). If the vault is being abandoned, the bootstrap stays as historical record — no on-chain rollback needed for the counters themselves. |
| After Step 8 (live fees flowing) | Same as Step 6 — re-rotate recipients + drain. The vault's internal account is fully recoverable. |

Emergency stop:

```bash
# Pause rebates at the vault layer (V2G-R1 method):
cast send $VAULT 'pauseRebates()' --private-key $GUARDIAN_PK
```

This makes the next `onRebatePaid` revert at the vault hook (per
§3.5 `testV2GR3_HookRevertDoesNotBlockConsumeFees`,
`consumeFees` itself still completes — the rebate accounting just
goes stale until ops investigates).

---

## 9. Acceptance criteria for V2G-R5 close

- [ ] Vault deployed.
- [ ] FM-V2 + CV extensions deployed (or accepted as carry-forward).
- [ ] Recipient + funding-account rotated to vault.
- [ ] Per-asset bootstrap completed; `bootstrapped[asset] == true`.
- [ ] First new fee event after cutover updates vault buckets (verified via `FeeRecorded` event + `deopt_fee_vault_*` metric tick).
- [ ] CV internal balance matches `feeBalance + rebateReserve` (invariant 2).
- [ ] `FeeVaultDrift`, `FeeVaultCVBalanceMismatch`, `FeeVaultHookFailed` alerts not firing.
- [ ] Grafana vault panels live.

---

## 10. Cross-links

- V2G-R0 design: `docs/PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md`
- V2G-R1 implementation: `deopt-v2-sol/docs/PROTOCOL_FEE_VAULT_IMPLEMENTATION_V2G_R1.md`
- V2G-T canonical pack: `docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`
- V2G-PX RFQ runbook: `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`

---

## 11. Hard-gate compliance

This runbook broadcasts nothing. Every cutover step requires
operator approval at the timelock/multisig surface. No live state
changed by reading this doc.
