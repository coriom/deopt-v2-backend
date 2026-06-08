# RFQ-smoke backend nonce-sync remediation — RESULT

**Posture: CLOSED GREEN at 2026-06-08.** `OPTION_NONCE_SYNC_ENABLED=true`
applied via single-line `.env` append + graceful restart. Newly-created
RFQ intent now pulls on-chain `NEW_OME.nonces(account)` (1, 1) instead
of stale DB default (0, 0). Simulation green: `simulation_ok` at block
42 581 165. New armed intent `95516dbd-…` ready for separately-authorised
broadcast. **No chain mutation. No broadcast attempted. R5 drift = 0
preserved. Backend live-capable throughout.**

**Scope:** Base Sepolia (chain `84532`) only. No mainnet.

---

## 1. Root cause (recap)

Two prior RFQ precheck attempts halted at simulation with
`BadNonce()` revert (selector `0x4bd574ec` per
`src/matching/OptionMatchingEngine.sol:75`):

| Attempt | Intent | DB nonces | On-chain nonces | Result |
|---|---|---|---|---|
| 1 | `c4d719b5-…` | (0, 0) | (1, 1) | `simulation_failed` (terminal) |
| 2 | `ba26fc92-…` | (0, 0) | (1, 1) | `simulation_failed` (terminal) |

Reason: `OPTION_NONCE_SYNC_ENABLED` was unset (default `false`) per
`src/config/env.rs:346`. Per `src/options/service.rs:1949-1985`, the
intent-creation helper `option_execution_nonces` returns hard-coded
`(0, 0)` when nonce-sync is disabled, so every option-execution intent
created after the first live orderbook smoke (which incremented the
chain-side counter from 0 → 1) signed against stale nonces.

---

## 2. Source / config analysis

| File:line | Finding |
|---|---|
| `src/config/env.rs:345-351` | `OptionNonceSyncConfig` env keys: `OPTION_NONCE_SYNC_ENABLED` (default `false`), `OPTION_NONCE_SYNC_REQUIRE_RPC` (default `true`), `OPTION_NONCE_SYNC_STRICT` (default `true`). Backend uses `execution.rpc_url` + `options.matching_engine_address`. |
| `src/config/env.rs:467` | `option_nonce_sync.validate_startup()` runs at config load. |
| `src/nonce_sync/mod.rs:192-209` | `read_option_nonce_value` is the reader: `ensure_option_enabled` + `ensure_option_ready_for_read` + `parse_evm_address`, then calls `provider.option_matching_nonce(matching_engine_address, account)`. **Pure read.** |
| `src/options/service.rs:1949-1957` | `option_nonce_provider` returns `Some(HttpJsonRpcProvider)` only when enabled; else `None`. |
| `src/options/service.rs:1959-1985` | `option_execution_nonces` returns `(0, 0)` if disabled; else queries chain for both accounts. |
| `src/options/service.rs:1754-1789` | `create_option_rfq_execution_intent` calls `option_nonce_provider` then `option_execution_nonces`; the returned `(buyer_nonce, seller_nonce)` is baked into the persisted intent row and the EIP-712 digest. |
| `src/options/service.rs:1720-1746` | Same pattern for orderbook intent creation (`create_option_orderbook_execution_intent_with_nonce_provider`). |

Safety semantics:
- Only `eth_call` reads against `NEW_OME.nonces(address)`.
- No signing.
- No broadcast.
- No DB mutation outside the new intent row that would have been created anyway.

Minimum required env change: **single line** `OPTION_NONCE_SYNC_ENABLED=true`.
Defaults for `REQUIRE_RPC=true` and `STRICT=true` are correct.

---

## 3. Env remediation

| Step | Action |
|---|---|
| Backup | `.env` → `.env.bak.nonce_sync.20260608T145505Z` (mode 0600, 3 323 B) |
| Edit | Appended one line `OPTION_NONCE_SYNC_ENABLED=true` (with a header comment) |
| Diff vs backup | `+1` line (non-comment, non-blank). All other keys unchanged. |
| Restart | TERM `54421` (graceful exit < 1s) → nohup spawn `63259` → `/executor/status` ready in 1s |
| Post-restart `/executor/status` | 6/6 broadcast-capability flags green (unchanged) |
| Startup banner | shows `option_confirmation_worker_enabled=true`, `executor_dry_run=true`, no panic, no `is required` errors |
| Non-PerpTrade ERROR/WARN since restart | **0** |

---

## 4. Chain / governance safety (pre-fresh-RFQ)

| Subject | Value |
|---|---|
| chain_id | `84532` |
| Timelock.owner / pendingOwner | OPS_MULTISIG / `0x0` |
| Timelock.guardian | OPS_MULTISIG |
| DEPLOYER proposer / executor | `false / false` |
| NEW_OME.paused | `false` |
| NEW_OME.isExecutor(BE) / (DEPLOYER) | `true / false` |
| BE.balance | `3 793 948 403 441 928` wei (unchanged) |
| BE.nonce | `1` (unchanged) |
| R5 drift | `0` |
| `NEW_OME.nonces(BUYER)` | **`1`** |
| `NEW_OME.nonces(SELLER)` | **`1`** |

