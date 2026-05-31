# V2G-E — Live PERP + OPTION Rebate Smoke Campaign

## Status

- Milestone: **V2G-E** — drives the first live `FeeRebatedV2`
  emissions on Base Sepolia by trading the V2G-D2 / V2G-D3 Tier 4
  maker (`0x290b…9274`) against the Tier 2 taker (`0x77cA…0020`).
- Mode: agent-prepared, operator-broadcast. Agent never broadcasts,
  never prints private keys, never edits `.env`, never mutates
  Merkle root / rebate budget / fee consumer wiring, never touches
  `OLD_PERP_ENGINE`.
- Date opened: 2026-05-30.

## Phase 1 — Pre-smoke on-chain state (read-only, verified)

| Field | Value | Source |
|---|---|---|
| `FeesManagerV2` | `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` | V2G-C / V2G-D2 |
| `FeesManagerV2.merkleRoot()` | `0xd8a627d7…fc2df` | V2G-D2 (unchanged across D3) |
| `FeesManagerV2.rootValidFrom()` | `1780099200` (2026-05-30 00:00 UTC) | V2G-D2 |
| `FeesManagerV2.rootValidUntil()` | `1781913600` (2026-06-20 00:00 UTC) | V2G-D2 |
| `FeesManagerV2.rebateBudget(mUSDC)` | `1_000_000` (1 mUSDC at 6 dp) | V2G-C, preserved |
| `FeesManagerV2.rebateFundingAccount()` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (Timelock) | unchanged |
| `mUSDC.balanceOf(rebateFundingAccount)` | `1_000_000` | V2G-C, preserved |
| `mUSDC.allowance(funder, FeesManagerV2)` | `0` | exhausted by V2G-C `fundRebateBudget` (expected) |
| `isFeeConsumer(NEW_PERP 0xc6C5…141c)` | `true` | V2F-K |
| `isFeeConsumer(NEW_MARGIN 0x287C…48Cc)` | `true` | V2E-E |
| `isFeeConsumer(OLD_PERP 0xB363…b53B)` | `false` | V2F-LM stranded |
| `NEW_PERP.useFeesManagerV2()` | `true` | V2F-L |
| `NEW_PERP.feesManagerV2()` | `0x00dA0B9876…774f` | V2F-K |
| `NEW_MARGIN.useFeesManagerV2()` | `true` | V2E-E |
| `NEW_MARGIN.feesManagerV2()` | `0x00dA0B9876…774f` | V2E-E |
| `currentTier(Tier4 maker 0x290b…9274)` | `4` | V2G-D3 |
| `currentTier(Tier2 taker 0x77cA…0020)` | `2` | V2G-D3 |
| `getFeeProfile(4, PERP)` | `(makerPpm=-100, takerPpm=150)` | code |
| `getFeeProfile(2, PERP)` | `(makerPpm=-50, takerPpm=200)` | code |
| `getFeeProfile(4, OPTION)` | `(makerPpm=-50, takerPpm=75)` | code |
| `getFeeProfile(2, OPTION)` | `(makerPpm=-10, takerPpm=125)` | code |
| `Vault.balances(maker 0x290b…9274, mUSDC)` | `0` | fresh EOAs |
| `Vault.balances(taker 0x77cA…0020, mUSDC)` | `0` | fresh EOAs |
| `mUSDC.balanceOf(maker)` | `0` | fresh EOAs |
| `mUSDC.balanceOf(taker)` | `0` | fresh EOAs |
| `balance(maker)` ETH | `999_667_142_605_738 wei` (~0.001 ETH) | V2G-D3 dust |
| `balance(taker)` ETH | `999_667_912_220_361 wei` (~0.001 ETH) | V2G-D3 dust |

Every SmokePerpV2Rebate / SmokeOptionV2Rebate gate from V2G-D3 still
returns `PASSED`. The single remaining precondition before the
first rebate trade is **vault collateral for both EOAs**.

## Phase 2 — Vault funding broadcast (human-signed packet)

Existing `script/PrepareSmokeV1Accounts.s.sol` (V2F-J2) reused
as-is. Same single-broadcast, three-signer pattern: deployer mints
mUSDC and dusts ETH; each EOA self-approves the `CollateralVault`
and self-deposits. No contract / Merkle / fee-consumer mutation.

### Preflight (read-only, agent-run)

`PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM=false` preflight against the V2G-D2 EOAs:

```
chainId 84532
deployer (placeholder for preflight only) sanitized
buyer  (PERP_SMOKE_BUYER_PRIVATE_KEY → Tier 4 maker)  0x290bD12C93E467Bf51c51f5273D35bdDb19e9274
seller (PERP_SMOKE_SELLER_PRIVATE_KEY → Tier 2 taker) 0x77cA9DD6cCce2D692FB23877a2db7178807b0020
BASE_COLLATERAL_TOKEN (mUSDC)        0x6eAe407f5640B006faC9965182e238582A3B412E
COLLATERAL_VAULT                     0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE   2_000_000   (2 mUSDC per EOA, both PERP + OPTION room)
PERP_SMOKE_FUND_ETH_WEI              300_000_000_000_000  (0.0003 ETH per EOA, approve + deposit headroom)
State snapshot: before
 mUSDC.owner()                       0xc35F7A8A103A9A4464adfaa76B9B514093D23C27   (deployer == mUSDC owner)
 mUSDC.decimals()                    6
 Vault.launchActiveCollateral(mUSDC) true
 Vault.depositsPaused()              false
 buyer/seller ETH (wei)              999_667_142_605_738  / 999_667_912_220_361
 buyer/seller mUSDC                  0 / 0
 buyer/seller Vault.balances(_, mUSDC) 0 / 0
PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM not set; preflight done, no transactions sent.
```

Preflight passes. All preconditions match the `_validatePreconditions`
checks the script enforces when `CONFIRM=true`.

### Human broadcast packet (Phase 2)

Operator opens a shell with `DEPLOYER_PRIVATE_KEY` (mUSDC owner
`0xc35F7A…3C27`) and `PERP_SMOKE_*` exported from the local
secret files. Then:

```sh
cd ~/DEOPT/deopt-v2-sol

# Operator-only — never echoed, never written to disk
export DEPLOYER_PRIVATE_KEY=<operator supplies>
export PERP_SMOKE_BUYER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)
export PERP_SMOKE_SELLER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)

BASE_COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3 \
PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE=2000000 \
PERP_SMOKE_FUND_ETH_WEI=300000000000000 \
PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM=true \
forge script script/PrepareSmokeV1Accounts.s.sol:PrepareSmokeV1Accounts \
  --rpc-url "$RPC_URL" --broadcast --slow
```

Expected transaction shape (8 broadcasts):

| # | Signer | Target | Function | Notes |
|---|---|---|---|---|
| 1 | DEPLOYER | mUSDC `0x6eAe…412E` | `mint(maker, 2_000_000)` | 2 mUSDC to Tier 4 maker |
| 2 | DEPLOYER | mUSDC | `mint(taker, 2_000_000)` | 2 mUSDC to Tier 2 taker |
| 3 | DEPLOYER | maker EOA | `call{value: 0.0003 ETH}` | gas dust to maker |
| 4 | DEPLOYER | taker EOA | `call{value: 0.0003 ETH}` | gas dust to taker |
| 5 | Tier 4 maker | mUSDC | `approve(VAULT, 2_000_000)` | self-signed |
| 6 | Tier 4 maker | VAULT | `deposit(mUSDC, 2_000_000)` | self-signed |
| 7 | Tier 2 taker | mUSDC | `approve(VAULT, 2_000_000)` | self-signed |
| 8 | Tier 2 taker | VAULT | `deposit(mUSDC, 2_000_000)` | self-signed |

Hard-refused surface (asserted by the script): no PerpEngine /
MarginEngine / MatchingEngine / FeesManagerV2 calls; refuses on
mainnet chain id; refuses if `mUSDC.owner() != deployer` or
`launchActiveCollateral=false` or `depositsPaused=true`.

### Post-broadcast verification commands

```sh
cast call $VAULT 'balances(address,address)(uint256)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 $MUSDC --rpc-url $RPC_URL  # expect 2_000_000
cast call $VAULT 'balances(address,address)(uint256)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 $MUSDC --rpc-url $RPC_URL  # expect 2_000_000
cast call $MUSDC 'balanceOf(address)(uint256)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 --rpc-url $RPC_URL  # expect 0 (minted then deposited)
cast call $MUSDC 'balanceOf(address)(uint256)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 --rpc-url $RPC_URL  # expect 0
cast balance 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 --rpc-url $RPC_URL  # expect ~0.0013 ETH
cast balance 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 --rpc-url $RPC_URL  # expect ~0.0013 ETH
```

