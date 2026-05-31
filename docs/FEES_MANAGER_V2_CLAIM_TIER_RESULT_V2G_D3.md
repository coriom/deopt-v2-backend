# V2G-D3 — FeesManagerV2 claimTier Broadcasts + Smoke Preflight PASSED

## Status

- Milestone: **V2G-D3** (closes V2G-D / V2G-D2 / V2G-C rotation).
- Date: 2026-05-30.
- Mode: human-gated Base Sepolia claims, fully verified. **4 EOA-signed
  broadcasts** total: 2 ETH funding sends from DEPLOYER, 2
  self-signed `claimTier` calls from the fresh Tier 4 maker and
  Tier 2 taker. No `.env` mutation, no DB-row mutation, no
  deployed-contract change, no rebate-budget mutation, no rebate
  trades.
- Outcome: `currentTier(Tier4 maker) = 4`, `currentTier(Tier2 taker)
  = 2`; **both `SmokePerpV2Rebate` and `SmokeOptionV2Rebate`
  preflights flip from `MakerHasNoNegativeRebateTier(0, 50)` to
  `PASSED`**. V2G-E (live PERP + OPTION rebate trades through the
  V2D-V / V2E-G executor pattern) becomes unlocked.

## Broadcasts (in order)

| # | Action | Signer | Target | Tx | Block | Gas | Status |
|---|---|---|---|---|---|---|---|
| 1 | `setMerkleRoot(V2G-D2 root, 1780099200, 1781913600)` | DEPLOYER `0xc35F…3C27` | FeesManagerV2 | `0x2700b2a6442d7f9a909ef93eb4f9be97aa1c446c6974e977f6f2193eabff8c99` | 42194852 | 36 114 | ✅ |
| 2 | `cast send 0.001ether → 0x290b…9274` | DEPLOYER | EOA (Tier 4 maker) | (operator-recorded) | — | 21 000 | ✅ |
| 3 | `cast send 0.001ether → 0x77cA…0020` | DEPLOYER | EOA (Tier 2 taker) | (operator-recorded) | — | 21 000 | ✅ |
| 4 | `claimTier(Tier 4 …)` | Tier 4 maker `0x290b…9274` | FeesManagerV2 | `0x07ad52691ac4c396fe6e7a12d2cbb9734c4acbf5d41dd91db8ec6490f713b1d0` | 42195066 | 53 917 | ✅ |
| 5 | `claimTier(Tier 2 …)` | Tier 2 taker `0x77cA…0020` | FeesManagerV2 | `0x224d0567dce62e5fe99e48d32f70e9cc61d1500b519f272ac450add9c9d85fd0` | 42195235 | 53 908 | ✅ |

Total Base Sepolia ETH spent across V2G-D2 + V2G-D3: ≈ 2 000 666 µETH
(0.002 ETH funding + 36 114 + 53 917 + 53 908 + 2×21 000 gas at ~0.006 gwei).

## Per-claim event verification

Both `claimTier` txs emit exactly one log against `FeesManagerV2`:

```
topic0 = 0x1aa53d859aef18ddaaab24085cab40738bdb89de39fcebe844072d11069b1945
       = keccak("TierClaimed(address,uint8,uint64)")
topic1 = indexed account (32-byte left-padded EOA)
data   = abi.encode(tier:uint8, validUntil:uint64)
```

- Tier 4 claim: `topic1 = 0x…290bd12c93e467bf51c51f5273d35bddb19e9274`,
  `data = 0x00…04 || 0x00…6a35d800` (`tier=4`, `validUntil=1781913600`).
- Tier 2 claim: `topic1 = 0x…77ca9dd6ccce2d692fb23877a2db7178807b0020`,
  `data = 0x00…02 || 0x00…6a35d800` (`tier=2`, `validUntil=1781913600`).

The `tx.from` of each claim is the claimant EOA itself
(self-signed; `FeesManagerV2.claimTier` enforces `msg.sender ==
account` at L189 of `src/fees/FeesManagerV2.sol`).

## Post-V2G-D3 live state

