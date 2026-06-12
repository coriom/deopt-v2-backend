# E2E-SEPOLIA-LIVE-BROADCAST-RETRY — Next Task Brief

> **APPROVAL CONSUMED — SUCCESS (2026-06-12).** Operator approval
> line "I approve one Base Sepolia test broadcast retry for this
> run." consumed by `executeTrade` tx
> `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`
> at block `42750521`, **status `1`**, gas `683_044`. Nonces
> advanced `2 → 3`; positions ±1; `OptionTradeExecuted` event
> captured; vault balances reconciled with fees. M-P5 Phase B
> COMPLETE. Full record in `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`.
> The rest of this doc is preserved for historical reference.


**Date written:** 2026-06-12
**Origin:** `SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md` (BS-6 CLOSED via Path A).
**Target:** **one** operator-approved Base Sepolia option-execution broadcast against the **correctly-wired** MatchingEngine `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`.
**Posture:** **Approval-gated retry. NEVER auto-broadcast. NEVER mainnet. NEVER Safe tx. NEVER AWS / KMS. NEVER production `.env` edit. NEVER more than one option execution broadcast under one approval line.**

> **This task is NOT executed by the calling milestone. The harness MUST require the literal operator approval line below before any `cast send` for `executeTrade`.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve one Base Sepolia test broadcast retry for this run."

Properties:
* Authorises **exactly one** `executeTrade(...)` broadcast on Base Sepolia chain id `84532`.
* Plus the **immediately-required oracle refresh** transactions (`MockPriceSource.setPrice` on primary + secondary; budget ≤ 4 to allow one recovery refresh).
* Expires 4 hours after the approval is received.
* Does NOT authorise any other action (no further deploy, no governance, no second executeTrade, no mainnet).
* Is NOT interchangeable with the original "I approve one Base Sepolia test broadcast for this run." line; that was consumed by tx `0xb4e7984f…`.

---

## 2. Hard preconditions

| # | Precondition | Verifying check |
|---|---|---|
| P1 | Approval line (§1) present verbatim | grep |
| P2 | `cast chain-id` returns `84532` | required first call |
| P3 | Private file at `~/DEOPT/private/operator-private/sepolia.inputs.private.env` PRESENT, mode `600`, outside any git repo | `stat`, `git check-ignore` |
| P4 | Private file `OPTION_MATCHING_ENGINE = 0x5a5EBF9A9CCd7c012518569DE8283982982670f6` (post-retarget) | sourced subshell readback |
| P5 | Private file `OPTION_MARGIN_ENGINE = 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` (post-retarget) | sourced subshell readback |
| P6 | **`EXECUTOR_PRIVATE_KEY` PRESENT** in the private file or process env. `cast wallet address --private-key $EXECUTOR_PRIVATE_KEY` must equal `0x295005fd4F311e6691F008D57d32FCFEde844518` | derive + compare |
| P7 | `isExecutor(EXECUTOR_ADDRESS)` on the new ME returns `true` | view |
| P8 | `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` derive to the respective `BUYER_ADDRESS` / `SELLER_ADDRESS` | derive + compare |
| P9 | Bidirectional wiring: `ME.marginEngine() == MarginEngine` AND `MarginEngine.matchingEngine() == ME` | 2 view calls |
| P10 | `previewTradeValidity(t)` returns 5/5 `true` with `buyerNonce = nonces(BUYER)` and `sellerNonce = nonces(SELLER)` on the new ME (both currently `2`) | view |
| P11 | `getPriceSafe(under, settle)` returns nonzero + `ok == true` **immediately** before the `executeTrade` broadcast | view |
| P12 | `.env` (`/home/corio/DEOPT/deopt-v2-backend/.env`) NOT touched | `stat -c '%y'` |

If ANY precondition fails → **STOP** before any state-mutating call.

---

## 3. Hard stops

The harness MUST stop, write the partial-result doc, and exit non-zero if any of:

* `cast chain-id` returns anything other than `84532`.
* Any RPC URL or private key would be written to a public log.
* Any `cast send` / `forge create` / `forge script --broadcast` is invoked before the approval line is matched verbatim.
* `cast wallet address --private-key $EXECUTOR_PRIVATE_KEY` does NOT equal `0x295005fd4F311e6691F008D57d32FCFEde844518`.
* `isExecutor(EXECUTOR_ADDRESS)` on the new ME returns `false`.
* `previewTradeValidity` returns any `false` flag.
* `getPriceSafe` is stale (`ok == false`) at the moment of `executeTrade` invocation.
* A tx hash for `executeTrade` is produced (do NOT retry under the same approval).
* Any error envelope contains a mainnet RPC URL substring.
* Mainnet chain id (`8453`) appears anywhere.

On hard stop: write `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_PARTIAL_RESULT.md` with the failing precondition + step. Do NOT include raw RPC values, private keys, or balances.

---

## 4. Execution sequence

```
4.0 Preflight (P1 .. P12)
4.1 JIT oracle refresh — 2 cast send setPrice (primary + secondary)
4.2 Final pre-broadcast getPriceSafe sanity (must be ok=true)
4.3 Single executeTrade broadcast on new ME — exactly 1 tx
4.4 Receipt analysis + reconciliation
4.5 Docs + RUN_STATE
4.6 Validations
```

### 4.1 JIT oracle refresh

Primary `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc`
Secondary `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517`

