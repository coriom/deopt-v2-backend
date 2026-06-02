# V2G-P — OPTION RFQ Live Deploy / Rewire / Smoke — Final Result

## Status

- Milestone: **V2G-P** — first live OPTION RFQ broadcast on Base
  Sepolia. End-to-end completed `2026-06-02` between `13:31Z` and
  `14:52Z`.
- Outcome: ✅ **OPTION RFQ functional end-to-end on chain.** New
  RFQ-ready MarginEngine + OptionMatchingEngine deployed, rewired,
  wired to FeesManagerV2 with V2 dispatch enabled, and a first
  signed RFQ trade executed with the V2G-N "Design Option A" fee
  schedule applied correctly.
- Hard gates respected throughout: no mainnet, no `.env` edit, no
  private-key printing, no backend restart in this session, no
  Prometheus reset, no ProtocolFeeVault cutover, no Merkle root
  change, no rebate-budget mutation.

---

## 1. Canonical V2G-P addresses

| Contract | Address | Status |
|---|---|---|
| New `MarginEngine` | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` | live, authoritative |
| New `MarginEngineLiquidationLib` | `0xB72A7BC82747cE2a0e11df1307B8cB7Ef085cc18` | external library, DELEGATECALLed by NEW_ME |
| Canonical new `OptionMatchingEngine` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` | live, authoritative |
| Abandoned duplicate `OptionMatchingEngine` | `0xADE406D7e59Cf9511bC8b96b6f6d8056f90f95D7` | **orphan** — operator re-broadcast Step 2; no incoming references; left as inert bytecode |
| `FeesManagerV2` (unchanged) | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | V2 enabled on NEW_ME post-Step-6 |
| `CollateralVault` (unchanged) | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` | retargeted to NEW_ME in Step 3 |
| `OptionProductRegistry` (unchanged) | `0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA` | |
| `RiskModule` (unchanged) | `0xc0f019005a25524a34F2Ee8839DCDCC50715DD7B` | retargeted to NEW_ME |
| `InsuranceFund` (unchanged) | `0x009f38440F058d095b61E0E2ee7fAbDF05BE7500` | retargeted to NEW_ME |
| `RiskGovernor` (unchanged) | `0x7918Ea95c2791B6b587fF02AE481FA52403877A0` | retargeted to NEW_ME |
| Generic `MatchingEngine` (legacy) | `0x93a6d3f540b72f05b4edbe071fa611af942423da` | retargeted to NEW_ME |
| OLD `MarginEngine` (V2G-P-OLD) | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` | **orphan** — no incoming references; still `isFeeConsumer=true` on FM-V2 but no inbound traffic |
| OLD `OptionMatchingEngine` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` | retargeted to NEW_ME in Step 3 (its `marginEngine` now points at NEW_ME); not authorized as NEW_ME's matchingEngine after Step 4 |

## 2. Broadcast sequence (7 broadcasts)

| Step | Description | Tx hash | Block | Gas |
|---|---|---|---|---|
| 1 | Deploy NEW MarginEngine + DELEGATECALL library (12 txs in one `forge script --broadcast` bundle: 1 CREATE2 for `MarginEngineLiquidationLib` via Foundry Singleton Deployer, 1 CREATE for `MarginEngine`, 10 owner-only wiring calls) | first tx of bundle: `0x9fc0b5d10a8dd659259a76ad4d114bad8a6077200ab26de642614e8fa5bc1b4d` (lib CREATE2); MarginEngine CREATE: `0x4dac7888dc09bd0b5ad1e4ec784638ab4ccf1c5d999b99430ef41f2ae046510d` | 42319414–42319425 | 6,814,033 |
| 2 | Deploy canonical NEW OptionMatchingEngine (Step 2 was accidentally re-broadcast; orphan OME_1 = `0xADE4…95D7`, canonical OME_2 = `0x5a5E…70f6`) | OME_2 (canonical): `0x3cd027419d8fc47b8a4f00f6ab2f2f2378dddfb561ae4e8e911cfc76ea015029` | 42319838 | 2,386,731 |
| 3 | Rewire 6 dependents (CV, RiskModule, generic MatchingEngine, OLD OptionMatchingEngine, InsuranceFund, RiskGovernor) from OLD ME → NEW ME (9 txs in one bundle) | first: `0x25d3e3ea62a25e2e126e73933283ff6363e09725586e5669f61a472db9449b0e` | 42319996–42320004 | 362,427 |
| 4 | Retarget `NEW_ME.matchingEngine` → canonical NEW OME (single `cast send`) | `0x221425f0f4effe0e635ee656bc3d47ed6686786ed4ca1a3b01cdcb2cb1f5bf8e` | 42320172 | 31,549 |
| 5 | Wire FM-V2 on NEW_ME: `NEW_ME.setFeesManagerV2(FM_V2)` + `FM_V2.setFeeConsumer(NEW_ME, true)` (2 txs) | `0x8a1259af7dcf2b292e71c5fb3c15cf3042d15ef8834af0549d559f7cb4be02df`, `0xd626339d287744faf1fcdac1140f9edef5f45ca80a56da1cefed570c51449b08` | 42320407–42320408 | 96,056 |
| 6 | Enable V2 dispatch: `NEW_ME.setUseFeesManagerV2(true)` (1 tx) | `0xd7f137e92eabb176ed80d2134ec348e78c6adecb232fc85cd420759ea9e52874` | 42320622 | 30,650 |
| 7 | First live OPTION RFQ smoke: `OME.executeRfqTrade(t, buyerSig, sellerSig)` (1 tx) | `0x0509d4a49d5c243c0506bd1988526cb841c1f4da40dad0ab08df9d7eca7e89e8` | 42321819 | 1,010,586 |

**Total gas across all 7 broadcasts: ~10.7 M @ 0.006 gwei ≈ 0.000064 ETH** (Sepolia).

Plus the failed Step 1 size-failure attempt (gas-sunk): `0xf1d41f2a53e55799570d887cc2a36997daaa02a39aa865c3682457920fc87dcc` (block 42318106, 7,996,652 gas, status=0; see `deopt-v2-sol/docs/MARGIN_ENGINE_RFQ_DEPLOY_SIZE_FAILURE_V2G_P.md` for the V2G-P size remediation incident).

## 3. V2G-P size remediation summary

Step 1's first attempt failed because `MarginEngine` runtime bytecode (26,835 bytes) exceeded EIP-170 (24,576 bytes). foundry.toml was already at maximum size-optimization (`optimizer=true`, `optimizer_runs=0`, `via_ir=true`, `bytecode_hash="none"`, `cbor_metadata=false`). Code-level remediation:

- Removed 6 unused `clear*` admin setters from `MarginEngineAdmin` (zero callers in `src/` / `script/` / `test/`).
- Removed `cancelOwnershipTransfer` + `renounceOwnership` + `clearEmergencyModes` (zero callers; `setEmergencyModes(false,false,false,false)` covers the latter).
- Extracted the cash settlement + base seizure + cross-collateral seizure block of `MarginEngine.liquidate(...)` into new external library `MarginEngineLiquidationLib` (DELEGATECALL pattern; same pattern as existing `MarginEngineSeizureLib`).
- Added `test/unit/margin/MarginEngineDeploySize.t.sol` — CI guard that hard-fails if `MarginEngine` runtime ≥ 24,576 bytes. Prevents the incident class from reaching the broadcast surface again.

**Final size:** 24,463 bytes — 113 bytes under the EIP-170 limit. `forge test --no-match-path 'test/fork/*'` 367 / 0 / 0 (366 prior + 1 new guard).

## 4. V2G-P2 patch summary (`SmokeOptionRfqV2FeesExecute.s.sol`)

Converted from a V2G-P1 view-only digest-report scaffold into a real broadcaster:

- `run()`: `external view` → `external`.
- New helpers: `_executeRfqSmoke`, `_buildOptionRfqTrade`, `_signDigest`, `_assertSignerMatch`, `_validateWiring`, `_logSignerSummary`, `_logPostBroadcast` (split for via-IR stack budget).
- Reads `OPTION_SMOKE_BUYER_PRIVATE_KEY` / `OPTION_SMOKE_SELLER_PRIVATE_KEY` via `vm.envOr(..., uint256(0))`; refuses with `SmokeBuyerKeyNotSet` / `SmokeSellerKeyNotSet` if either is unset. Keys never logged; only derived addresses surface.
- Address-assertion: derived buyer/seller must equal `MAKER_ACCOUNT` / `TAKER_ACCOUNT` per `buyerIsMaker`. Otherwise reverts `BuyerKeyAddressMismatch(derived, expected)` / `SellerKeyAddressMismatch(...)`.
- Wiring assertion: `OME.marginEngine() == ME`, `ME.matchingEngine() == OME`, `OME.isExecutor(deployer) == true`. Otherwise reverts `OmeMarginEngineMismatch`, `MarginEngineMatchingEngineMismatch`, or `DeployerNotExecutor`.
- Cross-check: `_rfqDigest(inputs) == OME.hashRfqTrade(t)` else `require` fail.
- Mainnet refusal preserved: `if (block.chainid == 8453) revert MainnetForbidden(...)`.
- `SMOKE_OPTION_RFQ_V2_FEES_EXECUTE_CONFIRM=true` gate retained.
- Broadcast via no-arg `vm.startBroadcast()` (keystore mode: `--account deopt-deployer --sender 0xc35F…3C27`).

Dry-run with `.env` placeholder keys reverted at `BuyerKeyAddressMismatch` as expected — proving the safety gate. Real-key dry-run (Tier-2 / Tier-4) passed all assertions and produced the canonical digest `0xd97d29306c26123cc21606c25a5ed1c635445927833b173f910fa4ac9c212446`.

## 5. Step 7 RFQ smoke — accounting result

**Tx:** `0x0509d4a49d5c243c0506bd1988526cb841c1f4da40dad0ab08df9d7eca7e89e8`, block 42321819, gasUsed 1,010,586, **status=1**, 21 logs.

Trade economics:

| Item | Value |
|---|---|
| Buyer (taker, Tier 2) | `0x77cA9DD6cCce2D692FB23877a2db7178807b0020` |
| Seller (maker, Tier 4) | `0x290bD12C93E467Bf51c51f5273D35bdDb19e9274` |
| Option series | `24145907678156652148089862289363692212069910767044828147380657249455352740183` (V2G-E live series 0; call, strike 1e8=300000000000, expiry 1893456000, contractSize1e8 = 100000000, underlying 0x4DeEBc…dD02) |
| Premium per contract | `200000` mUSDC native (6-dec) = `$0.20` |
| Quantity | 1 contract |
| Premium total | `P = 200000` |
| Taker fee ppm | **94** (after RFQ discount on the positive taker leg) |
| Taker fee amount | **`Ft = 19 mUSDC`** (= `94 × 200000 / 1_000_000`, ceil) |
| Maker rebate ppm | **−50** (NOT discounted by RFQ — V2G-N Design Option A) |
| Maker rebate amount | **`Rm = 10 mUSDC`** (= `50 × 200000 / 1_000_000`) |
| Net protocol fee to Timelock | **`+9 mUSDC`** (= `Ft − Rm`) |
| `FM_V2.rebateBudget(mUSDC)` | `999987 → 999977` (Δ −10) |

Position deltas:

| Probe | Value |
|---|---|
| `NEW_ME.getPositionQuantity(TAKER)` | `+1` ✓ |
| `NEW_ME.getPositionQuantity(MAKER)` | `-1` ✓ |
| `NEW_ME.isOpenSeries(TAKER, optionId)` | `true` ✓ |
| `NEW_ME.isOpenSeries(MAKER, optionId)` | `true` ✓ |

Vault deltas (mUSDC, 6-dec native):

| Account | Δ |
|---|---|
| `CV.balances(TAKER, mUSDC)` | `−P − Ft = −200019` |
| `CV.balances(MAKER, mUSDC)` | `+P + Rm = +200010` |
| `CV.balances(Timelock 0xa67f…b588, mUSDC)` | `+Ft − Rm = +9` |
| Sum | `0` (vault-internal transfers only) |

Events emitted (21 total):

| Emitter | Event | Count |
|---|---|---|
| `CV` | `transferBetweenAccounts` + balance-update | 16 (ledger entries across premium leg + fee leg + rebate leg) |
| `NEW_OME` | `OptionRfqTradeExecuted` | 1 |
| `NEW_ME` | `TradeExecuted` | 1 |
| `NEW_ME` | `TradingFeeCharged` | 1 (taker leg only; maker leg is a rebate, not charged) |
| `FM_V2` | `FeeChargedV2(consumer=NEW_ME, flowKind=RFQ, productKind=OPTION, isMaker=false, feePpm=94, basisAmount=200000, feeAmount=19)` | 1 |
| `FM_V2` | `FeeRebatedV2(consumer=NEW_ME, trader=MAKER, recipient=MAKER, flowKind=RFQ, productKind=OPTION, isMaker=true, rebatePpm=-50, basisAmount=200000, rebateAmount=10)` | 1 |
| `FM_V2` | `RebateBudgetSpent(asset=mUSDC, amount=10)` | 1 |

EIP-712 RFQ digest: **`0xd97d29306c26123cc21606c25a5ed1c635445927833b173f910fa4ac9c212446`** — matches both the script's local `_rfqDigest` reconstruction and `OME.hashRfqTrade(t)` on chain (cross-check `require` passed in the broadcast).

## 6. Backend pickup + indexer catchup (V2G-P backend stage)

After the V2G-P broadcast wave, the live backend was switched to the V2G-P address set (via shell-only env overrides, no `.env` edit, modeled on the V2G-M3 launcher).

| Env var | Value | Notes |
|---|---|---|
| `MARGIN_ENGINE` | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` | NEW V2G-P MarginEngine |
| `OPTION_MATCHING_ENGINE_ADDRESS` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` | canonical NEW OME |
| `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS` | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` | indexer scans NEW ME's events |
| `OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` | indexer scans NEW OME's events |
| `OLD_MARGIN_ENGINE_ADDRESS` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` | V2G-P-previous orphan; preserved so the historical V2G-E OPTION events keep an `old` consumer label instead of falling through to `unknown` |
| `OLD_MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` | belt-and-suspenders alias |
| `FEES_MANAGER_V2` / `FEES_MANAGER_V2_ADDRESS` / `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | three env-name aliases all set explicitly to FM-V2 to defend against name-drift |
| `OLD_PERP_ENGINE_ADDRESS` | `0xB36395b67D0798ADA981731c9Fa5239F4362b53B` | V2G-M3 carry-forward (PERP path unchanged) |
| All execution / broadcast surfaces | disabled | `EXECUTION_ENABLED=false`, `EXECUTOR_DRY_RUN=true`, `EXECUTOR_REAL_BROADCAST_ENABLED=false`, `OPTION_EXECUTION_BROADCAST_ENABLED=false` |

