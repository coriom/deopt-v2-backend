# FX-Q1-B5 fast-path run log

**Date:** 2026-06-07 (UTC ≈ session timestamps below).
**Authorization:** operator-issued "testnet fast path" — agent executed directly inside the local repos and terminal context with testnet keys + Base Sepolia RPC. No mainnet. No live broadcast.

**End state:** both orderbook and RFQ dry-run paths green. Backend remains dry-run / broadcast-off. R5 invariants intact.

---

## 0. Inputs

| Field | Value |
|---|---|
| chain id | 84532 (Base Sepolia) |
| backend | `127.0.0.1:8080` (PID restarted from 29512 → 46378 once mid-run) |
| BACKEND_EXECUTOR | `0x295005fd4F311e6691F008D57d32FCFEde844518` |
| BUYER | `0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC` |
| SELLER | `0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88` |
| NEW_OME | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` |
| mUSDC | `0x6eAe407f5640B006faC9965182e238582A3B412E` |
| CV | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` |
| PFV | `0x7C0a3B6feBd5BFFc164f37738299AeB453181886` |
| NEW_FM_V2 | `0xF6626177f3B85cc3239667Cc53C04A8007652944` |
| RG | `0x7918Ea95c2791B6b587fF02AE481FA52403877A0` |
| OPTION_SERIES_ID | `0x8b34d095ebfb300f21868dea4a0ff5e1d6f8ebd5463facaa8bcbc6075df50e6d` (active; ETH call, strike $3000, expiry 2030-01-01) |

Keys loaded from `wallet.txt` (lines 25/26 `BUYER_PRIVATE_KEY=…` and `SELLER_PRIVATE_KEY=…`). Derived addresses verified match expected BUYER/SELLER. Persisted to `/tmp/fast_path_env.sh` mode 0600 for subsequent shell invocations. **No key value ever written to stdout, logs, or this doc.**

---

## 1. Pre-flight gates (all green)

- `chain_id = 84532` ✓
- `mUSDC.owner() = DEPLOYER` ✓ (mint capacity)
- `mUSDC.decimals() = 6` ✓
- `CV.launchActiveCollateral(mUSDC) = true` ✓
- `CV.depositsPaused() = false` ✓
- `NEW_OME.isExecutor(BE) = true`, `isExecutor(DEPLOYER) = false`, `owner = Timelock`, `guardian = OPS_MULTISIG`, `paused = false` ✓
- `/executor/status`: `dryRun=true`, `realBroadcastEnabled=false`, `broadcastEnabled=false`, `simulationEnabled=true`, `rpcConfigured=true`, `persistenceRequired=true` ✓

---

## 2. Funding (PrepareSmokeV1Accounts.s.sol)

Initial fund: 20_000_000 base units per side (per FX-Q1-B5E). Then escalated:

| Step | Action | Result |
|---|---|---|
| First broadcast (B5E) | 20_000_000 mUSDC per side | `BUYER/SELLER CV.balance = 20_000_000` |
| Margin-fail diagnostic | orderbook simulate reverted `MarginRequirementBreached(seller)` selector `0x6f003885` (with seller address as arg) — 20 mUSDC insufficient for option-write margin | escalation needed |
| Second broadcast (this fast path) | top-up 100_000_000_000 mUSDC per side (`PERP_SMOKE_FUND_USDC_AMOUNT_NATIVE=100000000000`, `PERP_SMOKE_FUND_ETH_WEI=0`) | `BUYER/SELLER CV.balance = 100_020_000_000` (~ $100k mUSDC ea) |

DEPLOYER ETH balance at top-up: 0.00192 ETH. Gas used by 6-tx top-up: ~ 0.0000053 ETH (well within budget).

---

## 3. Orderbook dry-run (B5C)

### Order pair (from `/tmp/dry_run_orderbook_{seller,buyer}.filled.json`, refreshed `deadline_ms = 1780860287485`)

