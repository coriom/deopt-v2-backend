# Dry-run intent packet — V2G-FX-Q1-B3

**Posture:** operator runbook. **No flag flip, no live broadcast, no
chain mutation by agent.** Operator runs the commands; agent
verifies log/status output afterwards.

**Scope:** Base Sepolia (chain 84532). Backend in dry-run mode
(per FX-Q1-B2). Broadcast flags off (per FX-Q1-C pre-close).

**Goal:** drive **one** fee-only orderbook execution intent AND
**one** fee-only RFQ execution intent through the dry-run
simulation surface to confirm the signer identity end-to-end
without flipping live broadcast.

**Anchors:**
- `BACKEND_EXECUTOR = 0x295005fd4F311e6691F008D57d32FCFEde844518`
- `NEW_OME = 0x5a5EBF9A9CCd7c012518569DE8283982982670f6`
- `EXECUTOR_FROM_ADDRESS = OPTION_EXECUTION_SIMULATION_FROM = BACKEND_EXECUTOR`
- backend on `http://127.0.0.1:8080` (PORT from `.env`)
- `OPTION_EXECUTION_BROADCAST_ENABLED=false` AND `EXECUTOR_REAL_BROADCAST_ENABLED=false` ⇒ broadcast route refuses every attempt; simulate route is wide open.

---

## 0. Hard stops

```text
no chain tx                                 ✅
no live backend broadcast                   ✅
no RFQ/orderbook live smoke                 ✅
no trade                                    ✅
no reserve allocation                       ✅
no GOV-G                                    ✅
no .env edit by agent                       ✅
no private key output by agent              ✅
no admin token output by agent              ✅
no secrets printed                          ✅
no mainnet                                  ✅
```

---

## 1. Discovered intent submission paths

Source survey of `~/DEOPT/deopt-v2-backend/src/api/routes.rs` (lines 238-313) and `options/service.rs` shows the **intent lifecycle is matcher-driven**, not direct-POST. There is **no** `POST /options/execution-intents` endpoint that takes a raw trade body — intents are created internally when an `OptionFill` or `OptionRfqFill` lands.

### 1.1 Routes the operator drives directly (no admin token required for any of these on the option-execution surface)

| Step | Method | Path | Purpose |
|---|---|---|---|
| **A** | `POST` | `/options/orders` | submit an option order; matcher may create a fill |
| **B** (RFQ alternative to A) | `POST` | `/options/rfqs` | create RFQ |
| **B'** | `POST` | `/options/rfqs/:rfq_id/quotes` | MM submits quote |
| **B''** | `POST` | `/options/rfqs/:rfq_id/accept/:quote_id` | requester accepts; creates RFQ fill |
| **C** | `GET` | `/options/execution-intents` | list intents (find the one the matcher just created) |
| **D** | `GET` | `/options/execution-intents/:id/signing-payload` | EIP-712 payload for buyer + seller to sign |
| **E** | (CLI) | `cargo run --bin sign_option_execution_intent` | sign payload with buyer's PK; repeat with seller's PK |
| **F** | `POST` | `/options/execution-intents/:id/signatures` | submit both signatures |
| **G** | `POST` | `/options/execution-intents/:id/simulate` | run `eth_call` simulation **← THIS is the dry-run step** |
| **H** | `GET` | `/options/execution-intents/:id/simulation` | read simulation result |
| ~~I~~ | ~~`POST`~~ | ~~`/options/execution-intents/:id/broadcast`~~ | **refused while flags off** — would return `"option execution broadcast is disabled"` per `service.rs:2422` |

**`ensure_admin_access` is NOT called** on any `/options/execution-intents/*` route (verified by grep — no occurrence in lines 2820-2885 where the handlers live). The simulate handler is reachable without admin auth.

### 1.2 Admin auth shape (for reference; not needed for §G)

- `ensure_admin_access` is the active gate (`routes.rs:1675-1690`). Only `/admin/*` paths call it.
- If the operator wants to cross-check via `/admin/execution/summary`, they supply `x-admin-token: <REDACTED>`. Agent NEVER sees this header value.

### 1.3 CLI binaries available