Operator should paste back the txs (or the script's terminal output)
so the agent can record block / gas / hash for each leg.

## Phase 2 — Vault funding result (broadcast)

8 broadcasts landed at blocks `42195777 → 42195784` for a total of
`456_992 gas` paid by DEPLOYER at ~0.006 gwei (≈ `0.00000274 ETH`).
Per-leg hashes:

| # | Signer | Action | Tx |
|---|---|---|---|
| 1 | DEPLOYER | mUSDC.mint(maker, 2_000_000) | `0x3cc422b27a66c82498598b35c1a8114e65b693b23c2a2798551aec1ebc668c19` |
| 2 | DEPLOYER | mUSDC.mint(taker, 2_000_000) | `0xc1cbcf882f7a40553882933ad485f27d897e5214da8346d468155d2796f71712` |
| 3 | DEPLOYER | call{value=0.0003 ETH}(maker) | `0xedda9a9d9d6e2fc9b3ec6643004250fba5296c27680835923cc182138c9e27c0` |
| 4 | DEPLOYER | call{value=0.0003 ETH}(taker) | `0x2b278cc4d672e9bfaee50b89b7f7f7f9efc96d3f59333f790cb925a1b758fa7c` |
| 5 | maker (Tier 4) | mUSDC.approve(VAULT, 2_000_000) | `0x515dddff9949441024acf40b50bb30469c462d7c4f681f00955292475f9b83b0` |
| 6 | maker | Vault.deposit(mUSDC, 2_000_000) | `0x6df199667be4bda369a63cc89c074d293dda5ad1f5b50860c9f26150c35ca97d` |
| 7 | taker (Tier 2) | mUSDC.approve(VAULT, 2_000_000) | `0x3688483fe129d2205b7ddf6786c92a477c4206cf057d320d218be471c884f567` |
| 8 | taker | Vault.deposit(mUSDC, 2_000_000) | `0x2b7b4b1b7689a51ce607cecbc6bfbe06f0387ee9ee0b84f479f611d03ddc56c6` |

Post-broadcast on-chain reads (live RPC):

| Field | Pre-V2G-E | Post Phase 2 |
|---|---|---|
| `Vault.balances(0x290b…9274, mUSDC)` | `0` | `2_000_000` |
| `Vault.balances(0x77cA…0020, mUSDC)` | `0` | `2_000_000` |
| `mUSDC.balanceOf(0x290b…9274)` | `0` | `0` (minted then deposited) |
| `mUSDC.balanceOf(0x77cA…0020)` | `0` | `0` |
| `balance(maker)` ETH (wei) | `999_667_142_605_738` | `1_298_729_156_423_370` (+0.0003 ETH dust − approve/deposit gas) |
| `balance(taker)` ETH (wei) | `999_667_912_220_361` | `1_298_729_762_811_989` |
| `nonce(maker)` | `1` | `3` (approve + deposit) |
| `nonce(taker)` | `1` | `3` |
| `FMv2.rebateBudget(mUSDC)` | `1_000_000` | `1_000_000` (unchanged) |
| `FMv2.merkleRoot()` | `0xd8a627d7…fc2df` | unchanged |
| `currentTier(maker)` | `4` | `4` |
| `currentTier(taker)` | `2` | `2` |

`script.s.sol/PrepareSmokeV1Accounts.s.sol`'s internal `_verifyPostState`
asserted the vault deltas exactly match `fundUsdcNative`, so the
deposit landed atomically — no partial state to clean up.

## Phase 3 — PERP rebate smoke dry-run (preflight)

V2G-B's `SmokePerpV2Rebate.s.sol` is read-only and already passes.
For V2G-E we need a script that actually signs and broadcasts the
trade. The V2F-LM `SmokeV2PerpFeesOnNew.s.sol` is structurally what
we want, but it intentionally REFUSES when `rebateBudget != 0` or
`merkleRoot != bytes32(0)`. Touching that defensive script for a
one-shot smoke would weaken it.

**New script**: `~/DEOPT/deopt-v2-sol/script/SmokePerpV2RebateExecute.s.sol`.
This is the broadcast sibling of V2G-B's preflight: it asserts that
the chain IS in rebate state (root + budget + maker negative-ppm
tier + taker positive-ppm tier), signs a tiny PERP cross with both
EOAs, and verifies on-chain vault deltas after the broadcast. Hard-
refuses chain id 8453, V2-not-enabled, OLD == NEW, paused matching,
non-executor caller, missing keys, buyer==seller, and zero/positive
maker ppm. No `setMerkleRoot` / `setFeesManagerV2` /
`setUseFeesManagerV2` / `setFeeConsumer` / `setFeeRecipient` /
`setRebateFundingAccount` / `fundRebateBudget` / `claimTier` — the
invariance assertion at the end reverts if any of those mutated.

Trade shape (defaults):

| Field | Value | Why |
|---|---|---|
| `marketId` | `1` (ETH-PERP) | only active market on NEW |
| `size1e8` | `1000` | yields a 30 000 native-mUSDC notional |
| `price1e8` | `300_000_000_000` ($3000) | matches the live ETH mock feed |
| `buyerIsMaker` | `true` | buyer = Tier 4 maker (rebate leg), seller = Tier 2 taker (fee leg) |
| `deadlineSeconds` | `600` | matches V2F-LM |

Expected on-chain effect (deterministic — Math.mulDiv Ceil for fees, Floor for rebates):

| Account | Vault delta (native mUSDC) |
|---|---|
| Tier 4 maker (`0x290b…9274`) | `+3` (rebate = floor(30 000 × 100 / 1e6) = 3) |
| Tier 2 taker (`0x77cA…0020`) | `−6` (fee = ceil(30 000 × 200 / 1e6) = 6) |
| `rebateFundingAccount` (`0xa67f…b588`) | `−3` (paid out to maker) |
| `feeRecipient` (`0xa67f…b588` — same as funder) | `+6` (collected from taker) |
| FMv2.`rebateBudget(mUSDC)` | `−3` (1_000_000 → 999_997) |

Net funder/feeRecipient vault delta: `+3` (taker fee > maker rebate, as the
fee profile guarantees for any negative-ppm maker tier paired with a positive-ppm
taker tier).

### Preflight result (read-only)

`CONFIRM=false` against the new script:

```
V2G-E PERP rebate smoke (executable)
chainId 84532
PERP_ENGINE (NEW)                         0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
OLD_PERP_ENGINE (must stay stranded)      0xB36395b67D0798ADA981731c9Fa5239F4362b53B
PERP_MATCHING_ENGINE                      0x774d96E5739bffadEE91508b4D3D74F5BE29F165
FEES_MANAGER_V2_ADDRESS                   0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
COLLATERAL_VAULT                          0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
PERP_MARKET_ID                            1
PERP_SMOKE_SIZE_1E8                       1000
PERP_SMOKE_PRICE_1E8                      300_000_000_000
PERP_SMOKE_BUYER_IS_MAKER                 true
State snapshot: before
 Matching.perpEngine()                    NEW
 Matching.paused()                        false
 NEW.useFeesManagerV2()                   true
 OLD.useFeesManagerV2()                   false
 FeesManagerV2.isFeeConsumer(NEW)         true
 FeesManagerV2.isFeeConsumer(OLD)         false
 FeesManagerV2.merkleRoot()               V2G-D2 root (non-zero)
 FeesManagerV2.rebateBudget(mUSDC)        1_000_000
 FeesManagerV2.feeRecipient               Timelock 0xa67f…b588
 FeesManagerV2.rebateFundingAccount       Timelock 0xa67f…b588 (same)
 Vault.balances(feeRecipient, mUSDC)      18      (V2F-LM accrual)
 Vault.balances(rebateFundingAccount, mUSDC) 18    (same account)
 Vault.balances(buyer=maker, mUSDC)       2_000_000
 Vault.balances(seller=taker, mUSDC)      2_000_000
 currentTier(buyer=maker)                 4
 currentTier(seller=taker)                2
 PERP makerPpm at buyer tier              -100
 PERP takerPpm at buyer tier              150
 PERP makerPpm at seller tier             -50
 PERP takerPpm at seller tier             200
 NEW.marketState(1).long/shortOI1e8       2 / 2   (V2F-LM residual)
 OLD.marketState(1).long/shortOI1e8       100_000_000 / 100_000_000   (stranded)
 ETH primary mock updatedAt               1780145482   (~26 min old at preflight time)
V2G-E PERP rebate smoke preflight PASSED. No confirm flag set; no transactions sent.
```

All hard refusal conditions are NOT triggered (matching unpaused,
V2 enabled and wired, root non-zero, budget ≥ `MIN_REBATE_BUDGET`).
The smoke-time precondition `matching.isExecutor(caller)` will be
asserted when the operator runs with `CONFIRM=true` (the operator's
DEPLOYER is `0xc35F7A…3C27`, which V2F-LM showed as an executor).

The ETH mock feed is older than the 60 s `ETH_USDC_MAX_DELAY`
window. The script therefore needs `REFRESH_MOCK_FEEDS_CONFIRM=true`
in the same broadcast (two `MockPriceSource.setPrice` txs land
before the `executeTrade` tx; with `--slow` they confirm in <10 s on
Base Sepolia, well inside the 60 s window).

## Phase 4 — Human gate: PERP rebate broadcast

### Broadcast packet

Operator opens a shell with `DEPLOYER_PRIVATE_KEY` exported (must
be `0xc35F7A…3C27`, which V2F-LM confirmed is an executor on
`PerpMatchingEngine`). Then:

```sh
cd ~/DEOPT/deopt-v2-sol

# Operator-only — keys never echoed, never written to disk, never sent to chat
export DEPLOYER_PRIVATE_KEY=<operator supplies>
export PERP_SMOKE_BUYER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)
export PERP_SMOKE_SELLER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)

PERP_ENGINE=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c \
OLD_PERP_ENGINE=0xB36395b67D0798ADA981731c9Fa5239F4362b53B \
PERP_MATCHING_ENGINE=0x774d96E5739bffadEE91508b4D3D74F5BE29F165 \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3 \
BASE_COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
PERP_SMOKE_SIZE_1E8=1000 \
PERP_SMOKE_PRICE_1E8=300000000000 \
PERP_SMOKE_BUYER_IS_MAKER=true \
ETH_USDC_PRIMARY_SOURCE=0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc \
ETH_USDC_SECONDARY_SOURCE=0x2103a84C0CAB9cf7680d602C8931FaDeD7064517 \
ETH_USDC_MOCK_PRICE_1E8=300000000000 \
MIN_REBATE_BUDGET=1000000 \
REFRESH_MOCK_FEEDS_CONFIRM=true \
SMOKE_PERP_V2_REBATE_EXECUTE_CONFIRM=true \
forge script script/SmokePerpV2RebateExecute.s.sol:SmokePerpV2RebateExecute \
  --rpc-url "$RPC_URL" --broadcast --slow
```

Exactly **3** broadcasts (2 oracle refresh, 1 `executeTrade`). No
admin writes, no Merkle / budget / consumer mutation, no second
trade, no retry. The script's `_assertInvariance` reverts if any
FeesManagerV2 wiring changes mid-run.

### Post-broadcast verification commands

```sh
cast receipt <executeTrade_tx_hash> --rpc-url $RPC_URL
cast call $FMV2 'rebateBudget(address)(uint256)' $MUSDC --rpc-url $RPC_URL     # expect 999_997
cast call $VAULT 'balances(address,address)(uint256)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 $MUSDC --rpc-url $RPC_URL   # expect 2_000_003
cast call $VAULT 'balances(address,address)(uint256)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 $MUSDC --rpc-url $RPC_URL   # expect 1_999_994
cast call $VAULT 'balances(address,address)(uint256)' 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588 $MUSDC --rpc-url $RPC_URL   # expect 21 (18 + 6 − 3)
cast call $PERP 'getPositionSize(address,uint256)(int128)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 1 --rpc-url $RPC_URL   # expect +1000 (long)
cast call $PERP 'getPositionSize(address,uint256)(int128)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 1 --rpc-url $RPC_URL   # expect -1000 (short)
```

Operator pastes the `executeTrade` tx hash back for event decode +
backend / metrics verification (Phase 4b).


## Phase 4 — PERP rebate broadcast result (V2G-E-1)

The operator ran the packet above. Forge logged the script's full
state snapshot delta and then the broadcast block. 3 broadcasts
landed in consecutive blocks `42196088 → 42196090`. Total gas
`906_891` at ~0.006 gwei (≈ `0.0000054 ETH` total executor cost).

| # | Tx | Block | Gas | Purpose |
|---|---|---|---|---|
| 1 | `0x9635d3c52e062a1e0783168eee64bdf4cfdcf52c959c52779942c0e01751920f` | 42196088 | 32 129 | ETH primary mock refresh |
| 2 | `0xee6d112f1b50ee74dcc0293efffd72a4095f3bd78ec79ffaa0d32e31875031bb` | 42196089 | 32 129 | ETH secondary mock refresh |
| 3 | **`0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394`** | **42196090** | **842 633** | `PerpMatchingEngine.executeTrade` |

### Tx 3 receipt (PERP `executeTrade`)

| Field | Value |
|---|---|
| `transactionHash` | `0x5c15e923…aa394` |
| `blockNumber` | `42196090` |
| `blockHash` | `0x288a41ab…7ef0` |
| `from` | `0xc35F7A8A…3C27` (executor) |
| `to` | `0x774d96E5…F165` (PerpMatchingEngine) |
| `status` | **`0x1` (success)** |
| `gasUsed` | `842_633` |
| `effectiveGasPrice` | `6_000_000 wei` (0.006 gwei) |
| `cumulativeGasUsed` | `4_602_616` |
| `transactionIndex` | `31` |
| `type` | `2` (EIP-1559) |
| `logs.count` | `17` |

### Decoded fee events

**FeeRebatedV2** (maker leg) — index 147:
```
emitter           = 0x00da…774f (FeesManagerV2)
consumer          = 0xc6c5…141c (NEW PerpEngine)
trader            = 0x290b…9274 (Tier 4 maker)
recipient         = 0x290b…9274 (trader receives the rebate)
settlementAsset   = 0x6eae…412e (mUSDC)
productKind       = 1 (PERP)
flowKind          = 0 (ORDERBOOK)
rebatePpm         = -100
basisAmount       = 30_000   (native notional)
rebateAmount      = 3        (floor(30_000 × 100 / 1e6))
```

**FeeChargedV2** (taker leg) — index 153:
```
emitter           = 0x00da…774f (FeesManagerV2)
consumer          = 0xc6c5…141c (NEW PerpEngine)
trader            = 0x77cA…0020 (Tier 2 taker)
recipient         = 0xa67f…b588 (Timelock = feeRecipient)
settlementAsset   = 0x6eae…412e (mUSDC)
productKind       = 1 (PERP)
flowKind          = 0 (ORDERBOOK)
isMaker           = false
feePpm            = 200
basisAmount       = 30_000   (native notional)
feeAmount         = 6        (ceil(30_000 × 200 / 1e6))
```

**RebateBudgetSpent**: `settlementAsset=mUSDC, amount=3`

All three events were emitted by FeesManagerV2; the
`PerpMatchingEngine.TradeExecuted` and `PerpEngine.TradeExecuted`
logs also appear. **No log was emitted by `OLD_PERP_ENGINE
0xB363…b53B`** — the script's `_verifyOldUntouched` confirms it.

### Post-broadcast vault + budget verification (live RPC)

| Account / field | Pre-trade | Post-trade | Δ | Expected | Match |
|---|---:|---:|---:|---:|:---:|
| `Vault.balances(maker, mUSDC)` | 2 000 000 | 2 000 003 | +3 | +3 (rebate) | ✅ |
| `Vault.balances(taker, mUSDC)` | 2 000 000 | 1 999 994 | −6 | −6 (fee) | ✅ |
| `Vault.balances(funder, mUSDC)` | 18 | 21 | +3 | +3 (fee 6 − rebate 3) | ✅ |
| `FMv2.rebateBudget(mUSDC)` | 1 000 000 | 999 997 | −3 | −3 (rebate paid) | ✅ |
| `NEW.marketState(1).longOI1e8` | 2 | 1 002 | +1 000 | +1 000 (long side) | ✅ |
| `NEW.marketState(1).shortOI1e8` | 2 | 1 002 | +1 000 | +1 000 (short side) | ✅ |
| `OLD.marketState(1).longOI1e8` | 100 000 000 | 100 000 000 | 0 | 0 (stranded, untouched) | ✅ |
| `NEW.getPositionSize(maker, 1)` | 0 | +1 000 | +1 000 | maker = long (buyerIsMaker=true) | ✅ |
| `NEW.getPositionSize(taker, 1)` | 0 | −1 000 | −1 000 | taker = short | ✅ |
| `PME.nonces(maker)` | 0 | 1 | +1 | one trade signed | ✅ |
| `PME.nonces(taker)` | 0 | 1 | +1 | one trade signed | ✅ |

### Backend admin / indexer verification

Backend started in **read-only** mode (`EXECUTION_ENABLED=false`,
`EXECUTOR_REAL_BROADCAST_ENABLED=false`, `EXECUTOR_PRIVATE_KEY` unset)
with the V2D-S + V2E-F + V2F-O env override stack. Indexer ticked
past the trade block.

`GET /admin/fees/onchain?tx_hash=0x5c15e923…aa394`:

```jsonc
{
  "event_model": "v2",
  "fee_charged_v2_count": 1,
  "fee_rebated_v2_count": 1,
  "observed_total_charged": "6",
  "observed_total_rebated": "3",
  "net_protocol_fee": "3",
  "by_recipient":      {"0xa67f…b588": "6"},
  "by_side":           {"taker": "6"},
  "by_trader":         {"0x77ca…0020": "6"},
  "rebated_by_trader": {"0x290b…9274": "3"},
  "reconciliation_status": "onchain_observed",
  "source_of_truth": "onchain",
  "trading_fee_event_count": 0,
  "events": [
    { "event_name": "FeeChargedV2", "is_maker": false, "product_kind": "perp",
      "flow_kind": "orderbook", "fee_ppm": 200, "basis_amount": "30000",
      "fee_amount": "6", "trader": "0x77ca…0020",
      "recipient": "0xa67f…b588", "settlement_asset": "0x6eae…412e",
      "source_contract": "0x00da…774f", "block_number": 42196090,
      "log_index": 153 },
    { "event_name": "FeeRebatedV2", "is_maker": true, "product_kind": "perp",
      "flow_kind": "orderbook", "rebate_ppm": -100, "basis_amount": "30000",
      "rebate_amount": "3", "trader": "0x290b…9274",
      "recipient": "0x290b…9274", "settlement_asset": "0x6eae…412e",
      "source_contract": "0x00da…774f", "block_number": 42196090,
      "log_index": 147 }
  ]
}
```

Backend decode matches the on-chain decode byte for byte.

### `/metrics` verification

Note: the committed `.env` still references the OLD PerpEngine
address in `PERP_ENGINE_ADDRESS` (V2F-O documented this env-hygiene
gap; per V2F-O the .env was intentionally not flipped in V2F-O
scope). With that env, the V2 metric bucketed the new events under
`consumer="unknown"`. The agent verified the metric routes
correctly to `consumer="new"` by restarting the backend with a
shell-only override (`.env` untouched):

```text
PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
```

After the restart, `/metrics`:

```
# HELP deopt_perp_fee_charged_v2_total PERP FeeChargedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither).
# TYPE deopt_perp_fee_charged_v2_total gauge
deopt_perp_fee_charged_v2_total{consumer="new"} 3        # 2 from V2F-LM Tier-0 + 1 from V2G-E
deopt_perp_fee_charged_v2_total{consumer="old"} 0
deopt_perp_fee_charged_v2_total{consumer="unknown"} 0

# HELP deopt_perp_fee_rebated_v2_total PERP FeeRebatedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither).
# TYPE deopt_perp_fee_rebated_v2_total gauge
deopt_perp_fee_rebated_v2_total{consumer="new"} 1        # the V2G-E trade
deopt_perp_fee_rebated_v2_total{consumer="old"} 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} 0
```

The OLD-consumer alert (`PerpFeeChargedOnOldStrandedEngine` per the
V2F-Q `RUNBOOK_PERP_V2_FEE_ALERTS.md`) stays **green** — zero
events on the stranded `0xB363…b53B` address.

The `.env` file is left **untouched** to preserve the operator's
existing configuration; the env-hygiene cleanup belongs to its own
follow-up milestone (carry-over noted in §"Remaining blockers").

## Phase 5 — OPTION rebate smoke dry-run (preflight)

For OPTION we mirror the PERP approach with a new script
`~/DEOPT/deopt-v2-sol/script/SmokeOptionV2RebateExecute.s.sol`. It
asserts that NEW MarginEngine wires FeesManagerV2 (V2 enabled),
that the maker tier has a negative OPTION makerPpm and the taker
tier has a positive OPTION takerPpm, and that rebate budget +
Merkle root are live. Then it signs an `OptionTrade` with both EOAs
and broadcasts via `OptionMatchingEngine.executeTrade`.

Trade shape (defaults reproduce the V2E-G ETH-call series with a
larger premium so the −50 ppm maker rebate is ≥ 1 native unit):

| Field | Value | Why |
|---|---|---|
| `optionId` | `24145907678156652148…740183` | ETH-call series active since V2D-V |
| `underlying` | `0x4DeEBc5f…BDd02` (mWETH) | matches manifest |
| `settlementAsset` | `0x6eAe407f…412E` (mUSDC) | matches manifest |
| `expiry` | `1893456000` (2030-01-01) | matches V2E-G |
| `strike1e8` | `300_000_000_000` ($3000) | matches V2E-G |
| `isCall` | `true` | matches V2E-G |
| `contractSize1e8` | `100_000_000` (= 1e8) | hard-locked by registry |
| `quantity` | `1` | minimal |
| `premiumPerContract` | `200_000` (= 0.2 mUSDC) | yields premium 200 000 ⇒ rebate 10, fee 25 |
| `buyerIsMaker` | `false` | seller is the maker rebate leg, mirrors V2E-G assignment |

Account assignment (note swap vs PERP):

| Side | EOA | Role |
|---|---|---|
| `buyer` | `0x77cA…0020` | **Tier 2 taker** (pays premium + 125 ppm fee) |
| `seller` | `0x290b…9274` | **Tier 4 maker** (receives premium − 50 ppm rebate) |

Expected on-chain effect:

| Account | Vault Δ | Reason |
|---|---:|---|
| Tier 2 taker (`0x77cA…0020`, buyer) | `−200_025` | −200 000 premium − 25 fee |
| Tier 4 maker (`0x290b…9274`, seller) | `+200_010` | +200 000 premium + 10 rebate |
| Timelock funder/feeRecipient | `+15` | +25 fee − 10 rebate (net) |
| `FMv2.rebateBudget(mUSDC)` | `−10` | 999 997 → 999 987 |

Expected events:
- `FeeChargedV2(consumer=NEW_MARGIN 0x287C…48Cc, trader=taker 0x77cA…0020, recipient=funder 0xa67f…b588, productKind=OPTION, flowKind=ORDERBOOK, isMaker=false, feePpm=125, basisAmount=200_000, feeAmount=25)`
- `FeeRebatedV2(consumer=NEW_MARGIN, trader=maker 0x290b…9274, recipient=maker, productKind=OPTION, flowKind=ORDERBOOK, rebatePpm=-50, basisAmount=200_000, rebateAmount=10)`
- `RebateBudgetSpent(mUSDC, 10)`
- `TradingFeeCharged` V1-compat log (event_model = `mixed`, same as V2E-G)
- `TradeExecuted`, `OptionTradeExecuted`, vault `InternalTransfer` / `Synced`

`OLD_PERP_ENGINE` is irrelevant (OPTION goes through MarginEngine).

### Preflight result (read-only)

`CONFIRM=false` against `SmokeOptionV2RebateExecute`:

```
V2G-E OPTION rebate smoke (executable)
OPTION_MATCHING_ENGINE                  0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
MARGIN_ENGINE                           0x287Cef479be5889eEfCa847F9e73C860898f48Cc
FEES_MANAGER_V2_ADDRESS                 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
OPTION_ID                               24145907678156652148…740183
UNDERLYING                              0x4DeEBc5f…BDd02
OPTION_EXPIRY / STRIKE_1E8 / IS_CALL    1893456000 / 300000000000 / true
OPTION_CONTRACT_SIZE_1E8 / QUANTITY     100000000 / 1
OPTION_PREMIUM_PER_CONTRACT             200000
OPTION_BUYER_IS_MAKER                   false
State snapshot: before
 Matching.marginEngine()                 NEW
 Matching.paused()                       false
 Margin.useFeesManagerV2()               true
 FeesManagerV2.isFeeConsumer(MARGIN)     true
 FeesManagerV2.merkleRoot()              V2G-D2 root (non-zero)
 FeesManagerV2.rebateBudget(mUSDC)       999_997   (after V2G-E PERP)
 Vault.balances(feeRecipient, mUSDC)     21       (after V2G-E PERP)
 Vault.balances(rebateFundingAccount)    21       (same account)
 Vault.balances(buyer=taker, mUSDC)      1_999_994 (after V2G-E PERP)
 Vault.balances(seller=maker, mUSDC)     2_000_003 (after V2G-E PERP)
 currentTier(buyer=taker)                2
 currentTier(seller=maker)               4
 OPTION makerPpm at buyer tier (taker)   -10      (unused here — buyer is taker)
 OPTION takerPpm at buyer tier (taker)   125      ← applied
 OPTION makerPpm at seller tier (maker)  -50      ← applied
 OPTION takerPpm at seller tier (maker)  75       (unused here — seller is maker)
 ETH primary mock updatedAt              1780160464 (~53 min old)
V2G-E OPTION rebate smoke preflight PASSED. No confirm flag set; no transactions sent.
```

All hard refusal conditions remain off. The ETH mock feed is past
the 60 s window, so the broadcast needs `REFRESH_MOCK_FEEDS_CONFIRM=true`
in the same packet (same pattern as PERP).

## Phase 6 — Human gate: OPTION rebate broadcast

### Broadcast packet

```sh
cd ~/DEOPT/deopt-v2-sol

# Operator-only — keys never echoed, never written to disk, never sent to chat
export DEPLOYER_PRIVATE_KEY=<operator supplies — executor 0xc35F7A…3C27>
# Note the swap vs PERP: OPTION_BUYER_IS_MAKER=false → buyer = taker (Tier 2),
# seller = maker (Tier 4). The Solidity argument names follow OptionTrade's
# buyer/seller fields, not the maker/taker role.
export OPTION_SMOKE_BUYER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)
export OPTION_SMOKE_SELLER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)

OPTION_MATCHING_ENGINE=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3 \
BASE_COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
OPTION_QUANTITY=1 \
OPTION_PREMIUM_PER_CONTRACT=200000 \
OPTION_BUYER_IS_MAKER=false \
ETH_USDC_PRIMARY_SOURCE=0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc \
ETH_USDC_SECONDARY_SOURCE=0x2103a84C0CAB9cf7680d602C8931FaDeD7064517 \
ETH_USDC_MOCK_PRICE_1E8=300000000000 \
MIN_REBATE_BUDGET=999000 \
REFRESH_MOCK_FEEDS_CONFIRM=true \
SMOKE_OPTION_V2_REBATE_EXECUTE_CONFIRM=true \
forge script script/SmokeOptionV2RebateExecute.s.sol:SmokeOptionV2RebateExecute \
  --rpc-url "$RPC_URL" --broadcast --slow
```

Exactly **3** broadcasts (2 oracle refresh, 1 `executeTrade`). No
admin writes, no Merkle / budget / consumer mutation, no second
trade. The script's `_assertInvariance` reverts on any FMv2 wiring
mutation; `_logFeeAccounting` reverts if no rebate / fee delta is
observed.

### Post-broadcast verification commands

```sh
cast receipt <executeTrade_tx_hash> --rpc-url $RPC_URL
cast call $FMV2 'rebateBudget(address)(uint256)' $MUSDC --rpc-url $RPC_URL     # expect 999_987
cast call $VAULT 'balances(address,address)(uint256)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 $MUSDC --rpc-url $RPC_URL   # expect 2_200_013
cast call $VAULT 'balances(address,address)(uint256)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 $MUSDC --rpc-url $RPC_URL   # expect 1_799_969
cast call $VAULT 'balances(address,address)(uint256)' 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588 $MUSDC --rpc-url $RPC_URL   # expect 36 (21 + 15)
cast call $NEW_MARGIN 'positionQuantity(address,uint256)(int128)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 24145907678156652148...740183 --rpc-url $RPC_URL   # expect -1 (seller wrote a call)
cast call $NEW_MARGIN 'positionQuantity(address,uint256)(int128)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 24145907678156652148...740183 --rpc-url $RPC_URL   # expect +1 (buyer bought a call)
```

Paste back the `executeTrade` tx hash and the agent will verify
events + backend admin endpoint + metrics (Phase 6b).

## Phase 6 — First broadcast attempt FAILED: `MarginRequirementBreached(seller)`

Date: 2026-05-31.

The operator ran the Phase 6 packet above. Forge's pre-broadcast
simulation traced all the way through `OptionMatchingEngine.executeTrade`
→ `MarginEngine.applyTrade`:

- Buyer/seller positions were updated.
- Premium 200 000 native moved buyer → seller via `transferBetweenAccounts`.
- `consumeFees(taker)` ran the taker leg: `FeeChargedV2(productKind=OPTION,
  flowKind=ORDERBOOK, feePpm=125, basisAmount=200_000, feeAmount=25)`.
- `consumeFees(maker)` ran the maker leg: `FeeRebatedV2(productKind=OPTION,
  flowKind=ORDERBOOK, rebatePpm=-50, basisAmount=200_000, rebateAmount=10)`
  and `RebateBudgetSpent(mUSDC, 10)`.
- `_enforceInitialMargin(seller)` reverted with
  `MarginRequirementBreached(0x290bD12C93E467Bf51c51f5273D35bdDb19e9274)`.

Trace risk numbers for the seller (Tier 4 maker = short call writer):

| Field | Value (native mUSDC) |
|---|---:|
| `equityBase`            | `2 200 013` (= 2 000 003 pre-trade vault + 200 000 premium + 10 rebate) |
| `maintenanceMarginBase` | `900 002 250` (≈ 900 mUSDC) |
| `initialMarginBase`     | `1 080 003 000` (≈ 1080 mUSDC, = 1.2× MM per `im_factor_bps=12000`) |

Because forge aborts the broadcast plan as soon as the simulation
reverts, **no transactions hit the chain**. Both the oracle refresh
txs and the `executeTrade` tx stayed in simulation memory only.

### Did the oracle refresh txs broadcast?

**No.** Live RPC reads after the abort confirm zero mutation:

| Field | Value | Compared to pre-attempt |
|---|---:|---|
| `MockPriceSource(ETH primary).getLatestPrice() updatedAt` | `1780160464` | identical (preflight value) |
| `MockPriceSource(ETH secondary).getLatestPrice() updatedAt` | `1780160466` | identical (preflight value) |
| `FMv2.rebateBudget(mUSDC)` | `999 997` | unchanged from post-PERP — would be `999 987` if rebate had landed |
| `Vault.balances(maker, mUSDC)` | `2 000 003` | unchanged from post-PERP |
| `Vault.balances(taker, mUSDC)` | `1 999 994` | unchanged from post-PERP |
| `Vault.balances(funder, mUSDC)` | `21` | unchanged from post-PERP |
| `mUSDC.balanceOf(maker)` / `mUSDC.balanceOf(taker)` | `0` / `0` | unchanged |
| `OptionMatchingEngine.nonces(maker)` / `(taker)` | `0` / `0` | unchanged — no signed OPTION order consumed |
| `MarginEngine.position(maker, optionId)` | `0` | unchanged — no short call opened |
| `MarginEngine.position(taker, optionId)` | `0` | unchanged — no long call bought |
| ETH balance (maker / taker) | `1 298 729 156 423 370 / 1 298 729 762 811 989` | unchanged from post-Phase-2 |

The forge `--slow` flag does NOT short-circuit simulation: the
script is simulated in full before any tx is dispatched. The two
`MockPriceSource.setPrice` calls only landed in the in-memory
fork.

### Why the seller's IM is ~1.08 B native

`MarginEngine` (and the shared `RiskModule`) computes margin for a
short call from the **strike notional**, not from the premium:

```
strikeNotionalNative
  = strike1e8 × contractSize1e8 × quantity / 1e8 / 1e8 × 10^settlementDecimals
  = 3·10^11 × 1·10^8 × 1 / 1·10^8 / 1·10^8 × 10^6
  = 3·10^9 native mUSDC
  = $3 000 in 6-dp mUSDC
```

Empirically, the risk module's MM factor for a short call is ≈ 30 %
of `strikeNotional`:

```
MM  = strikeNotional × MM_factor + base_per_contract × quantity
    = 3·10^9 × 0.30  + 10·10^6 × 1   ≈ 900 002 250 native    (matches trace)
IM  = MM × 1.2  (= im_factor_bps / BPS)
    = 1 080 003 000 native           (matches trace)
```

The premium (200 000 native) and the rebate (10 native) are
credited to the seller *before* the IM check, but they are
negligible compared to the IM gap of ~1.078 B native. There is no
trade size below `quantity=1` (the engine rejects `quantity=0`),
and `contractSize` is hard-locked at 1e8 by the OptionProductRegistry,
so the seller's IM cannot be reduced without changing the series
(strike).

### Recovery options considered

| Path | Live mutation | Feasibility |
|---|---|---|
| **A** Top up seller vault from DEPLOYER, retry same trade | DEPLOYER mints + seller approves + deposits (4 txs). No contract/governance/registry/Merkle/budget mutation. | ✅ smallest mutation, strongest accounting clarity |
| **B** Reduce option size / risk parameters / premium | `quantity` already at minimum (1), `contractSize1e8` hard-locked at 1e8, strike fixed in series, premium does not affect IM. | ❌ no lever available |
| **C** Create / use a safer (low-strike) option series | Requires deploying + activating a new series via `OptionProductRegistry`, `seriesActivationState`, plus risk-config updates. | ❌ large surface, multiple governance writes; out of scope for V2G-E |

**Path A is the right answer.** It re-uses
`script/PrepareSmokeV1Accounts.s.sol` (V2F-J2 / V2G-E Phase-2 same
pattern), only widening `PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE` to a
size that covers the seller's IM with a comfortable buffer.

### Recovery sizing

The trace IM is `1 080 003 000` native. To keep the broadcast safe
against any oracle / per-contract base drift, fund **1 500 000 000
native (1500 mUSDC) per EOA**. Post-funding vault balances:

| EOA | Pre-funding | Post-funding |
|---|---:|---:|
| Tier 4 maker (`0x290b…9274`) — will be the OPTION trade *seller* (short call writer) | `2 000 003` | `1 502 000 003` (≈ +39 % over IM 1 080 003 000) |
| Tier 2 taker (`0x77cA…0020`) — will be the OPTION trade *buyer* (long call) | `1 999 994` | `1 501 999 994` (the long side has no strike-notional IM; this side gets the leftover mint, harmless) |

The Tier 2 taker is funded symmetrically because
`PrepareSmokeV1Accounts.s.sol` mints+deposits the same amount to
both labelled-keys; there is no single-EOA branch, and adding one
just for V2G-E-RECOVERY would add risk surface for no real benefit
(testnet mUSDC has no supply cap and the extra balance is
discarded post-smoke).

`PERP_SMOKE_FUND_ETH_WEI=0` because the maker and taker both still
hold ~1.298 mETH from Phase 2, well above the ≈ 0.000001 ETH that
their two recovery txs (approve + deposit at ~0.006 gwei) will
cost.

### Recovery Phase 2b preflight (read-only, agent-run)

```
chainId 84532
deployer  ...0xc35F7A…3C27 (placeholder — operator supplies real key)
buyer  (Tier 4 maker EOA) 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274
seller (Tier 2 taker EOA) 0x77cA9DD6cCce2D692FB23877a2db7178807b0020
BASE_COLLATERAL_TOKEN (mUSDC)        0x6eAe407f5640B006faC9965182e238582A3B412E
COLLATERAL_VAULT                     0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE   1_500_000_000   (1500 mUSDC per EOA)
PERP_SMOKE_FUND_ETH_WEI              0               (EOAs already have ~1.3 mETH)
State snapshot: before
 mUSDC.owner()                         0xc35F7A8A…3C27       (deployer = owner)
 mUSDC.decimals()                      6
 Vault.launchActiveCollateral(mUSDC)   true
 Vault.depositsPaused()                false
 buyer  ETH (wei) / mUSDC / Vault      1_298_729_156_423_370 / 0 / 2_000_003
 seller ETH (wei) / mUSDC / Vault      1_298_729_762_811_989 / 0 / 1_999_994
PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM not set; preflight done, no transactions sent.
```

Preflight passes. All `_validatePreconditions` checks hold (mUSDC
owner = deployer, launch-active collateral, deposits unpaused).

### Recovery Phase 2b human broadcast packet

```sh
cd ~/DEOPT/deopt-v2-sol

# Operator-only — keys never echoed, never written to disk, never sent to chat
export DEPLOYER_PRIVATE_KEY=<operator supplies — mUSDC owner 0xc35F7A…3C27>
export PERP_SMOKE_BUYER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)
export PERP_SMOKE_SELLER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)

BASE_COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3 \
PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE=1500000000 \
PERP_SMOKE_FUND_ETH_WEI=0 \
PREPARE_SMOKE_V1_ACCOUNTS_CONFIRM=true \
forge script script/PrepareSmokeV1Accounts.s.sol:PrepareSmokeV1Accounts \
  --rpc-url "$RPC_URL" --broadcast --slow
```

Exactly **6** broadcasts (2 mints + 2 approves + 2 deposits — no
ETH dust because `PERP_SMOKE_FUND_ETH_WEI=0`). All from
DEPLOYER + Tier 4 + Tier 2 EOAs. No PerpEngine / MarginEngine /
MatchingEngine / FeesManagerV2 / OptionProductRegistry calls. Same
`_verifyPostState` invariants as Phase 2.

### Recovery Phase 6b — retry OPTION rebate broadcast

After Phase 2b lands, run the SAME packet from Phase 6 above —
unchanged. The preflight check `_validatePreconditions` already
passes; the only thing that was failing was the post-trade
`_enforceInitialMargin(seller)`, which now holds:
`equityBase 1 502 200 013 ≥ initialMarginBase 1 080 003 000`.

### Carry-forward verifications after Phase 6b

```sh
cast call $FMV2 'rebateBudget(address)(uint256)' $MUSDC --rpc-url $RPC_URL                                                                # expect 999_987
cast call $VAULT 'balances(address,address)(uint256)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 $MUSDC --rpc-url $RPC_URL                # expect 1_702_200_013   (1_502_000_003 + 200_000 premium + 10 rebate)
cast call $VAULT 'balances(address,address)(uint256)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 $MUSDC --rpc-url $RPC_URL                # expect 1_301_799_969   (1_501_999_994 − 200_000 premium − 25 fee)
cast call $VAULT 'balances(address,address)(uint256)' 0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588 $MUSDC --rpc-url $RPC_URL                # expect 36              (21 + 25 fee − 10 rebate)
cast call $NEW_MARGIN 'getPositionQuantity(address,uint256)(int128)' 0x290bD12C93E467Bf51c51f5273D35bdDb19e9274 $OPTION_ID --rpc-url $RPC_URL   # expect -1
cast call $NEW_MARGIN 'getPositionQuantity(address,uint256)(int128)' 0x77cA9DD6cCce2D692FB23877a2db7178807b0020 $OPTION_ID --rpc-url $RPC_URL   # expect +1
```

V2G-E remains open: the on-chain state has not regressed (Phase 6
revert was a clean abort, not a partial mutation), and Phase 4
(PERP) results stand on their own. Phase 2b (the recovery
top-up) is the only new human broadcast before Phase 6b retry.

## Phase 2b — Vault funding top-up RESULT (broadcast)

The operator ran the Phase 2b packet. 6 broadcasts landed at blocks
`42205635 → 42205640` for `346 592 gas` total at ~0.006 gwei
(≈ `0.0000021 ETH` total executor cost).

| # | Signer | Action | Tx |
|---|---|---|---|
| 1 | DEPLOYER | `mUSDC.mint(maker, 1_500_000_000)` | `0x665752e381772727b96f5aa34f71f8bfb3d3df142dd55a73d46d228231b3fdf6` |
| 2 | DEPLOYER | `mUSDC.mint(taker, 1_500_000_000)` | `0xc50253332319ce73517f852e8bb2a23586c1278ec4605a4b561acbec6e50477f` |
| 3 | Tier 4 maker | `mUSDC.approve(VAULT, 1_500_000_000)` | `0xb1670d1d514eb5f29284839e9fb88d0e54fafe87d7eef69d793eb4a0b95a1130` |
| 4 | Tier 4 maker | `Vault.deposit(mUSDC, 1_500_000_000)` | `0x6a8451384e745b03e23b22733f551d7d038cfb5abec9c59ffdfd08526eb08238` |
| 5 | Tier 2 taker | `mUSDC.approve(VAULT, 1_500_000_000)` | `0xb50188a8e91fd185dbee18dcc1a186de7c13aa1159d5581e4fc8fd7b73fe7890` |
| 6 | Tier 2 taker | `Vault.deposit(mUSDC, 1_500_000_000)` | `0xb5780741133a5c7c7bd4510b3bc34f3305670eb3f7840cf30ce15f74f975d6a7` |

Deposit gas now `74_217` per leg (vs `108_417` in Phase 2) because
the per-(account,token) vault slot is no longer cold — the SSTORE
just bumps an existing balance.

`PrepareSmokeV1Accounts._verifyPostState` asserted both
`Vault.balances(_, mUSDC)` increased by exactly `1_500_000_000`.

### Post-broadcast live RPC verification

| Field | Pre-Phase-2b | Post-Phase-2b | Δ |
|---|---:|---:|---:|
| `Vault.balances(maker, mUSDC)` | `2 000 003` | `1 502 000 003` | `+1 500 000 000` |
| `Vault.balances(taker, mUSDC)` | `1 999 994` | `1 501 999 994` | `+1 500 000 000` |
| `mUSDC.balanceOf(maker)` / `(taker)` | `0` / `0` | `0` / `0` | minted → deposited atomically |
| `Vault.balances(funder, mUSDC)` | `21` | `21` | unchanged |
| `FMv2.rebateBudget(mUSDC)` | `999 997` | `999 997` | unchanged |
| `FMv2.merkleRoot()` | `0xd8a627d7…fc2df` | unchanged | — |
| `currentTier(maker)` / `(taker)` | `4` / `2` | `4` / `2` | unchanged |
| ETH balance (maker / taker) | `1 298 729 156 423 370 / 1 298 729 762 811 989` | `1 297 997 410 133 558 / 1 297 998 016 522 177` | ≈ −732 k wei each (gas for approve+deposit) |

Seller (Tier 4 maker) vault `1 502 000 003` ≥ IM `1 080 003 000`
with `422 197 013` native of headroom **after** the premium+rebate
credit during applyTrade — the IM check that aborted Phase 6 will
now hold.

### Recovery dry-run re-confirmation (read-only)

Re-ran `SmokeOptionV2RebateExecute` with `CONFIRM=false` against
the new live state. The script's `_validatePreconditions` still
returns PASSED (matching active, V2 wired, fee consumer set, root
non-zero, budget ≥ minimum, maker tier has negative makerPpm, taker
tier has positive takerPpm). The IM check happens inside
`applyTrade` — only forge's full broadcast simulation can prove it
now passes — which we ask the operator to do via the same Phase 6
broadcast packet (unchanged from before the failed attempt).

## Phase 6b — OPTION rebate broadcast retry (HUMAN GATE)

Re-run the SAME Phase 6 packet. No changes to the inputs.

```sh
cd ~/DEOPT/deopt-v2-sol

# Operator-only — keys never echoed, never written to disk, never sent to chat
export DEPLOYER_PRIVATE_KEY=<operator supplies — executor 0xc35F7A…3C27>
export OPTION_SMOKE_BUYER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)
export OPTION_SMOKE_SELLER_PRIVATE_KEY=$(jq -r '.[0].private_key' ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)

OPTION_MATCHING_ENGINE=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc \
FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f \
COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3 \
BASE_COLLATERAL_TOKEN=0x6eAe407f5640B006faC9965182e238582A3B412E \
OPTION_QUANTITY=1 \
OPTION_PREMIUM_PER_CONTRACT=200000 \
OPTION_BUYER_IS_MAKER=false \
ETH_USDC_PRIMARY_SOURCE=0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc \
ETH_USDC_SECONDARY_SOURCE=0x2103a84C0CAB9cf7680d602C8931FaDeD7064517 \
ETH_USDC_MOCK_PRICE_1E8=300000000000 \
MIN_REBATE_BUDGET=999000 \
REFRESH_MOCK_FEEDS_CONFIRM=true \
SMOKE_OPTION_V2_REBATE_EXECUTE_CONFIRM=true \
forge script script/SmokeOptionV2RebateExecute.s.sol:SmokeOptionV2RebateExecute \
  --rpc-url "$RPC_URL" --broadcast --slow
```

Expected this run:

| Account | Vault Δ |
|---|---:|
| Tier 2 taker (`0x77cA…0020`, buyer) | `−200 025` (premium 200 000 + fee 25) |
| Tier 4 maker (`0x290b…9274`, seller) | `+200 010` (premium 200 000 + rebate 10) |
| Timelock funder/feeRecipient | `+15` (fee 25 − rebate 10) |
| `FMv2.rebateBudget(mUSDC)` | `999 997 → 999 987` (−10) |

Expected events on the `executeTrade` tx:
- `FeeChargedV2(consumer=NEW_MARGIN 0x287C…48Cc, trader=taker, productKind=OPTION, flowKind=ORDERBOOK, isMaker=false, feePpm=125, basisAmount=200_000, feeAmount=25)`
- `FeeRebatedV2(consumer=NEW_MARGIN, trader=maker, productKind=OPTION, flowKind=ORDERBOOK, rebatePpm=-50, basisAmount=200_000, rebateAmount=10)`
- `RebateBudgetSpent(mUSDC, 10)`
- A V1-compat `TradingFeeCharged` per side (event_model = `mixed` per V2E-G precedent)
- `TradeExecuted` (MarginEngine), `OptionTradeExecuted` (OptionMatchingEngine)
- Vault `InternalTransfer` x3 (premium leg + rebate leg + fee leg) and `Synced` per side

`OLD_PERP_ENGINE` is untouched (OPTION goes through MarginEngine, not PerpEngine).

Operator pastes the `executeTrade` tx hash and I continue with the
on-chain event decode + backend admin endpoint + metrics
verification.

## Phase 6b — OPTION rebate broadcast result

The operator ran the Phase 6b packet. 3 broadcasts landed at
blocks `42206001 → 42206003` for `1 110 494 gas` total at
~0.006 gwei (≈ `0.0000067 ETH` total executor cost). Forge
simulation passed `_enforceInitialMargin(seller)` this time (the
script's logged delta accounting confirms it before any
broadcasts left the simulation).

| # | Tx | Block | Gas | Purpose |
|---|---|---|---|---|
| 1 | `0x5dff0a4032313cad5bbf47096ccd4e4cbd6d7d75c03712b63ab80361c4cdb99f` | 42206001 | 32 129 | ETH primary mock refresh |
| 2 | `0x0d1706bcdc05977d8d15b87b4d2982f432ae4695ab7f92a100f127c460110c03` | 42206002 | 32 129 | ETH secondary mock refresh |
| 3 | **`0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149`** | **42206003** | **1 046 236** | `OptionMatchingEngine.executeTrade` |

### Tx 3 receipt (OPTION `executeTrade`)

| Field | Value |
|---|---|
| `transactionHash` | `0x9a85cbce…3149` |
| `blockNumber` | `42206003` |
| `from` | `0xc35F7A8A…3C27` (executor) |
| `to` | `0xf2D1D85c…F420b` (OptionMatchingEngine) |
| `status` | **`0x1` (success)** |
| `gasUsed` | `1 046 236` |
| `effectiveGasPrice` | `6 000 000 wei` (0.006 gwei) |
| `cumulativeGasUsed` | `5 895 240` |
| `transactionIndex` | `24` |
| `type` | `2` (EIP-1559) |
| `logs.count` | `21` |

Log emitter breakdown:

| Address | Count | Role |
|---|---:|---|
| `0x00340c360353a5ab784c5bc5c44322a6af0625d3` | 15 | CollateralVault (InternalTransfer + Synced per leg) |
| `0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f` | 3 | FeesManagerV2 (FeeChargedV2 + FeeRebatedV2 + RebateBudgetSpent) |
| `0x287cef479be5889eefca847f9e73c860898f48cc` | 2 | NEW MarginEngine (TradeExecuted + V1-compat TradingFeeCharged) |
| `0xf2d1d85cd363be3bc160d14883c80e7c2c4f420b` | 1 | OptionMatchingEngine (OptionTradeExecuted) |

`OLD_PERP_ENGINE 0xB363…b53B` emitted nothing (irrelevant for OPTION, but the invariance check still holds).

### Decoded fee events

**FeeRebatedV2** (maker leg) — log index 133:
```
emitter           = 0x00da…774f (FeesManagerV2)
consumer          = 0x287c…48cc (NEW MarginEngine)
trader            = 0x290b…9274 (Tier 4 maker)
recipient         = 0x290b…9274 (trader receives the rebate)
settlementAsset   = 0x6eae…412e (mUSDC)
productKind       = 0 (OPTION)
flowKind          = 0 (ORDERBOOK)
rebatePpm         = -50
basisAmount       = 200_000   (premium native)
rebateAmount      = 10        (floor(200_000 × 50 / 1e6))
```

**FeeChargedV2** (taker leg) — log index 125:
```
emitter           = 0x00da…774f (FeesManagerV2)
consumer          = 0x287c…48cc (NEW MarginEngine)
trader            = 0x77cA…0020 (Tier 2 taker)
recipient         = 0xa67f…b588 (Timelock = feeRecipient)
settlementAsset   = 0x6eae…412e (mUSDC)
productKind       = 0 (OPTION)
flowKind          = 0 (ORDERBOOK)
isMaker           = false
feePpm            = 125
basisAmount       = 200_000   (premium native)
feeAmount         = 25        (ceil(200_000 × 125 / 1e6))
```

**RebateBudgetSpent**: `settlementAsset=mUSDC, amount=10`

The V1-compat `TradingFeeCharged` is emitted by MarginEngine for
the taker leg only (V2 fee path early-returns before emitting the
V1-compat log on the rebate leg), per V2E-G precedent. `event_model`
in the backend admin response is `mixed`.

### Post-broadcast vault + budget verification (live RPC)

| Account / field | Pre-OPTION | Post-OPTION | Δ | Expected | Match |
|---|---:|---:|---:|---:|:---:|
| `Vault.balances(maker, mUSDC)` | `1 502 000 003` | `1 502 200 013` | +200 010 | +200 000 premium + 10 rebate | ✅ |
| `Vault.balances(taker, mUSDC)` | `1 501 999 994` | `1 501 799 969` | −200 025 | −200 000 premium − 25 fee | ✅ |
| `Vault.balances(funder, mUSDC)` | `21` | `36` | +15 | +25 fee − 10 rebate | ✅ |
| `FMv2.rebateBudget(mUSDC)` | `999 997` | `999 987` | −10 | −10 (rebate paid) | ✅ |
| `OME.nonces(maker)` | `0` | `1` | +1 | one trade signed | ✅ |
| `OME.nonces(taker)` | `0` | `1` | +1 | one trade signed | ✅ |
| `FMv2.merkleRoot()` | V2G-D2 | unchanged | 0 | invariant | ✅ |
| `currentTier(maker)` / `(taker)` | `4` / `2` | `4` / `2` | 0 | invariant | ✅ |

### Backend admin / indexer verification

After the indexer ticked past block `42206003`:

`GET /admin/fees/onchain?tx_hash=0x9a85cbce…3149`:

```jsonc
{
  "event_model": "mixed",
  "fee_charged_v2_count": 1,
  "fee_rebated_v2_count": 1,
  "observed_total_charged": "25",
  "observed_total_rebated": "10",
  "net_protocol_fee": "15",
  "by_recipient":      {"0xa67f…b588": "25"},
  "by_side":           {"taker": "25"},
  "by_trader":         {"0x77ca…0020": "25"},
  "rebated_by_trader": {"0x290b…9274": "10"},
  "reconciliation_status": "onchain_observed",
  "source_of_truth": "onchain",
  "trading_fee_event_count": 1,        // V1-compat TradingFeeCharged for the fee leg
  "events": [
    { "event_name": "TradingFeeCharged", "is_maker": false, "fee_amount": "25",
      "trader": "0x77ca…0020", "recipient": "0xa67f…b588",
      "source_contract": "0x287c…48cc", "block_number": 42206003, "log_index": 131 },
    { "event_name": "FeeChargedV2", "is_maker": false, "product_kind": "option",
      "flow_kind": "orderbook", "fee_ppm": 125, "basis_amount": "200000",
      "fee_amount": "25", "trader": "0x77ca…0020", "recipient": "0xa67f…b588",
      "source_contract": "0x00da…774f", "block_number": 42206003, "log_index": 125 },
    { "event_name": "FeeRebatedV2", "is_maker": true, "product_kind": "option",
      "flow_kind": "orderbook", "rebate_ppm": -50, "basis_amount": "200000",
      "rebate_amount": "10", "trader": "0x290b…9274", "recipient": "0x290b…9274",
      "source_contract": "0x00da…774f", "block_number": 42206003, "log_index": 133 }
  ]
}
```

Backend decode matches the on-chain decode byte for byte. The
event ordering inside the tx (`FeeChargedV2(125)` →
`TradingFeeCharged(131)` → `FeeRebatedV2(133)`) is consistent with
the MarginEngine call order: V2 fee path runs for the taker leg
(emitting `FeeChargedV2` + V1-compat `TradingFeeCharged`), then for
the maker leg (emitting `FeeRebatedV2` only — V2 rebate path
returns before the V1-compat log).

### `/metrics` verification (with the shell-override `PERP_ENGINE_ADDRESS=NEW`)

```
deopt_perp_fee_charged_v2_total{consumer="new"} 3        # unchanged from PERP-only (no OPTION counts here by design)
deopt_perp_fee_charged_v2_total{consumer="old"} 0
deopt_perp_fee_charged_v2_total{consumer="unknown"} 0
deopt_perp_fee_rebated_v2_total{consumer="new"} 1        # the V2G-E PERP trade
deopt_perp_fee_rebated_v2_total{consumer="old"} 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} 0
```

The V2F-Q metric set (`PERP_V2_FEE_REBATE_METRICS_ALERTING_V2F_Q.md`)
covers PERP only. There is no OPTION-specific `deopt_option_fee_*_v2`
counter yet — that's an explicit V2F-Q scope decision (deferred to
a future metrics expansion). The OPTION V2 events are still
captured in the indexer / `/admin/fees/onchain` path, where the
V2G-E rebate is decoded with full fidelity.