```bash
cast send 0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc \
  "setPrice(uint256)" 300000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --private-key "$OWNER_PRIVATE_KEY" \
  --confirmations 1

cast send 0x2103a84C0CAB9cf7680d602C8931FaDeD7064517 \
  "setPrice(uint256)" 300000000000 \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --private-key "$OWNER_PRIVATE_KEY" \
  --confirmations 1
```

`OWNER_PRIVATE_KEY` is correct for these — it owns all 4 MockPriceSource contracts (verified during the setup-fixes pack).

### 4.2 Pre-broadcast getPriceSafe

```bash
cast call "$OPTION_ORACLE_ROUTER" \
  "getPriceSafe(address,address)(uint256,uint256,bool)" \
  0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02 \
  0x6eAe407f5640B006faC9965182e238582A3B412E \
  --rpc-url "$BASE_SEPOLIA_RPC_URL"
# Required: price > 0; ok == true
```

### 4.3 Single executeTrade broadcast

Trade tuple (series #0, qty=1, premium=1 mUSDC native, **buyerNonce=2, sellerNonce=2**, fresh deadline):

```bash
TUPLE="(INTENT_ID,$BUYER_ADDRESS,$SELLER_ADDRESS,$SERIES_ID,$UNDERLYING,$SETTLEMENT,$EXPIRY,$STRIKE_1E8,true,$CONTRACT_SIZE_1E8,1,1000000,false,2,2,$DEADLINE)"

# Re-derive digest against the NEW ME's domain separator:
DIGEST=$(cast call "$OPTION_MATCHING_ENGINE" \
  "previewTradeDigest(...)" "$TUPLE" --rpc-url "$BASE_SEPOLIA_RPC_URL")

# Re-sign with buyer + seller:
BSIG=$(cast wallet sign --no-hash --private-key "$BUYER_PRIVATE_KEY"  "$DIGEST")
SSIG=$(cast wallet sign --no-hash --private-key "$SELLER_PRIVATE_KEY" "$DIGEST")

# Single broadcast from the executor key (NOT from owner):
cast send "$OPTION_MATCHING_ENGINE" \
  "executeTrade((bytes32,address,address,uint256,address,address,uint64,uint64,bool,uint128,uint128,uint128,bool,uint256,uint256,uint256),bytes,bytes)" \
  "$TUPLE" "$BSIG" "$SSIG" \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" --private-key "$EXECUTOR_PRIVATE_KEY" \
  --confirmations 1 --gas-limit 1500000
```

* **Broadcaster MUST be `EXECUTOR_PRIVATE_KEY`.** `OWNER_PRIVATE_KEY` is NOT `isExecutor==true` on the new ME and will revert `NotAuthorized()`.
* If the tx hash is produced (any status), do NOT submit a second `executeTrade`.

### 4.4 Receipt analysis

* Expect `status = 1`, `OptionTradeExecuted` event in the receipt logs.
* If `status = 0`: replay via `cast run --quick` to extract revert reason; document; do not retry under the same approval.
* Confirm `nonces(BUYER)` and `nonces(SELLER)` both incremented from `2` to `3`.

### 4.5 Docs + RUN_STATE

Create `docs/E2E_SEPOLIA_LIVE_BROADCAST_RESULT_V2.md` (or update the existing result doc with a §17 "retry" section). Flip the live gate to "BROADCAST CONFIRMED" if `status=1`. Update RUN_STATE with closure paragraph.

### 4.6 Validations

* `git diff --check` clean
* sensitive-string scan: zero hits
* `.env` modified? NO
* private file printed? NO
* RPC URL printed? NO
* private key printed? NO
* tx count ≤ 5 (4 oracle prep budget + 1 executeTrade)

---

## 5. Scope — what the retry DOES

* 1 setPrice on primary MockPriceSource
* 1 setPrice on secondary MockPriceSource
* 1 `executeTrade` on the **new** ME `0x5a5EBF9A…`
* Optionally 1–2 recovery setPrice if the freshness window slips (cap 4 total prep txs)
* Doc + RUN_STATE updates
* No source-code change

Expected total broadcast tx count: **3 nominal**, **≤ 5 worst case**.

---

## 6. Scope — what the retry DOES NOT do

* Does NOT touch mainnet.
* Does NOT call governance / `setMatchingEngine` / `setExecutor` / `setOwner`.
* Does NOT call `forge create` / `forge script --broadcast`.
* Does NOT engage backend signer / AWS / KMS / Safe tx.
* Does NOT edit `.env`.
* Does NOT touch the stale ME `0xf2D1D85…` or stale MarginEngine `0x287Cef47…` (their state is documented for historical reference only).

---

## 7. Forbidden (whole retry)

* No mainnet (chain id `8453`).
* No Safe tx.
* No AWS / KMS creation.
* No production `.env` edit.
* No `.env.sepolia` commit.
* No private key in any log or doc.
* No exact balance / allowance / price in any public doc.
* No second `executeTrade` under the same approval line.
* No skipping of any precheck.
* No claim "mainnet-ready" or "audited".

---

## 8. Acceptance criteria

The retry closes successfully when ALL of:

* Approval line consumed (single `executeTrade` confirmed).
* `executeTrade` tx receipt `status = 1`.
* `OptionTradeExecuted` event present in the receipt logs.
* `nonces(BUYER)` / `nonces(SELLER)` on new ME both = `3`.
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` banner flipped to "BROADCAST CONFIRMED".
* No source code changed.
* No `.env` modified.
* Sensitive-string scan clean.
* `git diff --check` clean.

---

## 9. Cross-links

* `docs/SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md` (this milestone's predecessor)
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` (the original failed attempt)
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of retry-next-task brief.**