```text
src/bin/sign_option_execution_intent.rs   ← sign an EIP-712 digest from a JSON payload file
src/bin/sign_option_rfq_quote.rs          ← sign an RFQ quote payload
src/bin/sign_perp_trade.rs                ← (perp; out of scope here)
src/bin/sign_rfq_quote.rs                 ← (perp RFQ; out of scope)
src/bin/mm_wt_smoke.rs                    ← MM WebTransport smoke (out of scope)
```

`sign_option_execution_intent` semantics (from `src/bin/sign_option_execution_intent.rs`):
- Reads `--payload <file>` (JSON from step D).
- Reads private key from env var named via `--private-key-env` (default `BUYER_PRIVATE_KEY`).
- Validates that the derived signer == buyer OR seller in the payload.
- Outputs `{ signer_address, signature }` JSON to stdout. **Never prints the private key.**

This is what we'll use to sign on behalf of the buyer + seller test wallets.

---

## 2. Operator prerequisites

```text
[ ] backend running in dry-run mode (FX-Q1-B2 verified)
[ ] /executor/status reports realBroadcastEnabled=false, broadcastEnabled=false
[ ] operator has TWO Sepolia test wallets ("BUYER", "SELLER") with:
    [ ] non-zero margin in CV for the option series (deposit via existing
        /accounts deposit flow if needed — out of scope here)
    [ ] a small Sepolia ETH balance for signing (no on-chain spend by them
        since broadcast is off)
[ ] a small option series + market that the matcher accepts. If none
    pre-exists, create via POST /options/series (operator-side).
[ ] OPERATOR holds:
    BUYER_PRIVATE_KEY  (env var set in operator's session — NEVER pasted to agent)
    SELLER_PRIVATE_KEY (env var set in operator's session — NEVER pasted to agent)
[ ] for any /admin/* fallback queries: x-admin-token in operator's session
    (operator-only)
[ ] working directory: ~/DEOPT/deopt-v2-backend
```

The agent does NOT request, hold, or echo buyer/seller keys or admin tokens. All `cast wallet`-style derivations are operator-side and never piped to the agent's bash.

---

## 3. Fee-only candidate parameters (BOTH dry-runs)

Backend's `should_broadcast` policy (per `BACKEND_GAS_FEES_REBATES_POLICY_V1.md`) WILL reject any rebate-bearing candidate even before reaching `provider.send_raw_transaction` (which is gated off anyway). But the same gate exists at the **simulation** layer — `eth_call(executeTrade)` would revert at `PFV.onRebatePaid` if any negative maker amount were charged. So make sure parameters fit the fee-only profile:

```text
TRADE NOTIONAL          ≤ 100 mUSDC  (small)
makerPpm                ≥ 0          (no negative maker = no rebate)
takerPpm                ≥ 1          (small positive taker fee)
RFQ makerDiscountPpm    ≤ PPM_DENOMINATOR
                        AND effective_maker_ppm
                          = makerPpm * (PPM - makerDiscountPpm) / PPM
                          ≥ 0
RFQ takerDiscountPpm    ≤ PPM_DENOMINATOR
isLiquidation           false
```

The intent the matcher creates will carry the operative `makerPpm`, `takerPpm`, and (for RFQ) the discount ppms from the active `FeesManagerV2.getProfile(...)` / `getRfqDiscountProfile(...)`. The operator should pre-check the tier/product profile via `cast` against `NEW_FM_V2` if uncertain (out of scope of this packet).

---

## 4. Orderbook dry-run command sequence

### 4.1 Step A — submit a matched-order pair

```bash
# Operator-only; agent does not see request bodies (test wallets, signatures, etc.)
cd ~/DEOPT/deopt-v2-backend

# A1 - buyer side
curl -sS -X POST http://127.0.0.1:8080/options/orders \
  -H 'content-type: application/json' \
  -d @./fixtures/dry_run_orderbook_buyer.json
# expected: { order_id: "<uuid>", status: "OPEN" or "PARTIALLY_FILLED" or "FILLED", ... }

# A2 - seller side (crossing price → matcher creates a fill)
curl -sS -X POST http://127.0.0.1:8080/options/orders \
  -H 'content-type: application/json' \
  -d @./fixtures/dry_run_orderbook_seller.json
# expected: status FILLED, with fill_id(s)
```