The OLD-consumer alert remains **green** (zero events on
`0xB363…b53B`), consistent with the V2F-Q runbook.





## Phase 7 — Backend / admin / metrics verification (both txs)

Both V2G-E txs decode end-to-end through the V2F-N event indexer +
V2F-O admin endpoint + V2F-P/V2F-Q metric pipeline.

`/admin/fees/onchain?tx_hash=<perp_tx>` and `<option_tx>` returned
**reconciled** results (`source_of_truth=onchain`,
`reconciliation_status=onchain_observed`); `event_model=v2` for the
PERP leg (only V2 events on the tx) and `mixed` for the OPTION leg
(V2 events plus the V1-compat `TradingFeeCharged` emitted by the
MarginEngine for the taker fee leg, per V2E-G precedent).

`/metrics` PERP counters (after the operator-shell-only
`PERP_ENGINE_ADDRESS=NEW` override that V2F-O documented as a
backlogged env-hygiene cleanup):

```
deopt_perp_fee_charged_v2_total{consumer="new"}      = 3   (2 V2F-LM Tier-0 + 1 V2G-E)
deopt_perp_fee_charged_v2_total{consumer="old"}      = 0
deopt_perp_fee_charged_v2_total{consumer="unknown"}  = 0
deopt_perp_fee_rebated_v2_total{consumer="new"}      = 1   (V2G-E)
deopt_perp_fee_rebated_v2_total{consumer="old"}      = 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"}  = 0
```

