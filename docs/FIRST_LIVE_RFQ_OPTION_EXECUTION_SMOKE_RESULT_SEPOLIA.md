# FIRST LIVE OPTION-EXECUTION SMOKE — RFQ PATH — Base Sepolia — RESULT

## 0. Identity

| Field | Value |
|---|---|
| Milestone | FIRST_LIVE_SMOKE-EXEC-RFQ-SEPOLIA-FEE-ONLY (and CLOSEOUT-VERIFY) |
| Chain | Base Sepolia (`84532`) |
| Verifier | independent re-fetch via DB + chain receipt + event topic decode |
| Result | **CONFIRMED ON-CHAIN AND BACKEND — SUCCESS** |
| Date (UTC) | 2026-06-08 |
| Scope | single fee-only RFQ execution; no rebate path; no mainnet |

---

## 1. Broadcast tx

| Field | Value |
|---|---|
| **tx_hash** | `0x8538066ce0a10ede63f9e4c66161be8efdcd0edf6a63d176af0967b4bde95326` |
| blockNumber | `42 581 402` |
| transactionIndex | `8` (9th tx in block) |
| status | `0x1` (success) |
| from | `0x295005fd4F311e6691F008D57d32FCFEde844518` (BACKEND_EXECUTOR) |
| to | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` (NEW_OME) |
| gasUsed | `695 618` (cheaper than orderbook 907 100 by ~211k; RFQ path skips a matching-loop hop) |
| effectiveGasPrice | `6 000 000` wei (0.006 gwei) |
| cumulativeGasUsed (block) | `2 353 422` |
| receipt_block_hash | `0xe7bce0ad…48f31` |
| BE.balance debit | `4 254 797 000 569` wei (~4.25e12, ~0.0000043 ETH) |
| logs.length | `23` |

---

## 2. Intent

| Field | Value |
|---|---|
| INTENT_ID | `95516dbd-a68c-41eb-869f-e6790d9091f2` |
| transaction_id | `1253bf91-f8cf-4a6d-b2dc-a70f821b2b45` |
| onchain_intent_id | `0xa093d2e39c4cdd4582bd69d0fe2a6c666478a042ff678eea24e0bf162fe6856a` |
| source_type | `option_rfq_fill` |
| source_id (FILL_ID) | `27b89001-0cc1-4a0e-939b-58df5bb6c29e` |
| RFQ_ID | `2492cc2f-f8b1-4841-ad7a-285813f66f7e` |
| QUOTE_ID | `6721538a-3c3e-4292-9072-0e9fe6bb45a2` |
| signing digest | `0x2e4ea0a0c65e80b01905dc2576cb7d06d57b9067f14a4126543b523934cbcac4` |
| domain.chainId | `84532` |
| domain.verifyingContract | NEW_OME |
| simulation block | `42 581 165` |
| simulation_status | `simulation_ok` |
| backend status (terminal) | `broadcast_confirmed` (auto-transition in ≤2 s via OPTION_CONFIRMATION_WORKER) |
| backend error | `null` |
| buyer_nonce | `1` (pulled live from chain via OPTION_NONCE_SYNC) |
| seller_nonce | `1` (pulled live from chain via OPTION_NONCE_SYNC) |

DB `option_execution_transactions` row (independent recovery):
- `tx_hash` = `0x8538066c…5326`
- `status` = `submitted`
- `confirmation_status` = `mined_success`
- `confirmed_block_number` = `42 581 402`
- `confirmed_at_ms` = `1 780 931 102 791`
- `receipt_status` = `1`
- `gas_used` = `695 618`
- `effective_gas_price` = `0x5b8d80` (= `6 000 000` wei)
- `gas_check_status` = `ok`

---

## 3. Trade economics

| Field | Value |
|---|---|
| series_id | `0x8b34d095…df50e6d` (ETH call, strike $3000, expiry 2030-01-01) |
| premium_per_contract_native | `10 000 000` mUSDC (= $10) |
| quantity_contracts | `1` |
| source_price_1e8 | `1 000 000 000` (= $10) |
| RFQ taker | BUYER |
| RFQ MM (quote signer) | SELLER |
| fee profile (tier 0 OPTION) | `(makerPpm=50, takerPpm=250)` |
| RFQ discount profile (tier 0 OPTION) | `(0, 0)` |
| effective maker / taker fee | $0.0005 / $0.0025 = 500 / 2 500 mUSDC native |

---

## 4. BACKEND_EXECUTOR state delta

| Field | Pre | Post | Delta |
|---|---|---|---|
| nonce | `1` | **`2`** | **+1** ✓ exactly one tx |
| balance (wei) | `3 793 948 403 441 928` | `3 789 693 606 441 359` | `−4 254 797 000 569` |
| code | `0x` | `0x` | EOA preserved |

Cumulative since post-flag-flip baseline (pre-orderbook smoke):
- BE.nonce: `0 → 2` (1 orderbook + 1 RFQ).
- BE.balance: `3.8e15 → 3.79e15` (~`10.3e12` wei spent on 2 broadcasts).
- Headroom: ~893 more option-execution tx at observed Base Sepolia gas pricing.

On-chain option nonces (NEW_OME):

| Account | Pre-RFQ | Post-RFQ | Delta |
|---|---|---|---|
| BUYER | `1` | `2` | +1 ✓ |
| SELLER | `1` | `2` | +1 ✓ |

---

## 5. Accounting reconciliation (mUSDC native, RFQ alone)

| Account | Pre | Post | Δ (this RFQ) | Expected | Match |
|---|---|---|---|---|---|
| BUYER `CV.balances(BUYER, mUSDC)` | `100 009 997 500` | `99 999 995 000` | **`−10 002 500`** | `−10 000 000` premium + `−2 500` taker fee | ✓ exact |
| SELLER `CV.balances(SELLER, mUSDC)` | `100 029 999 500` | `100 039 999 000` | **`+9 999 500`** | `+10 000 000` premium − `500` maker fee | ✓ exact |
| **PFV.feeBalance(mUSDC)** | `3 028` | `6 028` | **`+3 000`** | `500 + 2 500 = 3 000` | ✓ exact |
| PFV.rebateReserve(mUSDC) | `0` | `0` | `0` | unchanged | ✓ |
| CV(PFV, mUSDC) | `3 028` | `6 028` | `+3 000` | matches feeBalance | ✓ |
| FM.rebateBudget(mUSDC) | `999 947` | `999 947` | `0` | unchanged (no rebate fired) | ✓ |

**Conservation (RFQ alone):** `−10 002 500 + 9 999 500 + 3 000 = 0` ✓.

**Cumulative since pre-orderbook baseline (2 trades: orderbook + RFQ):**

| Account | Pre-orderbook baseline | Post-RFQ | Δ cumulative | Expected (2 × per-trade) |
|---|---|---|---|---|
| BUYER | `100 020 000 000` | `99 999 995 000` | `−20 005 000` | `−20 005 000` ✓ |
| SELLER | `100 020 000 000` | `100 039 999 000` | `+19 999 000` | `+19 999 000` ✓ |
| PFV.feeBalance | `28` | `6 028` | `+6 000` | `+6 000` ✓ |
| **Conservation** | — | — | `0` | ✓ perfect across both trades |

---

## 6. On-chain events (23 logs total)

| Emitter | Address | topic0 | Count | Interpretation |
|---|---|---|---|---|
| **NEW_OME** | `0x5a5e…70f6` | `0xb2387b9f…4cb808` | **1** | `OptionTradeExecuted(onchain_intent_id, buyer, seller, …)` — topic1 = `0xa093d2e3…6856a` ✓ matches intent's `onchain_intent_id`; topic2 = BUYER ✓; topic3 = SELLER ✓ |
| **NEW_FM_V2** | `0xf662…2944` | `0xbbbe34ce…7f043` | **2** | `FeeCharged` events (buyer + seller fee assessments) |
| **PFV** | `0x7c0a…1886` | `0x8a6211fc…7fa8` | **2** | `onFeeCharged` vault hook acks (one per FM event) |
| Position token | `0x506c…0d30` | mixed | (in 23 total) | option position mint events |
| CV | `0x0034…25d3` | mixed | (in 23 total) | premium debit/credit + fee debit/credit |
| **RebateRecorded** | topic `0x44de92d7…4145f` | **`0`** | ✓ no rebate path fired |
| **OwnershipTransferred** | topic `0x8be0079c…6457e0` | `0` | ✓ no governance mutation |

---

## 7. NEW_OME / governance safety (unchanged by smoke)

| Subject | Value |
|---|---|
| `NEW_OME.owner` | TIMELOCK (unchanged) |
| `NEW_OME.guardian` | OPS_MULTISIG (unchanged; from GOV-A-OME) |
| `NEW_OME.paused` | `false` |
| `NEW_OME.isExecutor(BE)` | `true` |
| `NEW_OME.isExecutor(DEPLOYER)` | `false` |
| `Timelock.owner` | OPS_MULTISIG (unchanged; from GOV-G G-4) |
| `Timelock.pendingOwner` | `0x0` |
| `Timelock.guardian` | OPS_MULTISIG |
| `Timelock.proposers(OPS_MULTISIG) / executors(OPS_MULTISIG)` | `true / true` |
| `Timelock.proposers(DEPLOYER) / executors(DEPLOYER)` | `false / false` (GOV-G G-5) |

---

## 8. R5 invariants

| Subject | Value | Verdict |
|---|---|---|
| `CV(PFV, mUSDC) − feeBalance − rebateReserve` | `6 028 − 6 028 − 0` = **`0`** | ✓ drift preserved |
| `PFV.rebateReserve(mUSDC)` | `0` | ✓ unchanged |
| `NEW_FM_V2.rebateBudget(mUSDC)` | `999 947` | ✓ unchanged (no rebate consumed) |
| `PFV.owner` | TIMELOCK | ✓ |
| `NEW_FM_V2.owner` | TIMELOCK | ✓ |
| `RG.feesManager` | NEW_FM_V2 | ✓ |

---

## 9. Backend log safety scan

| Pattern | Hits | Verdict |
|---|---|---|
| `tx_hash` | 0 | ✓ no in-log tx_hash leakage |
| unique `0x[64-hex]` strings in log | 0 | ✓ no PK-shape, no hash leakage |
| `eth_sendRawTransaction` / `send_raw_transaction` | 0 / 0 | ✓ |
| `broadcast.*succeeded` / `submitted.*tx` | 0 / 0 | ✓ |
| `InsufficientRebateReserve` | 0 | ✓ |
| `NotAuthorized` | 0 | ✓ |
| `InvalidSignature` | 0 | ✓ |
| `BadNonce` | 0 | ✓ |

Sole non-blocking WARN since restart: `option RFQ quote accepted notification failed ... MM session ... is not connected`. This is the harmless MM-gateway-notification warning (the MM session was never connected because `mm_gateway_enabled=false` on the testnet rehearsal). It does not affect chain settlement.

---

## 10. Gas economics (RFQ vs orderbook)

| Path | gasUsed | required_gas (at 1.25× safety) | cost @ this Sepolia rate |
|---|---|---|---|
| Orderbook (V2 EXEC) | 907 100 | 1 172 540 | ~6.05e12 wei |
| RFQ (this milestone) | **695 618** | **904 022** | ~4.25e12 wei |

RFQ path is **~211 k gas cheaper** than orderbook. Likely cause: the RFQ flow has a pre-matched taker/maker pair from the quote acceptance, so no on-chain matching loop is required; the executor just settles the agreed price.

---

## 11. Operational observations

1. **`/executor/transactions/<intent>` returned `[]`** — the same projection quirk seen with the orderbook smoke. The DB `option_execution_transactions` row is fully populated (`tx_hash`, `confirmation_status=mined_success`, `receipt_status=1`); the legacy `/executor/transactions` endpoint is a perp-scaffold projection that doesn't surface option-execution transactions. Not a chain-side concern; cosmetic only.
2. **Confirmation worker auto-transitioned the intent in ≤2 seconds** — the worker enabled in `POST_GOV_G_OPS_CLEANUP_BEFORE_RFQ_SMOKE` worked exactly as designed.
3. **Nonce sync (enabled in `RFQ_SMOKE_NONCE_SYNC_REMEDIATION`) was load-bearing** — without it, the broadcast would have reverted with `BadNonce()` as it did twice before.

---

## 12. Validations (independent re-verify)

```
chain_id (RPC)                            : 84532 ✓
DB tx_hash recovery                       : 0x8538066c…5326 from option_execution_transactions ✓
chain receipt re-fetch                    : status=0x1, from=BE, to=NEW_OME ✓
receipt.blockNumber                       : 42 581 402 ✓
receipt.gasUsed                           : 695 618 ✓
BE.code                                   : 0x ✓
BE.nonce                                  : 2 ✓ (1 → 2 exact)
BE.balance delta                          : −4.25e12 wei ✓
NEW_OME.nonces(BUYER) / (SELLER)          : 1→2 / 1→2 ✓
NEW_OME OptionTradeExecuted               : 1, topic1=onchain_intent_id, topic2=BUYER, topic3=SELLER ✓
FM_V2 FeeCharged                          : 2 ✓
PFV onFeeCharged                          : 2 ✓
RebateRecorded                            : 0 ✓
OwnershipTransferred                      : 0 ✓
PFV.feeBalance delta (RFQ alone)          : +3 000 (matches 50+250 ppm of 10M) ✓
PFV.feeBalance cumulative since baseline  : +6 000 (2 trades) ✓
PFV.rebateReserve                         : 0 ✓
CV(PFV) delta                             : +3 000 ✓
BUYER CV delta (RFQ alone)                : −10 002 500 ✓
SELLER CV delta (RFQ alone)               : +9 999 500 ✓
trade conservation (RFQ alone)            : 0 ✓
cumulative conservation (2 trades)        : 0 ✓
R5 drift pre & post                       : 0 / 0 ✓
NEW_OME state                             : owner/guardian/paused/exec all expected ✓
Timelock state                            : owner/pendingOwner/guardian/roles all expected ✓
PFV / NEW_FM_V2 / RG ownership            : TIMELOCK / TIMELOCK / FM_V2 ✓
intent terminal status                    : broadcast_confirmed ✓
confirmation worker auto-transition       : observed in ≤2s ✓
log canary (8 patterns)                   : 0/0/0/0/0/0/0/0 ✓
private-key-shape in log                  : 0 ✓
secrets printed                           : 0 ✓
chain mutations by agent                  : 0 (read-only verifier) ✓
broadcast attempts                        : 0 ✓
.env edits                                : 0 ✓
governance mutation                       : 0 ✓
mainnet                                   : not touched ✓
```

---

## 13. Provenance

- DB recovery: `psql "$DATABASE_URL" -c "SELECT * FROM option_execution_transactions WHERE intent_id = '95516dbd-…'"` — full row returned.
- Chain receipt: `cast receipt --json` against the RPC URL configured for the backend.
- Event topic0 cross-verification via `cast keccak`.
- Source-of-truth contracts: `src/matching/OptionMatchingEngine.sol`, `src/fees/FeesManagerV2.sol`, `src/finance/ProtocolFeeVault.sol`.
- No private key, RPC URL, DATABASE_URL, or admin token appears in this document.
- Verifier: Claude (Opus 4.7) under `FIRST_LIVE_SMOKE-RFQ-CLOSEOUT-VERIFY`.

**End of result document.**