`fixtures/dry_run_orderbook_buyer.json` / `_seller.json` are operator-provided minimal payloads that cross at a single price. Notional + size kept small (≤ 100 mUSDC).

### 4.2 Step C — find the auto-created intent

```bash
curl -sS http://127.0.0.1:8080/options/execution-intents | jq '.[0]'
# expected: { intent_id: "<uuid>", status: "AWAITING_SIGNATURES", ... }
INTENT_ID=$(curl -sS http://127.0.0.1:8080/options/execution-intents | jq -r '.[0].intent_id')
echo "INTENT_ID=$INTENT_ID"
```

Cross-check: the intent's source_type should be `OptionOrderbookFill`.

### 4.3 Step D — fetch signing payload

```bash
curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signing-payload \
  | jq . \
  > /tmp/intent_payload.json
# expected fields: { digest, message: { buyer, seller, ... } }
```

Inspect (no secrets):
```bash
jq '{digest, buyer: .message.buyer, seller: .message.seller}' /tmp/intent_payload.json
```

### 4.4 Step E — sign with buyer + seller test keys (operator-side)

```bash
# Operator-only. Keys are in operator's session env. Agent NEVER sees them.
# BUYER_PRIVATE_KEY and SELLER_PRIVATE_KEY are set in this shell; never echoed.

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/intent_payload.json \
  --private-key-env BUYER_PRIVATE_KEY \
  > /tmp/buyer_sig.json

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/intent_payload.json \
  --private-key-env SELLER_PRIVATE_KEY \
  > /tmp/seller_sig.json
```

Each output: `{ signer_address, signature }`. Signer_address MUST equal the buyer/seller address in the payload — the CLI itself asserts this and errors out on mismatch (per `bin/sign_option_execution_intent.rs:42-52`).

### 4.5 Step F — submit signatures

```bash
BUYER_SIG=$(jq -r '.signature' /tmp/buyer_sig.json)
SELLER_SIG=$(jq -r '.signature' /tmp/seller_sig.json)

curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signatures \
  -H 'content-type: application/json' \
  -d "{
    \"buyer_signature\": \"$BUYER_SIG\",
    \"seller_signature\": \"$SELLER_SIG\"
  }"
# expected status: AWAITING_SIMULATION or READY_FOR_BROADCAST
```

### 4.6 Step G — simulate (THE DRY-RUN STEP)

```bash
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulate \
  -H 'content-type: application/json' \
  | jq .
# expected response JSON includes:
#   intent_id, intent_status, simulation_status="OK" or "REVERTED",
#   simulation_message, last_simulated_at_ms, ...
```

If `simulation_status = OK`: success. The simulator did an `eth_call(executeTrade)` from `EXECUTOR_FROM_ADDRESS = BACKEND_EXECUTOR` against NEW_OME, the chain returned success-path returndata. **No on-chain mutation; no tx hash; nothing broadcast.**

If `simulation_status = REVERTED`: read `simulation_message` for the revert reason. Common reasons:
- `NotAuthorized()` → backend's `from` is not an executor; re-check FX-Q1-A2.
- `BadNonce()` → buyer/seller nonces consumed elsewhere; resync.
- `InvalidSignature()` → CLI signed with wrong key, or payload was modified after signing.
- Anything mentioning `InsufficientRebateReserve` → trade has a negative maker; STOP and re-check params per §3.

### 4.7 Step H — read simulation result (idempotent re-fetch)

```bash
curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulation \
  | jq .
# same shape as step G response
```

---

## 5. RFQ dry-run command sequence

### 5.1 Step B — create RFQ + accept quote

```bash
# B1 - create RFQ
RFQ_ID=$(curl -sS -X POST http://127.0.0.1:8080/options/rfqs \
  -H 'content-type: application/json' \
  -d @./fixtures/dry_run_rfq.json | jq -r '.rfq_id')

# B' - MM submits a quote (operator drives MM-side from a separate session
# or uses the existing MM session; can be inline for dry-run)
QUOTE_ID=$(curl -sS -X POST http://127.0.0.1:8080/options/rfqs/$RFQ_ID/quotes \
  -H 'content-type: application/json' \
  -d @./fixtures/dry_run_rfq_quote.json | jq -r '.quote_id')

# B'' - requester accepts → creates OptionRfqFill → matcher creates intent
curl -sS -X POST http://127.0.0.1:8080/options/rfqs/$RFQ_ID/accept/$QUOTE_ID
# expected response carries the fill_id
```