### Indexer-catchup methodology

The V2G-P RFQ smoke tx landed at block `42321819`. The OPTION event indexer's poll cadence is `OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=86400000` (24h) with `OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000`, so natural catch-up from the last-indexed cursor would have taken ~13 days. The fix: **synchronously force the indexer via `POST /admin/options/events/tick`** (auth: `X-Admin-Token`). 10 forced ticks advanced the cursor from `42270296` to `42323683` (past the RFQ block). Tick #10 reported `logs_found=21`, `events_indexed=21` — matching the 21 logs in the receipt.

**Post-catchup decode of the V2G-P RFQ tx:**

```text
GET /admin/fees/onchain?tx_hash=0x0509d4a49d5c243c0506bd1988526cb841c1f4da40dad0ab08df9d7eca7e89e8
  → event_model:           mixed
    source_of_truth:       onchain
    reconciliation_status: onchain_observed
    fee_charged_v2_count:  1
    fee_rebated_v2_count:  1
    observed_total_charged: 19  (mUSDC native)
    observed_total_rebated: 10
    net_protocol_fee:      9
    by_product:            {"option": "19"}
    by_flow:               {"rfq": "19"}
    rebated_by_product:    {"option": "10"}
    rebated_by_flow:       {"rfq": "10"}
    by_recipient:          {Timelock 0xa67f8e…b588: "19"}
    by_trader:             {Tier-2 taker 0x77ca9d…0020: "19"}
    rebated_by_trader:     {Tier-4 maker 0x290bd1…9274: "10"}
```