| Field | Value | Status |
|---|---|---|
| `FeesManagerV2.merkleRoot()` | `0xd8a627d7a9b600370e6f490fdd789150d7f9c4ea2f09752c88121d1f758fc2df` | V2G-D2 (unchanged) |
| `FeesManagerV2.rootValidFrom()` | `1780099200` (2026-05-30 00:00 UTC) | V2G-D2 (unchanged) |
| `FeesManagerV2.rootValidUntil()` | `1781913600` (2026-06-20 00:00 UTC) | V2G-D2 (unchanged) |
| `FeesManagerV2.rebateBudget(mUSDC)` | `1_000_000` | V2G-C **preserved** |
| `mUSDC.balanceOf(0xa67f…b588)` | `1_000_000` | V2G-C **preserved** |
| `FeesManagerV2.rebateFundingAccount()` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` | unchanged |
| `FeesManagerV2.feeRecipient()` | `0xa67f…b588` | unchanged |
| `FeesManagerV2.owner()` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | unchanged |
| `FeesManagerV2.currentTier(0x290b…9274)` | `4` | **claimed** |
| `FeesManagerV2.currentTier(0x77cA…0020)` | `2` | **claimed** |
| `FeesManagerV2.claimedTiers(0x290b…9274)` | `(4, 1781913600)` | persisted |
| `FeesManagerV2.claimedTiers(0x77cA…0020)` | `(2, 1781913600)` | persisted |
| `FeesManagerV2.isFeeConsumer(0xc6C5…141c)` (NEW PerpV2) | `true` | unchanged |
| `FeesManagerV2.isFeeConsumer(0x287C…48Cc)` (NEW MarginV2) | `true` | unchanged |
| `FeesManagerV2.isFeeConsumer(0xB363…b53B)` (OLD PerpV1) | `false` | **stranded** |
| `FeesManagerV2.getFeeProfile(4, OPTION)` | `(-50, 75)` | Tier 4 OPTION makerPpm is negative |
| `FeesManagerV2.getFeeProfile(4, PERP)` | `(-100, 150)` | Tier 4 PERP makerPpm is negative |
| `balance(0x290b…9274)` | `999 667 142 605 738` wei (~0.000999667 ETH) | Tier 4 EOA can keep operating |
| `nonce(0x290b…9274)` | `1` | one tx sent |
| `balance(0x77cA…0020)` | `999 667 912 220 361` wei (~0.000999667 ETH) | Tier 2 EOA can keep operating |
| `nonce(0x77cA…0020)` | `1` | one tx sent |

## SmokePerpV2Rebate dry-run result — **PASSED**

```
$ FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
  PERP_ENGINE=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c \
  REBATE_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
  MAKER_ACCOUNT=0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 \
  TAKER_ACCOUNT=0x77cA9DD6cCce2D692FB23877a2db7178807b0020 \
  MIN_REBATE_BUDGET=1000000 \
  forge script script/SmokePerpV2Rebate.s.sol:SmokePerpV2Rebate --rpc-url "$RPC_URL"

V2G-B PERP rebate smoke preflight
  PERP_ENGINE (NEW)              = 0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
  OLD_PERP_ENGINE (stranded)     = 0xB36395b67D0798ADA981731c9Fa5239F4362b53B
  MAKER_ACCOUNT                  = 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274
  TAKER_ACCOUNT                  = 0x77cA9DD6cCce2D692FB23877a2db7178807b0020

State snapshot:
  PerpEngine.useFeesManagerV2()  = true
  PerpEngine.feesManagerV2()     = 0x00dA0B9876…  (FEES_MANAGER_V2)
  OldPerpEngine.useFeesManagerV2() = false        (stranded)
  FeesManagerV2.isFeeConsumer(NEW) = true
  FeesManagerV2.isFeeConsumer(OLD) = false
  FeesManagerV2.rebateBudget(mUSDC) = 1_000_000
  FeesManagerV2.merkleRoot()     = 0xd8a627d7…fc2df   (V2G-D2)
  FeesManagerV2.currentTier(MAKER) = 4               (was 0 in V2G-C)
  FeesManagerV2.currentTier(TAKER) = 2               (was 0 in V2G-C)
  PERP makerPpm at MAKER tier   = -100              (was +50 at Tier 0)