### 5.2 Steps C-H — identical to §4.2-§4.7, but the intent's source_type should be `OptionRfqFill`

```bash
# Find the RFQ intent (newest after step 5.1)
INTENT_ID=$(curl -sS 'http://127.0.0.1:8080/options/execution-intents?source_type=OptionRfqFill' \
  | jq -r '.[0].intent_id')

# Fetch payload
curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/rfq_intent_payload.json

# Sign (buyer + seller, separately)
cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/rfq_intent_payload.json \
  --private-key-env BUYER_PRIVATE_KEY \
  > /tmp/rfq_buyer_sig.json

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/rfq_intent_payload.json \
  --private-key-env SELLER_PRIVATE_KEY \
  > /tmp/rfq_seller_sig.json

# Submit
BUYER_SIG=$(jq -r '.signature' /tmp/rfq_buyer_sig.json)
SELLER_SIG=$(jq -r '.signature' /tmp/rfq_seller_sig.json)
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signatures \
  -H 'content-type: application/json' \
  -d "{\"buyer_signature\":\"$BUYER_SIG\",\"seller_signature\":\"$SELLER_SIG\"}"

# Simulate
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulate \
  | jq .
# expected simulation_status=OK
```

---

## 6. Expected log signals (BOTH dry-runs)

The backend log lines surrounding each `/simulate` POST should
show:

```text
event   = execution_simulation
network = sepolia
chain_id = 84532
signer  = "0x295005fd4f311e6691f008d57d32fcfede844518"   ← BACKEND_EXECUTOR (case-insensitive)
from    = "0x295005fd4f311e6691f008d57d32fcfede844518"   ← BACKEND_EXECUTOR
target  = "0x5a5ebf9a9ccd7c012518569de8283982982670f6"   ← NEW_OME (case-insensitive)
intent_id = "<uuid>"
result  = "ok"                                            ← MUST be ok
broadcast_called = false                                  ← MUST be false (broadcast route refused)
```

Plus, per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` §7.1 schema (if structured logging is wired):

```json
{
  "service": "deopt-backend",
  "subsystem": "executor",
  "event": "should_broadcast",
  "result": "dropped",
  "reason": "OPTION_EXECUTION_BROADCAST_ENABLED=false"
}
```

OR (if the intent simply stops at `READY_FOR_BROADCAST` without `should_broadcast` ever being invoked):

```text
intent_status = AWAITING_SIMULATION → SIMULATION_OK → READY_FOR_BROADCAST
```

Hard gates the operator's log archive MUST confirm:

```text
[ ] `signer` matches BACKEND_EXECUTOR (case-insensitive)
[ ] `from`   matches BACKEND_EXECUTOR
[ ] `target` matches NEW_OME
[ ] `chain_id` = 84532
[ ] `simulation` returned `ok` (HTTP 200, simulation_status="OK")
[ ] NO `tx_hash` in any log line
[ ] NO `eth_sendRawTransaction` in any log line
[ ] NO `provider.send_raw_transaction` in any log line
[ ] NO 64+ char `0x`-prefixed hex run that resembles a private key
```

---

## 7. Redaction rules for operator paste-back

When the operator pastes back log excerpts or response JSON, they MUST redact:

```text
- any private-key-shaped hex (64 char 0x-prefixed)
- any admin token (header value)
- any RPC URL with embedded API key — replace with .../v2/<REDACTED>
- any DATABASE_URL credentials
- buyer/seller signature `signature` fields are OK to include
  (they are sig values for the public-side intent payload, not keys)
- intent_id, fill_id, quote_id are OK (UUIDs only; no PII)
- buyer/seller addresses are OK (public addresses on chain)
- gas envelope numbers are OK
- tx_hash should NOT appear (broadcast off); if it does, this is FS-15
```

Suggested paste-back template:

```text
V2G-FX-Q1-B3 dry-run result:

