# FIRST LIVE OPTION-EXECUTION SMOKE — V2 — Base Sepolia — RESULT

## 0. Identity

| Field | Value |
|---|---|
| Milestone | FIRST_LIVE_SMOKE-EXEC-V2-SEPOLIA-FEE-ONLY |
| Chain | Base Sepolia (`84532`) |
| Verifier | independent re-fetch by FIRST_LIVE_SMOKE-V2-POST-CLOSE-VERIFY |
| Result | **CONFIRMED ON-CHAIN — SUCCESS** |
| Recorded (UTC) | 2026-06-08 (post-broadcast verification window) |
| Scope | single fee-only orderbook execution; no rebate path; no mainnet |

## 1. Broadcast tx

| Field | Value |
|---|---|
| **tx_hash** | `0xb2379a462e5aa82ea1d7e62cf4ae38c7143a41e8ea42d6c54b7ba298c061e800` |
| blockNumber | `42_571_249` |
| transactionIndex | `1` |
| status | `0x1` (success) |
| from | `0x295005fd4F311e6691F008D57d32FCFEde844518` (BACKEND_EXECUTOR) |
| to | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` (NEW_OME) |
| gasUsed | `907_100` (under 938 057 estimate; under 1 500 000 cap) |
| effectiveGasPrice | `6_000_000` wei (0.006 gwei) |
| L1 fee | `608_996_558_072` wei |
| Total BE.balance debit | `6_051_596_558_072` wei (~6.05e12) |
| logs.length | `23` |

## 2. Intent

| Field | Value |
|---|---|
| INTENT_ID | `f9e765c2-cc9b-4863-918c-a4db04867d46` |
| onchain_intent_id | `0xce0546da8eb15dd3a6b9173c946430a6b75c74461a6a52e83368d0c614262d2e` |
| source_type | `option_orderbook_fill` |
| source_id (fill_id) | `1294d1e3-d285-49ad-946b-8a0bc182b464` |
| series_id | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` (ETH call, strike $3000, expiry 2030-01-01) |
| signing digest | `0xcaab21790b6bd4af071e0306c39726c227f4a981a732615839fdbc6010a6ab0a` |
| domain.chainId | `84532` |
| domain.verifyingContract | NEW_OME |
| primary_type | `OptionTrade` |
| simulation block | `42_571_214` |
| simulation_status | `simulation_ok` |
| backend status (post) | `broadcast_submitted` (chain-side is authoritative — confirmation worker is disabled, see §11) |
| backend error | `null` |

## 3. Orders

| Side | Account | order_id | nonce | client_order_id | status |
|---|---|---|---|---|---|
| SELL (maker) | `0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88` | `a64ee052-e807-4837-8c51-78adc0438415` | 101 | `first-live-smoke-v2-20260608T092403Z-sell` | open → consumed by match |
| BUY (taker) | `0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC` | `d04a8ae8-7b6f-455e-b05a-15a95643f7cd` | 101 | `first-live-smoke-v2-20260608T092403Z-buy` | filled (1 fill) |

Match price (= maker price): `price_1e8 = 1_000_000_000` ($10/contract). Size: `100_000_000` (1 contract). Premium per contract native: `10_000_000` mUSDC native (= $10 at 6-decimals).

## 4. BACKEND_EXECUTOR state delta

| Field | Pre | Post | Delta |
|---|---|---|---|
| nonce | 0 | **1** | **+1** ✓ exactly one tx |
| balance (wei) | `3 800 000 000 000 000` | `3 793 948 403 441 928` | `−6 051 596 558 072` |
| code | `0x` | `0x` | EOA preserved |

Headroom: BE.balance still ≥ halt-floor `2e15` (≈ 626 more tx at observed Base Sepolia gas pricing).

## 5. Accounting reconciliation (mUSDC native)

| Account | Pre | Post | Observed Δ | Expected Δ |
|---|---|---|---|---|
| BUYER `CV.balances` | `100 020 000 000` | `100 009 997 500` | **`−10 002 500`** | premium `−10 000 000` + taker fee `−2 500` (250 ppm) = **`−10 002 500`** ✓ |
| SELLER `CV.balances` | `100 020 000 000` | `100 029 999 500` | **`+9 999 500`** | premium `+10 000 000` + maker fee `−500` (50 ppm) = **`+9 999 500`** ✓ |
| PFV.feeBalance | `28` | `3 028` | **`+3 000`** | maker fee `500` + taker fee `2 500` = **`+3 000`** ✓ |
| PFV.rebateReserve | `0` | `0` | `0` | unchanged ✓ |
| CV(PFV, mUSDC) | `28` | `3 028` | `+3 000` | matches PFV.feeBalance delta ✓ |
| FM.rebateBudget(mUSDC) | `999 947` | `999 947` | `0` | unchanged (no rebate consumed) ✓ |