OPTION-specific `deopt_option_fee_*_v2_total` counters do not
exist yet (V2F-Q scope was PERP-only by design). OPTION rebate
visibility flows entirely through `/admin/fees/onchain` for now.

The OLD-stranded alert (`PerpFeeChargedOnOldStrandedEngine`) stays
**green** — no events have been emitted by `0xB363…b53B` since
V2F-LM stranding.

## Phase 8 — Docs

This file is the V2G-E result. Companion updates:

- `docs/FEES_MANAGER_V2_CLAIM_TIER_RESULT_V2G_D3.md` — V2G-E
  closure note pointing here.
- `docs/REBATE_LIVE_SMOKE_PLAN_V2G_A.md` — V2G-E result section
  pointing here.

No new ALERTING_SPEC.md / RUNBOOK_PERP_V2_FEE_ALERTS.md changes
needed: the V2F-Q alert thresholds (OLD-consumer = 0, unknown-
consumer = 0) all observed correctly in V2G-E.

## Phase 9 — Validation

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` (deopt-v2-backend) | ✅ clean (no source changes in backend) |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo build --all-targets --all-features` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **661 passed, 0 failed, 0 ignored** (unchanged from V2G-D3 baseline) |
| `forge fmt` (deopt-v2-sol) | ✅ applied (also formatted V2G-A/V2G-B scripts that the linter caught) |
| `forge fmt --check` (post-format) | ✅ clean |
| `forge build` (deopt-v2-sol) | ✅ clean (lint warnings only — `mixed-case-variable` and `unsafe-typecast` notes, no errors) |
| `forge test --no-match-path 'test/fork/*'` (deopt-v2-sol) | ✅ **208 tests passed, 0 failed, 0 skipped** in 296.64 s |