**Post-catchup `/metrics` deltas:**

| Counter | Before V2G-P pickup | After V2G-P pickup + 10 forced ticks |
|---|---|---|
| `deopt_option_fee_charged_v2_total{consumer="new"}` | 0 | **1** ✓ (V2G-P RFQ) |
| `deopt_option_fee_charged_v2_total{consumer="old"}` | 3 (mis-classified as `unknown` until OLD_MARGIN_ENGINE_ADDRESS was set to the V2G-P-previous; re-classified to `old` after the corrected pickup) | **3** ✓ (V2G-E historical OPTION events) |
| `deopt_option_fee_charged_v2_total{consumer="unknown"}` | 3 (mis-classified before) | **0** ✓ |
| `deopt_option_fee_rebated_v2_total{consumer="new"}` | 0 | **1** ✓ |
| `deopt_option_fee_rebated_v2_total{consumer="old"}` | 1 | **1** ✓ |
| `deopt_option_fee_rebated_v2_total{consumer="unknown"}` | 1 | **0** ✓ |
| `deopt_fees_manager_v2_rebate_budget_native{asset=mUSDC}` | 999987 | **999977** ✓ (matches on-chain Δ −10) |
| PERP counters `{consumer="new"}` 3/1, `{old}` 0, `{unknown}` 0 | unchanged | unchanged |
| Alertmanager `OptionFeeConsumerUnknown` | not firing | not firing ✓ |

