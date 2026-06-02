# V2G-Y — Governance / Admin Safety Matrix + Emergency Runbooks

## Status

- Milestone: **V2G-Y** — full owner / guardian / timelock surface
  audit across the V2 contract suite + emergency-action runbooks.
  **Docs-only.** No live governance action.
- Date: 2026-06-01.

---

## 1. Owner / guardian surface inventory

Audited via `grep` on the live V2 contracts in `~/DEOPT/deopt-v2-sol/src`.

### 1.1 `FeesManagerV2.sol`

| Function | Authority | Risk |
|---|---|---|
| `transferOwnership(newOwner)` | owner | critical — full takeover |
| `setFeeRecipient(addr)` | owner | high — diverts revenue |
| `setRebateFundingAccount(addr)` | owner | high — diverts rebate funds; `addr=0` disables rebates |
| `setFeeConsumer(consumer, allowed)` | owner | high — allow-list of engines |
| `setMerkleRoot(root, validFrom, validUntil)` | owner | critical — controls all tier claims |
| `setFeeProfile(tier, product, makerPpm, takerPpm)` | owner | critical — economic policy |
| `setRfqDiscountProfile(tier, product, makerD, takerD)` | owner | critical — RFQ economic policy |
| `fundRebateBudget(asset, amount)` | owner | high — adds liability against the funding account |
| `withdrawRebateBudget(asset, amount, to)` | owner | high — pulls budget out |

### 1.2 `MarginEngine` (`MarginEngineAdmin.sol`)

| Function | Authority | Risk |
|---|---|---|
| `transferOwnership(newOwner)` | owner | critical |
| `setGuardian(addr)` | owner | high — gives pause authority |
| `pauseTrading/Liquidation/Settlement/CollateralOps` | guardian-or-owner | low (defensive) |
| `unpauseTrading/Liquidation/Settlement/CollateralOps` | owner | high — resumes after incident; must be reviewed |
| `setMatchingEngine(addr)` | owner | critical |
| `setOracle(addr)` | owner | critical |
| `setRiskModule(addr)` | owner | critical |
| `setInsuranceFund(addr)` | owner | high |
| `setFeesManager(addr)` (V1) | owner | medium — V1 fallback only |
| `setFeesManagerV2(addr)` | owner | critical — V2 path |
| `setUseFeesManagerV2(bool)` | owner | critical — flips V2 fee path on/off |
| `setFeeRecipient(addr)` (V1) | owner | medium — V1 fallback only |
| `setSeriesEmergencyCloseOnly(optionId, bool)` | guardian-or-owner | medium (per-series defensive) |
| `setSeriesActivationState(optionId, uint8)` | owner | high — series enablement |
| `setSeriesShortOpenInterestCap(...)` | owner | medium — risk policy |
| `setRiskParams(...)` | owner | high — global IM / MM bps |
| `setLiquidation*` | owner | high — liquidation economics |

### 1.3 `PerpEngine` (`PerpEngineAdmin.sol`)

(same admin surface as MarginEngine plus PERP-specific:)

| Function | Authority | Risk |
|---|---|---|
| `setMarketActivationState(marketId, state)` | owner | high |
| `setMarketEmergencyCloseOnly(marketId, bool)` | guardian-or-owner | medium |
| `setLaunchOpenInterestCap(marketId, cap)` | owner | medium |
| `setCollateralVault(addr)` | owner | critical |
| `setCollateralSeizer(addr)` | owner | high |

### 1.4 `CollateralVault` (`CollateralVaultAdmin.sol`)

| Function | Authority | Risk |
|---|---|---|
| `transferOwnership(newOwner)` | owner | critical |
| `setGuardian(addr)` | owner | high |
| `pauseDeposits/Withdrawals/InternalTransfers/YieldOps` | guardian-or-owner | low (defensive) |
| `unpause*` | owner | high |
| `setMarginEngine(addr)` | owner | critical — authoritative engine for `transferBetweenAccounts` |
| `setAuthorizedEngine(engine, allowed)` | owner | high (post-V2D-L pattern) |
| `setRiskModule(addr)` | owner | critical |
| `setTokenDepositCap(token, cap)` | owner | medium |
| `setCollateralRestrictionMode(bool)` | owner | medium — collateral whitelist on/off |
| `setLaunchActiveCollateral(token, bool)` | owner | medium |
| `setTokenStrategy(token, adapter)` | owner | high — yield adapter swap |
| `setCollateralToken(token, supported, decimals, factorBps)` | owner | high — adds/removes supported asset |