Note: forge fork tests (`test/fork/*`) were skipped because they
need a live RPC fork; V2G-D3 ran them earlier. The on-chain smoke
campaign itself is the equivalent fork verification for V2G-E.

Frontend: untouched in V2G-E scope (no frontend changes).

Frontend: untouched in V2G-E scope (no frontend changes).

## Final summary — accounting truth table

| Leg | tx | gas | basisAmount | feePpm/rebatePpm | feeAmount | rebateAmount | budget Δ | maker vault Δ | taker vault Δ | funder vault Δ |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| PERP | `0x5c15e923…aa394` | 842 633 | 30 000 | feePpm=200 / rebatePpm=−100 | 6 | 3 | −3 | +3 | −6 | +3 |
| OPTION | `0x9a85cbce…3149` | 1 046 236 | 200 000 | feePpm=125 / rebatePpm=−50 | 25 | 10 | −10 | +200 010 (incl. premium) | −200 025 (incl. premium) | +15 |

| Running totals | Initial | Post-PERP | Post-OPTION |
|---|---:|---:|---:|
| `FMv2.rebateBudget(mUSDC)` | `1 000 000` | `999 997` | `999 987` |
| `Vault.balances(maker, mUSDC)` | `2 000 000` (post Phase-2) | `2 000 003` | `1 502 200 013` (post Phase-2b top-up + OPTION) |
| `Vault.balances(taker, mUSDC)` | `2 000 000` (post Phase-2) | `1 999 994` | `1 501 799 969` (post Phase-2b top-up + OPTION) |
| `Vault.balances(funder, mUSDC)` | `18` (pre-Phase-2) | `21` | `36` |