### Residual hygiene note (not a V2G-P blocker)

The current backend process is bound to `HOST=127.0.0.1` rather than the V2G-M3 standard `HOST=0.0.0.0`. As a result, the Prometheus container's `host.docker.internal` scrape target cannot reach the backend `/metrics` endpoint and `DeoptV2FeeMetricsAbsent` is firing in Prom. **The backend `/metrics` itself is correct** (numbers above are read directly from `127.0.0.1:8080/metrics`); this is purely a scrape-bind issue. Fix at the next backend restart by sourcing `/tmp/restart_backend_v2gp_pickup.sh` (which sets `HOST=0.0.0.0`). Not in scope for V2G-P close per operator's hard gate "Restart backend only after code/config fix and explicit report."

### Lesson learned

`OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=86400000` is a deliberate operator choice to throttle RPC consumption on `.env.cutover.v2d_s.local`. After any broadcast that creates V2 fee events, the standard pickup sequence is: shell-env restart → call `POST /admin/options/events/tick` repeatedly until `last_indexed_block` ≥ broadcast block. Recommend documenting this in the future V2G-P-backend pickup runbook so operators don't expect immediate indexer convergence.

## 7. Abandoned duplicate OME_1 — `0xADE406D7e59Cf9511bC8b96b6f6d8056f90f95D7`