### 1.5 `OptionMatchingEngine` (post-V2G-P)

| Function | Authority | Risk |
|---|---|---|
| `transferOwnership(newOwner)` | owner | critical |
| `setGuardian(addr)` | owner | high |
| `pause/unpause` | guardian / owner | low / high |
| `setExecutor(addr, allowed)` | owner | high — allow-list of broadcast wallets |
| `setEngine(addr)` (MarginEngine) | owner | critical |
| `setRegistry(addr)` | owner | high |

### 1.6 `ProtocolFeeVault` (V2G-R1 + V2G-RX.1)

| Function | Authority | Risk |
|---|---|---|
| `transferOwnership(newOwner)` | owner | critical |
| `setRevenueReceiver(addr)` | owner | high |
| `setGuardian(addr)` | owner | high (V2G-RX.1) |
| `pauseRebates` | **guardian or owner** (V2G-RX.1) | medium (fast-pause) |
| `unpauseRebates` | owner | high |
| `allocateToRebateReserve(asset, amount)` | owner | medium |
| `withdrawRevenue(asset, to, amount)` | owner | high |
| `bootstrap(asset, gross, rebates, feeBalance, reserve)` | owner | high (one-shot per asset) |

> **V2G-RX.1 update:** the V2G-R0 design intent (guardian for fast
> pause, owner-only for unpause and heavy admin) is now implemented.
> `setGuardian` is owner-only and accepts `address(0)` to explicitly
> disable the fast-pause posture. Deployments shipping with
> `guardian == address(0)` must record
> `ALLOW_ZERO_GUARDIAN_CONFIRM=true` in the broadcast window log.

### 1.7 Cross-contract summary

| Risk class | Count | Examples |
|---|---|---|
| critical (full takeover or recipient/setter change) | 16 | `transferOwnership`, `setMerkleRoot`, `setFeesManagerV2`, `setMatchingEngine`, `setMarginEngine`, `setOracle`, `setCollateralVault` |
| high (economic / role change) | ≈ 30 | `setFee*`, `set*Engine`, `set*Account`, `setExecutor`, `setRebate*`, `setLaunchActiveCollateral` |
| medium (parameter tuning) | ≈ 12 | caps, liquidation params, restriction modes |
| low (defensive pause) | 14 | every `pause*` (guardian-or-owner) |

---

## 2. Governance action matrix

For each owner-controlled action, classify the *governance
authority* expected to perform it in production.

| Class | Description | Authority | Latency |
|---|---|---|---|
| **Multisig immediate** | Defensive actions, pauses, emergency disables | guardian multisig (Ops 2-of-3) | seconds |
| **Timelock 24h** | Recipient / setter changes; economic policy tweaks | Timelock with 24h delay; proposer = governance multisig (3-of-5) | 24h |
| **Timelock 72h** | Critical surface — `transferOwnership`, `setMerkleRoot`, `setFee*Profile`, `setOracle`, `setMatchingEngine`, `setMarginEngine`, `setUseFeesManagerV2`, `setCollateralVault` | Timelock with 72h delay; proposer = governance multisig | 72h |
| **Breakglass** | Out-of-band emergency override of the timelock | hardware-MFA + 2-person quorum; documented separately | minutes |

### 2.1 Action → authority assignment