**Fee basis observed: PREMIUM × ppm / 1_000_000** (matches `FM.productFeeBasis(OPTION=0) = PREMIUM` per `IFeesManagerV2.sol:23`).

**Conservation:** BUYER Δ + SELLER Δ + PFV Δ = `−10_002_500 + 9_999_500 + 3_000 = 0` ✓ zero-sum.

## 6. On-chain events (23 logs total)

| Emitter | Address | topic0 | Count | Interpretation |
|---|---|---|---|---|
| **NEW_OME** | `0x5a5e…70f6` | `0xb2387b9f0e4823ecef9a16ea4aaba6598c0703fb5e9d8dba37ef303add4cb808` | **1** | OptionTradeExecuted; topic1 = onchain_intent_id; topic2 = buyer; topic3 = seller |
| **NEW_FM_V2** | `0xf662…2944` | `0xbbbe34ce8cca9ce3246f3848d7a433d098b1f5e967b78e46c71957302d07f043` | **2** | FeeCharged (buyer side + seller side) |
| **PFV** | `0x7c0a…1886` | `0x8a6211fcbaec33871f06dc695956ebd0159a99db86160410d1d22fd13ecc7fa8` | **2** | `onFeeCharged` vault hook acks (one per FM event) |
| Option Position Token | `0x506c…0d30` | `0x12cf6338…` (×2), `0x6f0909c4…` (×1) | 3 | position mint events for buyer + seller; trade event |
| **CV** | `0x0034…25d3` | `0xf67cd268…` (×12), `0x77178bcf…` (×3) | 15 | premium debit/credit + fee debit/credit + intermediate account state events |
| **`RebateRecorded`** (topic `0x44de92d7…4145f`) | — | — | **`0`** | **no rebate path fired** ✓ |
| `OwnershipTransferred` (topic `0x8be0079c…6457e0`) | — | — | `0` | no governance mutation ✓ |

## 7. NEW_OME / governance safety (post-broadcast)

| Subject | Value |
|---|---|
| `NEW_OME.owner` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (TIMELOCK) ✓ |
| `NEW_OME.guardian` | `0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46` (OPS_MULTISIG) ✓ |
| `NEW_OME.paused` | `false` ✓ |
| `NEW_OME.isExecutor(BE)` | `true` ✓ |
| `NEW_OME.isExecutor(DEPLOYER)` | `false` ✓ |
| `PFV.owner` | TIMELOCK ✓ |
| `NEW_FM_V2.owner` | TIMELOCK ✓ |
| `RG.feesManager` | NEW_FM_V2 ✓ |

No governance state changed by the smoke.

## 8. R5 invariants

| Invariant | Value | Verdict |
|---|---|---|
| `CV(PFV,mUSDC) − feeBalance − rebateReserve` | `3 028 − 3 028 − 0 = 0` | ✓ drift preserved |
| `PFV.rebateReserve(mUSDC)` | `0` | ✓ unchanged |
| `NEW_FM_V2.rebateBudget(mUSDC)` | `999 947` | ✓ unchanged |

## 9. Backend log safety scan

Backend log `/tmp/deopt_v2_backend.log` (271 624 bytes at close):

| Pattern | Hits | Verdict |
|---|---|---|
| `tx_hash` | `0` | ✓ no in-log tx_hash leakage (API responses carry it; logs do not) |
| unique `0x[64-hex]` | `0` | ✓ no PK-shape, no hash leakage |
| `eth_sendRawTransaction` / `send_raw_transaction` | `0 / 0` | ✓ |
| `InsufficientRebateReserve` | `0` | ✓ |
| `NotAuthorized` | `0` | ✓ |
| `InvalidSignature` | `0` | ✓ |
| `broadcast.*succeeded` / `submitted.*tx` | `0 / 0` | ✓ |
| non-PerpTrade ERROR/WARN | `0 unique` | ✓ |
| PerpTrade legacy noise (non-blocking; classified in FX-Q1-C-VERIFY) | continuing ~1/s | filterable, ignore |

## 10. Gas economics

| Component | Value |
|---|---|
| estimated_gas | `938_057` |
| safety bps | `12_500` (×1.25) |
| required_gas | `1_172_571` |
| broadcast_gas_limit | `1_500_000` (from `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000`) |
| `gas_check_status` | `ok` ✓ |
| actual `gasUsed` | `907_100` (3.3 % under estimate) |
| effectiveGasPrice | `6_000_000` wei (0.006 gwei; Base Sepolia base fee) |
| L2 exec cost | `5.443e12` wei |
| L1 data fee | `6.090e11` wei |
| Total per-tx cost | `6.052e12` wei |
| BE.balance headroom | `~626` more tx at this rate |