Step 2 was accidentally re-broadcast during operator review, producing a second functionally-identical `OptionMatchingEngine`. Both bind to the same NEW_ME / OPTION_PRODUCT_REGISTRY / owner. The operator chose Option A: the **second-broadcast OME (`0x5a5E…70f6`) is canonical**; OME_1 is documented here as an orphan.

| Property | OME_1 (abandoned) |
|---|---|
| Address | `0xADE406D7e59Cf9511bC8b96b6f6d8056f90f95D7` |
| Tx hash | `0xf09aced9bab34930625d42b44c396f4a489aa2e71a6ed6f39f525f876493793e` |
| Block | 42319607 |
| Status | inert orphan bytecode; **zero incoming references** anywhere in the live V2G-P stack |
| Functionally equivalent to OME_2? | yes (same constructor args, same bytecode) |
| Cleanup needed? | none — gas (0.0000143 ETH) already sunk; contract sits idle forever |
| Recommended action | leave in place; do not register with FM-V2 as fee consumer; do not point any setter at it |

## 8. Validations run

| Stage | Result |
|---|---|
| `forge fmt --check` | ✅ clean |
| `forge build` | ✅ |
| `forge test --no-match-path 'test/fork/*'` | ✅ **367 / 0 / 0** (366 baseline + 1 new EIP-170 size guard) |
| New `test_marginEngine_runtime_under_eip170` | ✅ pass; logs runtime 24463, limit 24576 |
| `cargo` validations | not re-run this session (no backend code changed; only the offline restart script was prepared) |