| Action | Authority |
|---|---|
| `FeesManagerV2.setMerkleRoot` | **timelock-72h** |
| `FeesManagerV2.setFeeProfile` | **timelock-72h** |
| `FeesManagerV2.setRfqDiscountProfile` | **timelock-72h** |
| `FeesManagerV2.setFeeRecipient` | **timelock-72h** |
| `FeesManagerV2.setRebateFundingAccount` | timelock-24h |
| `FeesManagerV2.setFeeConsumer` | timelock-24h |
| `FeesManagerV2.fundRebateBudget` | timelock-24h |
| `FeesManagerV2.withdrawRebateBudget` | timelock-72h |
| `Margin/PerpEngine.setMatchingEngine` | **timelock-72h** |
| `Margin/PerpEngine.setOracle` | **timelock-72h** |
| `Margin/PerpEngine.setRiskModule` | timelock-24h |
| `Margin/PerpEngine.setFeesManagerV2` | **timelock-72h** |
| `Margin/PerpEngine.setUseFeesManagerV2` | **timelock-72h** |
| `Margin/PerpEngine.set*RiskParams` | timelock-24h |
| `*.pauseTrading / pauseLiquidation / pauseFunding / pauseCollateralOps / pauseDeposits / pauseWithdrawals / pauseInternalTransfers / pauseYieldOps` | **multisig-immediate** (guardian) |
| `*.unpause*` | timelock-24h |
| `*.setGuardian` | **timelock-72h** (changing the pause authority itself) |
| `*.transferOwnership` | **timelock-72h** + cooldown |
| `CollateralVault.setCollateralToken` | **timelock-72h** |
| `CollateralVault.setTokenStrategy` | **timelock-72h** |
| `CollateralVault.setMarginEngine` | **timelock-72h** |
| `CollateralVault.setLaunchActiveCollateral` | timelock-24h |
| `OptionMatchingEngine.setExecutor` | timelock-24h |
| `ProtocolFeeVault.bootstrap` | timelock-24h (one-shot per asset) |
| `ProtocolFeeVault.withdrawRevenue` | timelock-24h |
| `ProtocolFeeVault.pauseRebates` | **multisig-immediate** |
| `ProtocolFeeVault.unpauseRebates` | timelock-24h |
| `setMerkleRoot(0x0, 0, 0)` (emergency revoke) | **breakglass** |
| `setUseFeesManagerV2(false)` (emergency V2 disable) | **breakglass** |

### 2.2 Documentation requirements per class

| Class | Required docs |
|---|---|
| timelock-72h | Pre-broadcast preflight + dry-run + 2-reviewer signoff |
| timelock-24h | Pre-broadcast preflight + 1-reviewer signoff |
| multisig-immediate | Incident runbook + post-action review within 24h |
| breakglass | Quorum signoff + immediate post-action incident report + governance vote ratifying or unwinding within 72h |

---

## 3. Emergency runbooks

Five canonical incident response procedures. Every runbook:
- Is **operator-executable** (cast / forge commands).
- Never depends on a backend running.
- Never depends on monitoring being live.
- Documents the rollback path.

### 3.1 RB-DISABLE-V2-FEES — Disable V2 fee path globally

**When:** FeesManagerV2 misbehaves in a way that requires
immediate detachment from production fee flow.

```bash
# Switch BOTH engines back to V1 fee path.
cast send $NEW_MARGIN_ENGINE 'setUseFeesManagerV2(bool)' false \
  --private-key $TIMELOCK_PK
cast send $NEW_PERP_ENGINE 'setUseFeesManagerV2(bool)' false \
  --private-key $TIMELOCK_PK
```

**Verify:**
- Next fee event after the tx is a V1 `TradingFeeCharged`, NOT V2.
- `deopt_fees_charged_v2_total` counters stop incrementing.

**Rollback:** `setUseFeesManagerV2(true)` per engine, once
FeesManagerV2 is healthy.

---

### 3.2 RB-PAUSE-REBATES — Stop rebate payouts at the funding source

**When:** Rebate budget is being drained anomalously (e.g.
suspicious tier-4 maker activity, off-policy maker rebate
amounts).

```bash
# Option A: zero the funding account (instantaneous):
cast send $FEES_MANAGER_V2 'setRebateFundingAccount(address)' \
  0x0000000000000000000000000000000000000000 \
  --private-key $TIMELOCK_PK

# Option B (post-V2G-R5): pause at the vault layer:
cast send $PROTOCOL_FEE_VAULT 'pauseRebates()' --private-key $GUARDIAN_PK
```

**Verify:**
- Next rebate-consuming tx reverts with
  `RebateFundingAccountUnset` (Option A) or
  `RebatesPausedError` (Option B).