| Step | Field | Value |
|---|---|---|
| A1 SELLER POST | `order_id` | `b332b8a9-0c4f-4858-a1fa-530a554dd646` |
| | `status` | `open` |
| A2 BUYER POST | `order_id` | `f4559e5f-56d1-4d29-afbc-e8c73783f256` |
| | `status` | `filled` |
| | `fill_id` | `0023a175-a0df-4a94-a91b-ebf8f7328bba` |
| C INTENT | `intent_id` | `5aacf70d-9268-4e57-80cc-3a86ce0cd775` |
| | `source_type` | `option_orderbook_fill` |
| | `series_id` | matches |
| D PAYLOAD | `digest` | `0xb7866983616f53c8ab3dc803d1307393195c3765e3d8e6b8ea1984d8539d138b` |
| | `chainId` | `84532` |
| | `verifyingContract` | `NEW_OME` |

### Signing (`sign_option_execution_intent` CLI; `--payload-file` per source)

```
E1 buyer  signer_address = 0x394291a05d3df2d1d8bfcbc571dad773ac7077cc   sig_len = 132 ✓
E2 seller signer_address = 0xb1f1ae6cb0d154afe9503c3b0790adef0851fd88   sig_len = 132 ✓
```

### F submit signatures → `status: calldata_ready` ✓

### G simulate (after $100k top-up)

```json
{
  "intent_id": "5aacf70d-9268-4e57-80cc-3a86ce0cd775",
  "simulation_status": "simulation_ok",
  "block_number": 42547171,
  "error": null,
  "revert_data": null,
  "revert_selector": null,
  "submitted": false,
  "confirmed": false
}
```

### H GET simulation → same `simulation_ok` ✓

### Hard-gate scan (all green)
- no `tx_hash` ✓
- no `eth_sendRawTransaction` / `send_raw_transaction` ✓
- no `InsufficientRebateReserve` ✓
- no `NotAuthorized` ✓
- no `InvalidSignature` ✓
- no `BadNonce` ✓
- no `MarginRequirementBreached` ✓

---

## 4. RFQ dry-run (B5D)

### Config changes needed (testnet-only)

Operator backend `.env` was missing `OPTION_RFQ_ENABLED`. Appended:
```
OPTION_RFQ_ENABLED=true
OPTION_RFQ_REQUIRE_PERSISTENCE=true
```
Backend restarted cleanly (SIGTERM → 2s shutdown → relaunch background; orderbook intent survived persistence — re-checked `simulation_status=simulation_ok` post-restart).

Fixture TTLs adjusted to fit defaults (`OPTION_RFQ_MAX_TTL_MS=30000`, `OPTION_RFQ_MAX_QUOTE_TTL_MS=10000`):
- `/tmp/dry_run_rfq.filled.json`       `ttl_ms = 60000 → 10000`
- `/tmp/dry_run_rfq_quote.filled.json` `quote_ttl_ms = 30000 → 5000`

### Flow

| Step | Field | Value |
|---|---|---|
| B1 RFQ create | `option_rfq_id` | `1e70e023-7e29-4529-b886-f1c3f38a05da` |
| | `status` | `open` |
| B2 quote | `quote_id` | `a0c43cf0-1d66-454a-9c27-d5d1ec9a250f` |
| | `signature_status` | `not_required` (mode `disabled`) |
| B3 accept (empty `{}`) | `quote_status` | `accepted` |
| | `fill_id` | `d31dc9a7-146d-4767-bbdc-ea9dd4d3349e` |
| | warning | `MM session fx-q1-b3-rfq-session is not connected` (harmless; off-band MM notification only) |
| C INTENT | `intent_id` | `dc3ed941-3503-405f-85fd-731489329428` |
| | `source_type` | `option_rfq_fill` |
| D PAYLOAD | `digest` | `0x070054294fef3d418bffde0e3dba4829351a02f9b6d169d320b8584408929fcb` |

### Signing
```
E1 buyer  signer_address = 0x394291a05d3df2d1d8bfcbc571dad773ac7077cc   sig_len = 132 ✓
E2 seller signer_address = 0xb1f1ae6cb0d154afe9503c3b0790adef0851fd88   sig_len = 132 ✓
```

### F → G → H
```
status: calldata_ready
/simulate  HTTP 200  simulation_status = simulation_ok  block 42547253
/simulation HTTP 200  same
submitted: false ; confirmed: false
```