ORDERBOOK_INTENT
  intent_id              : <uuid>
  source_type            : OptionOrderbookFill
  buyer / seller addrs   : 0x... / 0x...
  signatures accepted    : yes (HTTP 200)
  simulate POST status   : 200
  simulation_status      : OK
  signer field in log    : 0x295005fd…4518
  from   field in log    : 0x295005fd…4518
  target field in log    : 0x5a5EBF9A…70f6
  chain_id               : 84532
  any tx_hash present?   : no
  any eth_sendRaw?       : no
  /executor/status post  : unchanged (dryRun=true, realBroadcastEnabled=false)

RFQ_INTENT
  intent_id              : <uuid>
  source_type            : OptionRfqFill
  (same fields as above)

Stop conditions hit: none | <list>
Notes: <free text; no secrets>
```

---

## 8. Stop conditions (§8 FX-* set)

Hit ANY of these → STOP entire FX-Q1-B3 session AND escalate to agent.

| Symbol | Trigger | Action |
|---|---|---|
| FB3-1 | Simulate POST returns HTTP 5xx | check backend logs; possible panic |
| FB3-2 | `simulation_status = REVERTED` with reason `NotAuthorized()` or `0xea8e4eb5` | env half-flip; re-check FX-Q1-A2 + FX-Q1-B2 |
| FB3-3 | `simulation_status = REVERTED` with reason `InvalidSignature()` | sign CLI signed with wrong key, OR payload edited post-sign |
| FB3-4 | `simulation_status = REVERTED` with reason `BadNonce()` | buyer/seller nonces consumed elsewhere; resync via /accounts nonce-sync |
| FB3-5 | `simulation_status = REVERTED` with reason mentioning `InsufficientRebateReserve` | candidate is rebate-bearing despite §3; abort and re-build with `makerPpm ≥ 0` |
| FB3-6 | `signer` or `from` field in logs ≠ BACKEND_EXECUTOR | env half-flip; re-check FX-Q1-A2 |
| FB3-7 | `target` field in logs ≠ NEW_OME | `OPTION_MATCHING_ENGINE_ADDRESS` mis-bound |
| FB3-8 | `chain_id` ≠ 84532 | env or RPC mismatch |
| FB3-9 | Any `tx_hash` appears | broadcast flag is on by mistake; STOP and run BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK §6 rollback |
| FB3-10 | Any 64+ char `0x`-prefixed hex run in logs (private-key-shaped) | logging hygiene breach; rotate key per `BACKEND_EXECUTOR_CUSTODY_PROFILE` §7.3 |
| FB3-11 | `/executor/status` `dryRun` / `realBroadcastEnabled` / `broadcastEnabled` changed mid-session | env or runtime drift; STOP |
| FB3-12 | Backend panicked or crashed during simulate | dump logs; do NOT restart blindly |
| FB3-13 | Buyer/seller signing CLI errored on signer-address mismatch | keys + payload addresses are out of sync — operator-side fix |
| FB3-14 | RFQ flow's `/accept/:quote_id` returned an error | RFQ quote expired or off-leg — re-issue quote |

---

## 9. Validations (agent-side)

```text
forge fmt --check     (sol)     : N/A (no .sol touched)
git diff --check      (sol)     : exit 0
git diff --check      (backend) : exit 0
cargo build / test    (backend) : N/A (no Rust source touched in this milestone)
.env edits by agent             : NONE ✅
secrets printed                 : NONE ✅
admin token output              : NONE ✅
private key output              : NONE ✅
backend live broadcast          : NONE ✅
chain probes                    : read-only ✅
flag flip                       : NONE ✅
smoke                           : NONE ✅
```

---

## 10. Cross-links

- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — parent.
- `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` — what happens NEXT after this packet runs clean.
- `FIRST_LIVE_SMOKE_AUTHORIZATION_V2G_FX_Q1.md` — what happens AFTER flag flip (separate operator decision).
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` §7 — log schema reference for §6 expected signals.
- `BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — gas/fee math; explains the §3 fee-only constraint.
- `BACKEND_EXECUTOR_CUSTODY_PROFILE_V2G_GOV_F.md` §7.3 — key-rotation procedure if FB3-10 fires.