- `deopt_fees_rebated_v2_total` counters stop incrementing.
- `deopt_fee_vault_rebates_paused == 1` (Option B).

**Rollback:**
- Option A: `setRebateFundingAccount(<correct address>)`.
- Option B: `unpauseRebates()`.

---

### 3.3 RB-DISABLE-CONSUMER — Drop a misbehaving engine from FM-V2

**When:** A specific engine is producing malformed V2 fee
events (wrong consumer, wrong amounts, etc.) and must be cut
out without halting the rest of the protocol.

```bash
cast send $FEES_MANAGER_V2 'setFeeConsumer(address,bool)' \
  $MISBEHAVING_ENGINE false \
  --private-key $TIMELOCK_PK
```

**Effect:** `consumeFees` from that engine reverts
`NotFeeConsumer(engine)`. The engine's own fee charge flow
fails atomically; affected trades revert. Other engines are
unaffected.

**Verify:**
- Per-engine `up=1` but `deopt_fees_charged_v2_total{consumer="<engine>"}` stops incrementing.
- `FeeOldConsumer` / `FeeUnknownConsumer` alert resolves
  (since the engine is no longer in the allowlist; events from
  it now silently fail).

**Rollback:** `setFeeConsumer(engine, true)` after triage.

---

### 3.4 RB-ROOT-EXPIRY — Revoke Merkle root + force re-claim

**When:** Tier merkle root is compromised (leaked snapshot, off-policy
leaves, signing-key compromise on the snapshot pipeline).

```bash
# Revoke immediately by setting an invalid window. Any existing
# claimed tier persists in _claimedTiers (V2G-Q invariant); new
# claims revert NoMerkleRoot or TierExpired.
cast send $FEES_MANAGER_V2 \
  'setMerkleRoot(bytes32,uint64,uint64)' \
  0x0000000000000000000000000000000000000000000000000000000000000000 \
  0 0 \
  --private-key $TIMELOCK_PK
```

**Effect:** `merkleRoot == 0` → all subsequent `claimTier` calls revert
`NoMerkleRoot`. Existing claims still resolve via `currentTier`
(V2G-Q `testV2GQ_RootRotationKeepsExistingClaimButInvalidatesOldProofs`).

**Verify:**
- `FeesManagerV2.merkleRoot()` returns `bytes32(0)`.
- `FeesManagerV2.rootValidFrom()` and `rootValidUntil()` return 0.
- Next `claimTier` reverts `NoMerkleRoot`.

**Rollback:** Re-publish a fresh merkle root via
`setMerkleRoot(newRoot, validFrom, validUntil)` once the
snapshot pipeline is re-secured. Users whose tier expired re-claim
with the new proof; previously-claimed tiers remain valid until
their per-leaf `validUntil`.

---

### 3.5 RB-BUDGET-DRAIN — Withdraw rebate budget back to the treasury

**When:** Rebate budget has grown beyond policy + ops wants to
reclaim it.

```bash
cast send $FEES_MANAGER_V2 \
  'withdrawRebateBudget(address,uint256,address)' \
  $ASSET $AMOUNT $TREASURY_RECIPIENT \
  --private-key $TIMELOCK_PK
```

**Effect:** Accounting-only `withdrawRebateBudget`. Does NOT
move underlying CV balance — the funds remain in the
`rebateFundingAccount` internal CV account. The withdraw only
decrements the FM-V2-side `rebateBudget` counter.

To actually move the underlying ERC20:

```bash
# Move from rebateFundingAccount's CV account to treasury EOA.
# This requires the rebateFundingAccount to call
# transferBetweenAccounts (which is onlyMarginEngine on live CV
# today — V2G-R3 extension required to let an internal-account
# holder withdraw directly).
```

**Verify:**
- `deopt_fees_manager_v2_rebate_budget_native{asset=…}` drops by `AMOUNT`.

**Rollback:** `fundRebateBudget(asset, amount)` re-credits the
counter (no underlying move needed if the funds didn't actually
leave the rebateFundingAccount).

---

### 3.6 RB-OLD-CONSUMER-ALERT — `FeeOldConsumer` fires