Hard-gate scan all green (same set as §3).

---

## 5. Backend safety + chain invariants (post-flow)

```
/executor/status:
  dryRun=true, realBroadcastEnabled=false, broadcastEnabled=false,
  simulationEnabled=true, executionEnabled=false, persistenceRequired=true

Chain:
  NEW_OME.isExecutor(BACKEND_EXECUTOR) = true
  NEW_OME.isExecutor(DEPLOYER)         = false
  NEW_OME.owner                         = Timelock
  NEW_OME.guardian                      = OPS_MULTISIG
  NEW_OME.paused                        = false
  BUYER  CV.balance(mUSDC)              = 100_020_000_000
  SELLER CV.balance(mUSDC)              = 100_020_000_000

R5/PFV invariants:
  PFV.owner                  = Timelock
  PFV.feeBalance(mUSDC)      = 28
  PFV.rebateReserve(mUSDC)   = 0
  CV.balances(PFV, mUSDC)    = 28
  drift = CV − fee − reserve = 0
  NEW_FM_V2.owner            = Timelock
  NEW_FM_V2.rebateBudget(mUSDC) = 999_947
  RG.feesManager             = NEW_FM_V2
```

No on-chain mutation of governance / fee-routing / executor surfaces. Buyer/seller funding is the only change to chain state, exactly per the scoped allow-list.

---

## 6. Forbidden-surface check
- No NEW_OME governance call.
- No FM-V2 / PFV / Timelock / RG mutation.
- No ownership / guardian / proposer / executor mutation.
- No reserve allocation.
- No live broadcast (both broadcast flags off; verified pre/post each phase).
- No mainnet.
- No private key, admin token, RPC API key, or `DATABASE_URL` printed to any artifact.

---

## 7. Files touched

| Path | Change |
|---|---|
| `~/DEOPT/deopt-v2-backend/.env` | appended `OPTION_RFQ_ENABLED=true` + `OPTION_RFQ_REQUIRE_PERSISTENCE=true` (testnet only) |
| `/tmp/fast_path_env.sh` | created (mode 0600) — `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` exports for subsequent bash invocations |
| `/tmp/dry_run_rfq.filled.json` | `ttl_ms` 60000 → 10000 |
| `/tmp/dry_run_rfq_quote.filled.json` | `quote_ttl_ms` 30000 → 5000 |
| `/tmp/tmp.x1nyhVe8rd/intent_payload.json` | orderbook intent payload (fetched) |
| `/tmp/tmp.x1nyhVe8rd/{buyer,seller}_sig.json` | orderbook signatures |
| `/tmp/tmp.x1nyhVe8rd/sim_{g,h}.json` | orderbook /simulate + /simulation responses |
| `/tmp/tmp.x1nyhVe8rd/rfq_payload.json` | RFQ intent payload |
| `/tmp/tmp.x1nyhVe8rd/rfq_{buyer,seller}_sig.json` | RFQ signatures |
| `/tmp/tmp.x1nyhVe8rd/rfq_ids.txt` | rfq id / quote id / fill id |
| `/tmp/deopt_v2_backend.log` | backend stdout/stderr since restart |
| `~/DEOPT/deopt-v2-sol/broadcast/PrepareSmokeV1Accounts.s.sol/84532/run-latest.json` | tx hashes for top-up broadcast |
| `~/DEOPT/deopt-v2-backend/docs/FX_Q1_B5_FAST_PATH_RUN_LOG.md` | **this doc** |

No git commits made. No source touched. No `.env.example` changes.

---

## 8. Blockers
- **None.** Both dry-runs green.

## 9. Next milestone

**FX-Q1-B fully closed** as far as the simulate path is concerned.

Operator may now:
1. Decide whether to proceed to FX-Q1-C live-broadcast flag flip (re-verify the PRE-1..PRE-5 blockers from `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`).
2. Or defer FX-Q1-C and run V2G-GOV-G first (independent chain-side track; earliest broadcast 2026-06-07 ~15:45 UTC per dependency graph).

Either order is acceptable per the dependency graph.