V2G-B PERP smoke preflight PASSED. Next step: …
```

`MakerHasNoNegativeRebateTier(0, 50)` is no longer raised. The
single remaining precondition is a tiny PERP trade through the
V2D-V backend executor (V2G-E gate).

## SmokeOptionV2Rebate dry-run result — **PASSED**

```
$ FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
  MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc \
  REBATE_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
  MAKER_ACCOUNT=0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 \
  TAKER_ACCOUNT=0x77cA9DD6cCce2D692FB23877a2db7178807b0020 \
  MIN_REBATE_BUDGET=1000000 \
  forge script script/SmokeOptionV2Rebate.s.sol:SmokeOptionV2Rebate --rpc-url "$RPC_URL"

V2G-B OPTION rebate smoke preflight
State snapshot:
  MarginEngine.useFeesManagerV2()  = true
  MarginEngine.feesManagerV2()     = 0x00dA0B9876…  (FEES_MANAGER_V2)
  FeesManagerV2.isFeeConsumer(MarginEngine) = true
  FeesManagerV2.rebateBudget(mUSDC) = 1_000_000
  FeesManagerV2.merkleRoot()       = 0xd8a627d7…fc2df   (V2G-D2)
  FeesManagerV2.currentTier(MAKER) = 4
  FeesManagerV2.currentTier(TAKER) = 2
  OPTION makerPpm at MAKER tier    = -50               (was +50 at Tier 0)