**When:** Prometheus emits `FeeOldConsumer` alert →
`OLD_PERP_ENGINE` (or any stranded engine) is emitting V2 fee
events. This MUST NEVER fire under normal operation.

**Triage:**

1. **Confirm** the alert is real, not a label-drift artefact:
   ```bash
   curl -s 'http://127.0.0.1:9090/api/v1/query?query=deopt_perp_fee_charged_v2_total{consumer="old"}'
   ```
2. **Inspect** which contract emitted the offending event:
   ```bash
   curl -s -H "X-Admin-Token: $ADMIN_API_TOKEN" \
     "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=<recent tx>"
   ```
3. If the offending contract is the genuine `OLD_PERP_ENGINE`:
   - **Sound the alarm** — this implies a stale env or a
     mis-wired rollback. Page on-call governance.
   - **Disable** the engine as a fee consumer (RB-DISABLE-CONSUMER).
   - **Confirm** the NEW_PERP_ENGINE is still wired
     (`FeesManagerV2.isFeeConsumer(NEW_PERP_ENGINE) == true`).
4. If the offending contract is a previously-unknown engine:
   - Treat as a new contract impersonating the fee path; the
     allow-list refusal already protects FM-V2 (events still emit
     but `consumeFees` would revert).
   - Add to allow-list ONLY after governance review.

**Rollback:** No automatic rollback — this is an
incident-response procedure, not a parameter change.

---

## 4. Owner / guardian / timelock migration plan

The pre-mainnet ownership migration is the V2G-Y close. Today
(testnet) every owner field is the operator EOA. Production
target:

```
Owner (timelock-72h target): ProtocolTimelock
Owner (timelock-24h target): ProtocolTimelock (single contract; delay set per-tx)
Guardian (multisig-immediate): OPS_MULTISIG (Safe 2-of-3)
Breakglass: GOVERNANCE_MULTISIG (Safe 3-of-5 + hardware MFA)
```

### 4.1 Migration order

```
1. Deploy ProtocolTimelock with 72h max delay (already deployed).
2. Deploy OPS_MULTISIG (Safe).
3. Deploy GOVERNANCE_MULTISIG (Safe).
4. For each contract:
   a. setGuardian(OPS_MULTISIG)
   b. transferOwnership(ProtocolTimelock)  ← LAST per contract
5. Test each emergency runbook end-to-end on staging.
6. Hand off operator EOA private keys to a sealed envelope (no live use).
```

**Order rationale:** `setGuardian` happens BEFORE
`transferOwnership` so the operator EOA can still execute the
guardian setter. After `transferOwnership` the operator EOA loses
authority over the contract; only the timelock can `unpause` or
change any setter.

### 4.2 Two-step ownership

Current contracts use single-step `transferOwnership`. V2G-Y
recommends migrating to OZ `Ownable2Step` to avoid accidental
transfer to a wrong / unreachable address. This is a contract
upgrade decision; carry to V2G-AUDIT for review.

---

## 5. Acceptance criteria for V2G-Y close

- [ ] All owner / guardian functions across the V2 contract suite catalogued (§1).
- [ ] Each function classified into one of the 4 governance authority classes (§2).
- [ ] Each emergency action has a runbook (§3) with cast commands + verify + rollback.
- [ ] Ownership migration plan drafted (§4) — execution gated by audit.
- [ ] No live ownership transfer happened.

---

## 6. Cross-links

- Threat model: `docs/ADMIN_AUTH_RBAC_THREAT_MODEL_V2G_V.md`
- W2 route gate: `docs/ADMIN_RBAC_ROUTE_ENFORCEMENT_V2G_W2.md`
- Canonical fee audit pack: `docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`
- V2G-PX RFQ runbook: `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`
- V2G-RX vault runbook: `docs/PROTOCOL_FEE_VAULT_INTEGRATION_RUNBOOK_V2G_RX.md`
- V2G-K alerts runbook: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`

## 7. Hard-gate compliance

This doc broadcasts nothing. All `cast send` examples assume the
human operator at the timelock surface. No ownership transfer was
executed; no guardian was changed; no rebate budget was modified.