All accounting reconciles to the native-unit penny on every leg.
No `OLD_PERP_ENGINE` mutation. No FeesManagerV2 wiring / Merkle /
budget mutation outside of `rebateBudget` decrement-only flows. No
DB rows deleted; the backend's V2-event ledger gained 3 rows total
(FeeChargedV2 PERP, FeeChargedV2 OPTION, FeeRebatedV2 OPTION +
FeeRebatedV2 PERP).

## Remaining blockers / carry-forward

1. **`.env` env-hygiene (V2F-O carry-over).** The committed `.env`
   still references `PERP_ENGINE_ADDRESS=0xB363…b53B` (OLD) and has
   no `OLD_PERP_ENGINE_ADDRESS=…` entry. The V2F-P/V2F-Q metric
   consumer classifier therefore buckets the V2G-E PERP fee events
   as `consumer="unknown"` under the committed env. V2G-E worked
   around this with a shell-only override (`.env` untouched). The
   real fix is a one-line `.env` edit (out of scope here per the
   user's hard rules). Cleanup belongs to its own milestone.

2. **OPTION V2 metrics.** V2F-Q only added PERP-specific
   `deopt_perp_fee_*_v2_total` counters. OPTION-specific
   `deopt_option_fee_*_v2_total` counters were intentionally
   deferred. Since V2G-E proved the OPTION rebate path on chain,
   adding the OPTION metrics is now an obvious next milestone.

3. **Recovery vault headroom.** Both EOAs now hold ~1.5 B native
   mUSDC in the vault. Future smoke trades (if any) should pick a
   strike-notional small enough that the IM fits — or top up
   further. The Phase-2b top-up was the smallest mutation that
   unblocked the existing ETH-$3000 series, but it leaves the
   smoke EOAs over-funded relative to the trade size; this is
   harmless on testnet.

## V2G-F closure (appended 2026-05-31)

V2G-F closed all four blockers carried forward from V2G-E:

1. **Env hygiene.** `.env.example` flipped to canonical NEW
   addresses + added `MARGIN_ENGINE`, `OLD_MARGIN_ENGINE_ADDRESS`,
   `FEES_MANAGER_V2` entries. Real `.env` left untouched per the
   hard rules; the operator patch is recorded in
   `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md`.
2. **OPTION V2 metrics.** Added
   `deopt_option_fee_charged_v2_total{consumer=...}` +
   `deopt_option_fee_rebated_v2_total{consumer=...}` mirroring the
   PERP shape. Both gauges classify against
   `MARGIN_ENGINE` (NEW) and `OLD_MARGIN_ENGINE_ADDRESS` (OLD) via a
   new `src/fees/option_consumer.rs` classifier that re-uses the
   PERP matcher to keep behaviour 1:1.
3. **Alerting.** `docs/alertmanager/option_v2_fee_alerts.yml` (new)
   ships `OptionFeeChargedFromOldMarginEngine`,
   `OptionFeeRebatedFromOldMarginEngine`,
   `OptionFeeConsumerUnknown`, and `FeesManagerV2RebateBudgetLow`.
   The merkle-root-unset operational notice is retired (root has
   been live since V2G-C; tier claims rely on it). Runbook +
   ALERTING_SPEC updated.
4. **Executor readiness.** Audited; no code change needed. Both
   `sign_perp_trade` and `sign_option_execution_intent` already
   support shell-only keys (the latter via `--private-key-env`).
   Operator pattern for V2G-D2 EOAs without committing secrets
   documented in V2G-F.

Live read-only verification under V2G-F reproduced clean against
both V2G-E txs:

```
deopt_perp_fee_charged_v2_total{consumer="new"} 3      old=0 unknown=0
deopt_perp_fee_rebated_v2_total{consumer="new"} 1      old=0 unknown=0
deopt_option_fee_charged_v2_total{consumer="new"} 3    old=0 unknown=0
deopt_option_fee_rebated_v2_total{consumer="new"} 1    old=0 unknown=0
deopt_fees_manager_v2_rebate_budget_native{
  asset="0x6eae407f5640b006fac9965182e238582a3b412e"
} 999987     (matches FMv2.rebateBudget(mUSDC) on chain)
```

Test suite: 661 → 675 passed (+14 V2G-F tests).

## Next recommended milestone

**V2G-F — close the V2G band.**

- Cut over the V2F-Q `PerpFeeChargedOnOldStrandedEngine` /
  `PerpFeeChargedFromUnknownConsumer` alerts to PRODUCTION-FIRING
  state (they have been observed quiet under live load: zero events
  on OLD across V2F-LM + V2G-E).
- Retire the V2G-A "merkle root unset" operational notice (root has
  been live since V2G-C / V2G-D2 with `currentTier ≥ 2` claims since
  V2G-D3).
- Land the OPTION V2 metric counters (deferred from V2F-Q).
- Land the `.env` env-hygiene cleanup (`PERP_ENGINE_ADDRESS=NEW` +
  `OLD_PERP_ENGINE_ADDRESS=OLD`).

## V2G-G closure (appended 2026-05-31)

V2G-G productionised the observability surface that V2G-E exercised
and V2G-F instrumented. See
`docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` for the full record.

In one paragraph: the V2G-E PERP tx
`0x5c15e923…aa394` and OPTION tx `0x9a85cbce…3149` were re-resolved
through the new `GET /admin/fees/v2/observability` endpoint (the
V2G-G admin snapshot route) under the same shell-only override stack
V2G-F used, and the four V2 fee gauges + budget gauge reproduced the
exact V2G-F closure values (PERP charged{new}=3, rebated{new}=1,
OPTION charged{new}=3, rebated{new}=1, all OLD/unknown=0, mUSDC
rebate budget = 999 987). The seven Prometheus alerts (3 PERP + 3
OPTION + budget low) plus the V2G-G additions
(`FeesManagerV2RebateBudgetStale`, `DeoptV2FeeMetricsAbsent`) were
all evaluated logically against the current `/metrics` shape and
none would fire. The deployable rules + Grafana dashboard + frontend
admin section + operator `.env` patch are now committed.

Test suite: 675 → 679 (+4 V2G-G HTTP tests on the new admin endpoint).