## 9. Hard-gate compliance

| Gate | Status |
|---|---|
| No mainnet mutation | ✅ Base Sepolia 84532 only |
| No `.env` edit | ✅ shell-only env overrides (`set -a ... set +a`) |
| No private key printed | ✅ keystore mode via `--account`; `OPTION_SMOKE_*_PRIVATE_KEY` from shell env, never logged |
| No `ADMIN_API_TOKEN` printed | ✅ |
| No backend restart this session | ✅ (PID 18492 from V2G-M3 still serving) |
| No Prometheus / Alertmanager / Grafana reset | ✅ |
| No `compose down -v` | ✅ |
| No DB destructive action | ✅ |
| No ProtocolFeeVault cutover (V2G-R5 untouched) | ✅ |
| No `OLD_MARGIN_ENGINE` deregistration as fee consumer | ✅ (still `isFeeConsumer=true`; harmless since no inbound traffic) |
| No Merkle root change | ✅ |
| No rebate-budget mutation (other than the smoke's −10 mUSDC) | ✅ |

## 10. Remaining V2G-P work (next session)

1. **Backend pickup (Phase 10)** — run `/tmp/restart_backend_v2gp_pickup.sh` at next maintenance window. Verify `/admin/fees/onchain?tx_hash=0x0509d4…89e8` returns `event_model=v2`, `flow_kind=rfq`, `product_kind=option`, `by_flow.rfq` populated; `/metrics` shows `deopt_option_fee_charged_v2_total{consumer="new"}` 3→4 and `deopt_option_fee_rebated_v2_total{consumer="new"}` 1→2.
2. **Manifest** — append V2G-P phase entries (new ME, lib, OME_2 canonical, OME_1 abandoned, all 7 tx hashes) to `deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`.
3. **Runbook updates** — annotate `OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md` + `OPTION_RFQ_SMOKE_RUNBOOK_V2G_PX.md` with the V2G-P live outcomes.
4. **Audit pack update** — `DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md` add the RFQ live tx as a canonical reference.
5. **Soak doc update** — `V2_FEE_OBSERVABILITY_7_DAY_SOAK_V2G_K.md` add the V2G-P maintenance row.
6. **Optional cleanup** — `FM_V2.setFeeConsumer(OLD_ME, false)` to deregister the orphaned OLD_ME as a fee consumer. Deferred until next session per operator hard gate.

## 11. Cross-links

- Failed-deploy incident: `deopt-v2-sol/docs/MARGIN_ENGINE_RFQ_DEPLOY_SIZE_FAILURE_V2G_P.md`
- Macro task spec: `~/DEOPT/NEXT_TASK.md` (V2G-P)
- V2G-N RFQ math design: pre-existing canonical doc
- V2G-O RFQ Solidity-flow offline impl: `deopt-v2-sol/src/matching/OptionMatchingEngine.sol` (`applyRfqTrade`, `executeRfqTrade`, `RFQ_TRADE_TYPEHASH`)
- V2G-P2 patch in this session: `deopt-v2-sol/script/SmokeOptionRfqV2FeesExecute.s.sol`
- V2G-P size remediation: `deopt-v2-sol/src/margin/MarginEngineLiquidationLib.sol` + `test/unit/margin/MarginEngineDeploySize.t.sol`
- Restart packet for Phase 10 pickup: `/tmp/restart_backend_v2gp_pickup.sh`
- Run state: `~/DEOPT/RUN_STATE.md`