---

## 5. Fresh RFQ → simulation_ok

### 5.1 RFQ / quote / accept
```
RFQ_ID    : 2492cc2f-f8b1-4841-ad7a-285813f66f7e   (taker BUYER, side buy)
QUOTE_ID  : 6721538a-3c3e-4292-9072-0e9fe6bb45a2   (mm SELLER, sig_status not_required)
FILL_ID   : 27b89001-0cc1-4a0e-939b-58df5bb6c29e   (matched at 1_000_000_000)
```

### 5.2 Nonce-sync proof
```
INTENT_ID         : 95516dbd-a68c-41eb-869f-e6790d9091f2
intent.buyer_nonce  : "1"   ← matches on-chain NEW_OME.nonces(BUYER)=1   ✓
intent.seller_nonce : "1"   ← matches on-chain NEW_OME.nonces(SELLER)=1  ✓
```
**Nonce sync proven end-to-end.** The remediation works: backend now pulls
on-chain nonces at intent-creation time and bakes them into the signed
EIP-712 digest.

### 5.3 Signing payload
```
digest                : 0x2e4ea0a0c65e80b01905dc2576cb7d06d57b9067f14a4126543b523934cbcac4
domain.chainId        : 84532          ✓
domain.verifyingContract : 0x5a5EBF9A…70f6 = NEW_OME ✓
message.buyer / seller   : BUYER / SELLER ✓
```

### 5.4 Signature submission
```
buyer  signer  : 0x394291a0…7077cc  sig_len 132  ✓
seller signer  : 0xb1f1ae6c…1fd88   sig_len 132  ✓
POST signatures → status calldata_ready ✓
calldata selector : 0x031f77b3   length(hex) : 1674
```

### 5.5 Simulation
```
simulation_status : simulation_ok   ✓
block_number      : 42_581_165
error             : null
revert_data       : null
submitted         : false  ✓ no broadcast
```
GET `/simulation` returns the same body (idempotent).

---

## 6. Broadcast withheld

```
BE.nonce (post-precheck)     : 1   ← unchanged ✓ (no chain tx)
BE.balance                   : 3 793 948 403 441 928 wei (unchanged)
R5 drift                     : 0
NEW_OME.paused               : false
log canary (8 patterns)      : 0 hits each ✓
  tx_hash / eth_sendRawTransaction / send_raw_transaction
  broadcast.*succeeded / InsufficientRebateReserve
  NotAuthorized / InvalidSignature / BadNonce
```

The armed intent `95516dbd-…` is at `calldata_ready / simulation_ok`.
Per `validate_broadcast_intent` (`src/options/execution.rs:534-543`),
it is eligible for `POST /broadcast` in a separately-authorised task.
This task does **not** release the broadcast.

---

## 7. Files touched

| Path | Change |
|---|---|
| `deopt-v2-backend/.env` | `+1` line: `OPTION_NONCE_SYNC_ENABLED=true` |
| `deopt-v2-backend/.env.bak.nonce_sync.20260608T145505Z` | **CREATED** (mode 0600 backup) |
| `deopt-v2-backend/docs/RFQ_SMOKE_NONCE_SYNC_REMEDIATION.md` | **THIS FILE** (created) |
| Backend process | graceful restart |
| `option_rfqs` / `option_rfq_quotes` / `option_rfq_fills` / `option_execution_intents` tables | new rows for `2492cc2f-…` / `6721538a-…` / `27b89001-…` / `95516dbd-…` |

No chain mutation. No source patch. No broadcast.

---

## 8. Validations

```
chain_id                                  : 84532 ✓
governance safety                         : Timelock.owner=OPS_MULTISIG, DEPLOYER stripped ✓
NEW_OME.paused                            : false ✓
NEW_OME.isExecutor pair                   : BE=true, DEPLOYER=false ✓
R5 drift pre & post                       : 0 / 0 ✓
BE.nonce pre & post                       : 1 / 1 ✓
.env net delta                            : +1 line ✓
backend restart                           : 1s ready ✓
/executor/status post-restart             : 6/6 unchanged ✓
on-chain nonces                           : (1, 1) ✓
intent nonce sync                         : (1, 1) matches on-chain ✓
signatures                                : calldata_ready ✓
simulation                                : simulation_ok at block 42_581_165 ✓
broadcast attempts by agent               : 0 ✓
chain mutations by agent                  : 0 ✓
log canary                                : 8/8 patterns clean ✓
secrets printed                           : 0 ✓
governance/ownership/Timelock mutation    : 0 ✓
mainnet                                   : not touched ✓
```

---

## 9. Next milestone

`FIRST_LIVE_SMOKE-RFQ-BROADCAST-SEPOLIA-FEE-ONLY` — separately
authorised single `POST /broadcast` call on `INTENT_ID =
95516dbd-a68c-41eb-869f-e6790d9091f2`, mirroring the orderbook
`FIRST_LIVE_SMOKE-EXEC-V2-SEPOLIA-FEE-ONLY` pattern (re-verify all
hard gates → broadcast once → poll to terminal → receipt + events +
accounting + R5 verification → paste-back).

**End of result document.**