V2G-B OPTION smoke preflight PASSED. Next step: …
```

The single remaining precondition is a tiny OPTION trade through
the V2E-G backend executor (V2G-E gate).

## V2G-D3 invariance proofs (re-asserted from live RPC)

After both claims landed, the following fields were re-read against
the live chain in a single batch with the smoke dry-runs:

- `merkleRoot()` = `0xd8a627d7…fc2df` (V2G-D2, unchanged by claims).
- `rebateBudget(mUSDC)` = `1_000_000` (unchanged by claims).
- `mUSDC.balanceOf(0xa67f…b588)` = `1_000_000` (no ERC-20 movement).
- `feeRecipient()`, `rebateFundingAccount()`, `owner()` all unchanged.
- `isFeeConsumer(NEW perp)` = `isFeeConsumer(NEW margin)` = `true`;
  `isFeeConsumer(OLD perp)` = `false` (V2F-LM acceptance state
  intact across V2G-D2 + V2G-D3).

`claimTier`'s on-chain effect is purely a write to
`_claimedTiers[account]`; the smoke dry-runs are the operational
proof that this write was sufficient to unblock the rebate path
without any other state change.

## V2G-E handoff — live rebate smoke campaign (next milestone)

V2G-D3 is the last preflight-only step. V2G-E performs **two**
human-gated live trades (not script-broadcast — backend executor
broadcast in the V2D-V / V2E-G pattern):

### Gate V2G-E-1: PERP rebate trade

- Maker: `0x290bD12C93E467Bf51c51f5273D35bdDb19e9274` (Tier 4)
- Taker: `0x77cA9DD6cCce2D692FB23877a2db7178807b0020` (Tier 2)
- Vault precondition: each EOA holds ≥ `1_000_000` mUSDC vault
  balance against `CollateralVault` (use the existing
  `script/PrepareSmokeV1Accounts.s.sol` pattern with these two
  EOAs as buyer/seller).
- Trade: a tiny PERP cross — size and price calibrated so that
  the PERP maker rebate at -100 ppm is ≥ 1 wei mUSDC (the V2F-J2
  trade with `size1e8=1 price1e8=3e11` rounded to 0; V2G-E
  requires sizing such that rebateAmount ≥ 1).
- Expected events:
  - `FeeChargedV2(taker=0x77cA…0020, productKind=PERP, flowKind=ORDERBOOK, ...)`
  - `FeeRebatedV2(maker=0x290b…9274, productKind=PERP, flowKind=ORDERBOOK, ...)`
  - `RebateBudgetSpent(mUSDC, rebateAmount)`
- Expected state delta:
  - `rebateBudget(mUSDC)` decreases by `rebateAmount`.
  - `mUSDC.balanceOf(0x290b…9274)` (or the maker vault) increases
    by `rebateAmount` (per V2D-V settlement path).
  - `mUSDC.balanceOf(0x77cA…0020)` (or the taker vault) decreases
    by `feeAmount`.
- Verification:
  - `GET /admin/fees/onchain?tx_hash=<tx>` (V2D-V / V2E-G admin path).
  - `deopt_perp_fee_charged_v2_total{consumer="new"}` and
    `deopt_perp_fee_rebated_v2_total{consumer="new"}` both advance
    by exactly 1 (V2F-Q metric).
  - OLD-stranded alert remains green (no events on
    `0xB36395…b53B`).

### Gate V2G-E-2: OPTION rebate trade

Same shape with `MarginEngine = 0x287Cef…48Cc` as the execution
path:
- `FeeChargedV2(taker, productKind=OPTION, …)`
- `FeeRebatedV2(maker, productKind=OPTION, …)` at -50 ppm
- `RebateBudgetSpent(mUSDC, rebateAmount)`

### Hard gates V2G-E will need

1. Operator funds each EOA's `CollateralVault` mUSDC balance with
   the PrepareSmokeV1Accounts pattern (mint + deposit). This is a
   DEPLOYER-signed broadcast against `TestnetMockERC20` + the
   collateral vault.
2. The backend executor signs and broadcasts each trade; this repo
   does **not** add a new smoke-trade Solidity script. V2G-E is a
   backend-driven smoke.
3. After each trade, `rebateBudget(mUSDC)` decreases — if it ever
   drops below the per-trade need, `fundRebateBudget(mUSDC, …)`
   must be re-run (V2G-C pattern). For two tiny trades at -100 / -50
   ppm this is far below the 1_000_000 budget; no top-up needed
   for the smoke.

V2G-E does **not** need any further `setMerkleRoot`, `claimTier`,
or `fundRebateBudget` work — the chain is fully prepared.

## Files added / changed (V2G-D3)

Backend:

- `docs/FEES_MANAGER_V2_CLAIM_TIER_RESULT_V2G_D3.md` — this file (**NEW**).
- `docs/FEES_MANAGER_V2_RECOVERY_V2G_D2.md` — appended V2G-D3
  closure note pointing here (see below).

Solidity / `.env` / DB / deployed contracts: **no changes**. V2G-D3
is read-only verification plus two self-signed `claimTier` txs.
No script or test had to be touched.

## Validation commands run

```
cast receipt tier4_claim_tx       ✅ status=1, single TierClaimed log, FeesManagerV2 only
cast receipt tier2_claim_tx       ✅ status=1, single TierClaimed log, FeesManagerV2 only
cast call currentTier(T4)         ✅ 4
cast call currentTier(T2)         ✅ 2
cast call claimedTiers(T4)        ✅ (4, 1781913600)
cast call claimedTiers(T2)        ✅ (2, 1781913600)
cast call merkleRoot()            ✅ 0xd8a627d7…fc2df  (unchanged)
cast call rebateBudget(mUSDC)     ✅ 1_000_000        (unchanged)
cast call balanceOf(funder)       ✅ 1_000_000        (unchanged)
cast call isFeeConsumer(NEW perp/NEW margin/OLD perp) ✅ true/true/false
cast call getFeeProfile(4, OPTION) ✅ (-50, 75)
cast call getFeeProfile(4, PERP)   ✅ (-100, 150)
forge script SmokePerpV2Rebate   --rpc-url $RPC_URL ✅ PASSED  (was MakerHasNoNegativeRebateTier(0,50))
forge script SmokeOptionV2Rebate --rpc-url $RPC_URL ✅ PASSED  (was MakerHasNoNegativeRebateTier(0,50))
```

The V2G-D2 forge / cargo suites (217 forge tests + 661 cargo tests
all green) remain authoritative — V2G-D3 made no source-tree
changes that could regress them.

## Exact blockers carried forward to V2G-E

1. **mUSDC vault balances for both fresh EOAs** ≥ rebate test
   amount. Reuse `script/PrepareSmokeV1Accounts.s.sol` with
   `0x290b…9274` as buyer/maker and `0x77cA…0020` as seller/taker.
2. **Backend executor configured** to sign V2G-E trades using the
   V2D-V (PERP) and V2E-G (OPTION) patterns.
3. **PERP / OPTION trade sizing** such that the rebate at -100 ppm
   / -50 ppm yields a non-zero mUSDC amount after 6-decimal
   rounding (lesson learned from V2F-J2 where rebate rounded to 0).

## Next recommended milestone

**V2G-E — live PERP + OPTION rebate smoke campaign.** Operator funds
vault balances, then triggers two backend-executor-signed trades
(one PERP, one OPTION). Each trade emits the
`FeeChargedV2 + FeeRebatedV2 + RebateBudgetSpent` triple,
decrements `rebateBudget(mUSDC)`, and advances the V2F-Q
`deopt_*_fee_*_v2_total{consumer="new"}` metrics. After V2G-E,
V2G-F (turn the alert / dashboard cutover live and retire the
`merkleRootUnset` operational notice) closes the V2G band.

## V2G-E closure (appended 2026-05-31)

V2G-E completed both legs live on Base Sepolia. The actual
broadcasts used new Solidity scripts (`SmokePerpV2RebateExecute.s.sol`
and `SmokeOptionV2RebateExecute.s.sol`) rather than the backend
executor — the backend's existing `.env` BUYER/SELLER keys are not
the V2G-D2 Tier 4 maker / Tier 2 taker, and the user's hard rule
"do not edit real `.env`" forced the script-driven path. The
Solidity scripts mirror the V2D-V / V2E-G executor pattern in shape
(EIP-712 sign + executeTrade) and the only thing the backend did
was index + decode + serve the admin endpoint.

| Gate | Tx | Result |
|---|---|---|
| V2G-E-1 PERP rebate | `0x5c15e923…aa394` (block 42196090, status 1, gas 842 633) | `FeeChargedV2(taker, OPTION feePpm=200, basis=30 000, fee=6)` + `FeeRebatedV2(maker, rebatePpm=-100, basis=30 000, rebate=3)` + `RebateBudgetSpent(mUSDC, 3)`. Vault deltas: maker +3, taker −6, funder +3. Budget: 1 000 000 → 999 997. |
| V2G-E-2 OPTION rebate | `0x9a85cbce…3149` (block 42206003, status 1, gas 1 046 236) | `FeeChargedV2(taker, OPTION feePpm=125, basis=200 000, fee=25)` + `FeeRebatedV2(maker, rebatePpm=-50, basis=200 000, rebate=10)` + `RebateBudgetSpent(mUSDC, 10)`. Vault deltas (incl. 200 000-native premium leg): maker +200 010, taker −200 025, funder +15. Budget: 999 997 → 999 987. |

V2G-E also exposed a vault-collateral gap for short-call IM: the
ETH-$3000 series demands ~1080 mUSDC native of IM on the writer
side, which the V2G-E Phase-2 funding (2 mUSDC) did not cover. The
first Phase-6 attempt reverted in forge simulation with
`MarginRequirementBreached(seller)` — no broadcast went out. The
recovery (V2G-E Phase 2b) added 1500 mUSDC native to each smoke
EOA via the same `PrepareSmokeV1Accounts.s.sol` pattern; after that
the Phase-6 retry landed cleanly. Full timeline + diagnosis:
`docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md`.

Backend `/admin/fees/onchain` decoded both txs end-to-end (event
counts, basis/fee/rebate amounts, by_recipient/by_side/by_trader/
rebated_by_trader, `event_model = v2` for PERP / `mixed` for OPTION).
V2F-Q PERP metrics correctly report
`deopt_perp_fee_*_v2_total{consumer="new"} = 3 / 1` (charged /
rebated) with `old` and `unknown` arms at zero. OLD-stranded alert
remains green.

V2G-F is now the right next milestone (cut the alerts to live,
retire the merkle-root-unset operational notice, add OPTION-side
V2 metric counters, fix the `.env` PERP_ENGINE_ADDRESS hygiene).