Per scope, the agent did NOT flip live-broadcast flags, did NOT authorise smoke, did NOT touch governance, did NOT mutate fee routing. All such actions remain operator decisions.

---

## 10. Closure row — FIRST LIVE OPTION-EXECUTION SMOKE V2 (Base Sepolia)

- **Date:** 2026-06-08
- **Milestone:** FIRST_LIVE_SMOKE-EXEC-V2-SEPOLIA-FEE-ONLY (chain-side fee-only, orderbook path)
- **tx_hash:** `0xb2379a462e5aa82ea1d7e62cf4ae38c7143a41e8ea42d6c54b7ba298c061e800`
- **block:** `42_571_249`
- **receipt.status:** `1` (success)
- **from / to:** `BACKEND_EXECUTOR` (`0x295005fd…4518`) → `NEW_OME` (`0x5a5EBF9A…70f6`)
- **INTENT_ID:** `f9e765c2-cc9b-4863-918c-a4db04867d46`
- **gasUsed / cap:** `907_100` / `1_500_000`
- **R5 drift:** `0` pre / `0` post
- **PFV.feeBalance(mUSDC) delta:** `+3_000` (50 ppm maker + 250 ppm taker × 10_000_000 premium)
- **BUYER / SELLER CV delta (mUSDC):** `−10_002_500` / `+9_999_500` (zero-sum with PFV)
- **Rebate path:** not fired (`PFV.rebateReserve(mUSDC) = 0` preserved)
- **Pre-broadcast gas-cap remediation:** `OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1_500_000` added per `FIRST_LIVE_SMOKE-GAS-CAP-REMEDIATION-PACKET`
- **Result document:** `FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md`
- **Independent re-verify:** GREEN via `FIRST_LIVE_SMOKE-V2-POST-CLOSE-VERIFY`

---

## 11. Closure row — FIRST LIVE OPTION-EXECUTION SMOKE — RFQ PATH (Base Sepolia)

- **Date:** 2026-06-08
- **Milestone:** FIRST_LIVE_SMOKE-EXEC-RFQ-SEPOLIA-FEE-ONLY (and CLOSEOUT-VERIFY)
- **tx_hash:** `0x8538066ce0a10ede63f9e4c66161be8efdcd0edf6a63d176af0967b4bde95326`
- **block:** `42 581 402`
- **receipt.status:** `1` (success)
- **from / to:** `BACKEND_EXECUTOR` (`0x295005fd…4518`) → `NEW_OME` (`0x5a5EBF9A…70f6`)
- **INTENT_ID:** `95516dbd-a68c-41eb-869f-e6790d9091f2`
- **RFQ_ID / QUOTE_ID / FILL_ID:** `2492cc2f-…` / `6721538a-…` / `27b89001-…`
- **gasUsed / cap:** `695 618` / `1 500 000`
- **R5 drift:** `0` pre / `0` post (cumulative since baseline preserved)
- **PFV.feeBalance(mUSDC) delta (this trade):** `+3 000`
- **PFV.feeBalance(mUSDC) cumulative since baseline:** `+6 000` (= `28 → 6 028`; matches 2 trades × 300 ppm of $10)
- **BUYER / SELLER CV delta (this trade):** `−10 002 500` / `+9 999 500` (zero-sum with PFV)
- **NEW_OME.nonces(BUYER) / (SELLER) delta:** `1 → 2 / 1 → 2`
- **BE.nonce delta:** `1 → 2` (cumulative since baseline: `0 → 2`)
- **Rebate path:** not fired (`PFV.rebateReserve(mUSDC) = 0` preserved)
- **Backend confirmation worker:** auto-transitioned intent to `broadcast_confirmed` in ≤2 s (Phase B of POST_GOV_G_OPS_CLEANUP)
- **Nonce sync:** load-bearing (intent created with `(1, 1)` from chain; without `OPTION_NONCE_SYNC_ENABLED=true` would have reverted `BadNonce()` as in two prior attempts)
- **Result document:** `FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md`
- **Independent re-verify:** GREEN via `FIRST_LIVE_SMOKE-RFQ-CLOSEOUT-VERIFY`