## 11. Operational observations (not blockers)

1. **Backend confirmation worker disabled.** `OPTION_CONFIRMATION_WORKER_ENABLED` defaults to `false` in env. The backend intent stays at `broadcast_submitted` until the worker is enabled (or until manual confirmation). On-chain receipt is authoritative; chain-side settlement is complete and irreversible. Enabling the worker (`OPTION_CONFIRMATION_WORKER_ENABLED=true`) closes the loop for future smokes; out-of-scope for this milestone.
2. **GET projection nulls.** Backend `/options/execution-intents/<id>` returns `submitted: null, confirmed: null` in the JSON projection even though the persisted DB row carries valid booleans. Same B5-known shape quirk; not a chain-side concern.
3. **Perp dry-run runner orphan row.** `src/execution/runner.rs` continues to log `execution intent is missing PerpTrade metadata: buyer_is_maker` for an orphan row in `execution_intents`. Cannot broadcast (hard-stop at `executor.rs:54-58`). Filterable; non-blocking; classified in FX-Q1-C-VERIFY-AFTER-FLAG-FLIP.

## 12. Validations (independent re-verify)

```
chain_id (RPC)                     : 84532 ✓
receipt.status                     : 0x1 ✓
receipt.from                       : BACKEND_EXECUTOR ✓
receipt.to                         : NEW_OME ✓
receipt.blockNumber                : 42571249 ✓
receipt.gasUsed                    : 907100 (under cap) ✓
BE.code                            : 0x ✓
BE.nonce                           : 1 ✓ (0 → 1 exact)
BE.balance delta                   : −6.052e12 wei ✓
PFV.feeBalance delta               : +3000 (matches 50+250 ppm of 10_000_000) ✓
PFV.rebateReserve                  : 0 ✓
CV(PFV) delta                      : +3000 ✓
BUYER CV delta                     : −10_002_500 ✓
SELLER CV delta                    : +9_999_500 ✓
conservation                       : 0 ✓
R5 drift                           : 0 ✓
NEW_OME.owner/guardian/paused/exec : unchanged ✓
PFV.owner / NEW_FM_V2.owner        : TIMELOCK ✓
RG.feesManager                     : NEW_FM_V2 ✓
OptionTradeExecuted on NEW_OME     : 1 ✓
FM_V2 FeeCharged events            : 2 ✓
PFV.onFeeCharged hook acks         : 2 ✓
RebateRecorded                     : 0 ✓
OwnershipTransferred               : 0 ✓
log canary (7 patterns)            : 0/0/0/0/0/0/0 ✓
unique 0x[64hex] in logs           : 0 ✓
second broadcast attempts          : 0 ✓
chain mutations by verifier        : 0 (read-only) ✓
.env edits by verifier             : 0 ✓
secrets printed                    : 0 ✓
GOV-G / reserve / governance       : not touched ✓
RFQ live smoke                     : not attempted ✓
mainnet                            : not touched ✓
```

## 13. Next milestone (recommended; operator-authorised separately)

1. **`OPTION_CONFIRMATION_WORKER_ENABLE`** (optional) — enable backend confirmation worker so subsequent smokes auto-transition `broadcast_submitted → broadcast_confirmed`.
2. **`FIRST_LIVE_SMOKE-RFQ-PRECHECK-SEPOLIA-FEE-ONLY`** — analogous packet for the RFQ flow; signer surface validated in B5 dry-run, fee profile (tier 0 OPTION) is the same fee-only `(50, 250)`, RFQ discount is `(0, 0)`, so same fee-only invariant applies.
3. **V2G-GOV-G** (Timelock cleanup) — independent track, unaffected by FX-Q1.
4. Optional housekeeping:
   - DB cleanup of orphan `execution_intents` Pending row to silence PerpTrade noise (operator-only).
   - Top-up BACKEND_EXECUTOR if planning a sustained smoke cadence (current headroom ≈ 626 tx; not pressing).
   - Archive this doc + the FX-Q1-B5 run log alongside `FX_Q1_C_OPERATOR_DECISION_PRE_FLIP.md` as the FX-Q1 closure bundle.

## 14. Provenance

- Verifier: Claude (Opus 4.7) acting under FIRST_LIVE_SMOKE-V2-POST-CLOSE-VERIFY task.
- All probes were read-only `eth_call` / `eth_getCode` / `eth_getBalance` / `eth_getTransactionCount` / `eth_getTransactionReceipt`.
- No transaction was broadcast by the verifier. No `.env` edit. No source patch. No DB mutation.
- No private key, RPC URL, DATABASE_URL, or admin token appears in this document.

**End of result document.**
