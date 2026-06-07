# Dry-run fixtures — V2G-FX-Q1-B4

Companion to `docs/DRY_RUN_INTENT_PACKET_V2G_FX_Q1_B3.md`. Files:

- `dry_run_orderbook_buyer.json`
- `dry_run_orderbook_seller.json`
- `dry_run_rfq.json`
- `dry_run_rfq_quote.json`

Posture: **operator-runnable**; the agent does NOT submit, sign,
broadcast, or hold any private key.

---

## 1. Placeholders the operator MUST fill

Every fixture contains one or more placeholder tokens (literal
`<...>`). They are NEVER agent-supplied. Operator substitutes
locally before POSTing:

| Token | Source | How to obtain |
|---|---|---|
| `<OPTION_SERIES_ID>` | backend | After backend is running: `curl -sS http://127.0.0.1:8080/options/series \| jq -r '.[0].option_series_id'` (read active series). If empty, create one via `POST /options/series` per your matcher's series-creation flow. Must be an `Active` series with `option_matching_engine_address` bound to NEW_OME. |
| `<BUYER_ADDRESS>` | operator test wallet | Buyer EOA on Sepolia. MUST have non-zero margin in CollateralVault for the series' settlement asset. Sample placeholder format: `"0x0000000000000000000000000000000000000001"` (the test pattern). REAL Sepolia run: use your own funded EOA. |
| `<SELLER_ADDRESS>` | operator test wallet | Seller / MM EOA on Sepolia. Same margin precondition. Different EOA from buyer (cannot be wash; both `should_broadcast` and the matcher reject same beneficial owner). |
| `<DEADLINE_MS>` | operator session | `node -e 'console.log(Date.now() + 60_000)'` OR `python3 -c "import time;print(int(time.time()*1000)+60000)"` — set to "now + 60s" each time you POST. |

Notes:
- `signature` fields use a literal 130-byte hex placeholder (`0xaa...aa`). The operator's `.env` has `SIGNATURE_VERIFICATION_MODE=disabled` AND `OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled` (per defaults in `src/config/env.rs:446` / `:258`); order + quote signature verification is OFF. If your env overrides either to `strict`, replace the placeholder with a real EIP-712 signature using the `sign_rfq_quote` / equivalent CLI; this is OUT OF SCOPE for the FX-Q1-B3 dry-run.
- `client_order_id` / `client_quote_id` / `session_id` are operator-readable strings; safe to leave as the fixture defaults. Append a session suffix (e.g. `-2025-06-07`) if you want to keep unique IDs across re-runs.

---

## 2. Fee-only candidate sanity (DO NOT MODIFY without re-checking)

The matcher will compute fees against the active `FeesManagerV2` tier+product profile when the execution intent simulates. For the candidates to remain **fee-only** (per FX-Q1-B3 §3):

```text
makerPpm                ≥ 0
takerPpm                ≥ 1
effective_maker_ppm     ≥ 0   (RFQ after discount)
no liquidation flag
notional ≤ 100 mUSDC
```

The fixtures default to **size = 1 contract** (`size_1e8 = 100_000_000`) AND **buyer price > seller price** (orderbook) / **MM quote price ≤ taker limit** (RFQ), so the cross results in exactly one trade. If the active series' `contract_size_1e8` is 1×10^8 and `settlement_asset = mUSDC` with `quote_decimals = 6`, the notional is:

```
notional_mUSDC = price_1e8 × size_1e8 × contract_size_1e8 / 1e24
              = 1.0e9   × 1.0e8    × 1.0e8             / 1e24
              = 10 mUSDC
```

Well under the 100 mUSDC budget. Operator MAY reduce price/size further for additional safety; MUST NOT raise without re-deriving the bound.

The operator MUST verify the **active fee profile** before submission:

```bash
NEW_FM_V2=0xF6626177f3B85cc3239667Cc53C04A8007652944
# Example: tier 0 OPTION product
cast call $NEW_FM_V2 \
  'getProfile(uint8,uint8)(int32,int32)' \
  0 0 \
  --rpc-url "$RPC_URL"
# expected: (makerPpm, takerPpm), both ≥ 0 for fee-only
```

If any active tier has `makerPpm < 0` (rebate-bearing), the operator MUST select a tier whose profile is fee-only OR temporarily seed the buyer/seller into a tier-0 fee-only bucket (operator-side decision; out of scope here).

---

## 3. Orderbook dry-run command sequence

The buyer's price (`1100000000` = 11.00) is higher than the seller's price (`1000000000` = 10.00) → matcher crosses at the resting side's price → 1 fill.

```bash
# Operator runs these. Working dir: ~/DEOPT/deopt-v2-backend

cd ~/DEOPT/deopt-v2-backend

# Pre-flight: fill placeholders in fixtures (operator-side; agent does NOT)
#   Replace <OPTION_SERIES_ID>, <BUYER_ADDRESS>, <SELLER_ADDRESS>, <DEADLINE_MS>
#   in the 4 fixture files. Operator's choice: edit in place, or copy to
#   /tmp/*.json and edit there. NEVER use sed -i if the original is needed.

# Refresh DEADLINE_MS for each POST:
DEADLINE_MS=$(python3 -c "import time; print(int(time.time()*1000)+60000)")
sed "s|<DEADLINE_MS>|$DEADLINE_MS|g" fixtures/dry_run_orderbook_buyer.json  > /tmp/buyer.json
sed "s|<DEADLINE_MS>|$DEADLINE_MS|g" fixtures/dry_run_orderbook_seller.json > /tmp/seller.json
# (the placeholders <OPTION_SERIES_ID>, <BUYER_ADDRESS>, <SELLER_ADDRESS>
#  must already be substituted in the fixture files BEFORE this step)

# A1 — seller resting order first (so the matcher has an ask to cross)
curl -sS -X POST http://127.0.0.1:8080/options/orders \
  -H 'content-type: application/json' \
  -d @/tmp/seller.json | jq .
# expected: { order_id, status: "open" }

# A2 — buyer aggressive order (crosses)
curl -sS -X POST http://127.0.0.1:8080/options/orders \
  -H 'content-type: application/json' \
  -d @/tmp/buyer.json | jq .
# expected: { order_id, status: "filled" or "partially_filled", ... }

# C — find auto-created execution intent
INTENT_ID=$(curl -sS http://127.0.0.1:8080/options/execution-intents \
  | jq -r '.[] | select(.source_type=="OptionOrderbookFill") | .intent_id' \
  | head -1)
echo "INTENT_ID=$INTENT_ID"

# D — fetch EIP-712 signing payload
curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/intent_payload.json
jq '{digest, buyer: .message.buyer, seller: .message.seller}' /tmp/intent_payload.json

# E — sign buyer + seller payloads with operator's test keys
#   BUYER_PRIVATE_KEY and SELLER_PRIVATE_KEY are in operator's env;
#   never echoed; never pasted to agent.
cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/intent_payload.json \
  --private-key-env BUYER_PRIVATE_KEY \
  > /tmp/buyer_sig.json

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/intent_payload.json \
  --private-key-env SELLER_PRIVATE_KEY \
  > /tmp/seller_sig.json

# F — submit signatures
BUYER_SIG=$(jq -r '.signature' /tmp/buyer_sig.json)
SELLER_SIG=$(jq -r '.signature' /tmp/seller_sig.json)
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signatures \
  -H 'content-type: application/json' \
  -d "{\"buyer_signature\":\"$BUYER_SIG\",\"seller_signature\":\"$SELLER_SIG\"}" \
  | jq .

# G — simulate (THE dry-run step)
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulate \
  | jq .
# expected: { intent_id, simulation_status: "OK", simulation_message: ..., ... }

# H — idempotent re-fetch
curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulation | jq .
```

---

## 4. RFQ dry-run command sequence

The MM quote price (`1000000000` = 10.00) ≤ taker limit (`1100000000` = 11.00); MM provides the ask, taker buys → acceptance creates RFQ fill.

```bash
cd ~/DEOPT/deopt-v2-backend

# B — create RFQ
RFQ_ID=$(curl -sS -X POST http://127.0.0.1:8080/options/rfqs \
  -H 'content-type: application/json' \
  -d @fixtures/dry_run_rfq.json | jq -r '.option_rfq_id')
echo "RFQ_ID=$RFQ_ID"

# B' — MM submits quote
QUOTE_ID=$(curl -sS -X POST http://127.0.0.1:8080/options/rfqs/$RFQ_ID/quotes \
  -H 'content-type: application/json' \
  -d @fixtures/dry_run_rfq_quote.json | jq -r '.quote_id')
echo "QUOTE_ID=$QUOTE_ID"

# B'' — accept (empty body)
curl -sS -X POST http://127.0.0.1:8080/options/rfqs/$RFQ_ID/accept/$QUOTE_ID \
  -H 'content-type: application/json' \
  -d '{}' | jq .
# expected: { status: "accepted", fill: { fill_id, ... } }

# C..H — same as orderbook §3 from step C onwards, filtered for source_type=OptionRfqFill
INTENT_ID=$(curl -sS http://127.0.0.1:8080/options/execution-intents \
  | jq -r '.[] | select(.source_type=="OptionRfqFill") | .intent_id' \
  | head -1)
echo "RFQ_INTENT_ID=$INTENT_ID"

curl -sS http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signing-payload \
  > /tmp/rfq_intent_payload.json

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/rfq_intent_payload.json \
  --private-key-env BUYER_PRIVATE_KEY \
  > /tmp/rfq_buyer_sig.json

cargo run --release --bin sign_option_execution_intent -- \
  --payload /tmp/rfq_intent_payload.json \
  --private-key-env SELLER_PRIVATE_KEY \
  > /tmp/rfq_seller_sig.json

BUYER_SIG=$(jq -r '.signature' /tmp/rfq_buyer_sig.json)
SELLER_SIG=$(jq -r '.signature' /tmp/rfq_seller_sig.json)
curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/signatures \
  -H 'content-type: application/json' \
  -d "{\"buyer_signature\":\"$BUYER_SIG\",\"seller_signature\":\"$SELLER_SIG\"}" \
  | jq .

curl -sS -X POST http://127.0.0.1:8080/options/execution-intents/$INTENT_ID/simulate | jq .
# expected: { simulation_status: "OK", ... }
```

---

## 5. Redaction rules for operator paste-back

When pasting back response JSON / log excerpts, redact per the
`DRY_RUN_INTENT_PACKET_V2G_FX_Q1_B3.md` §7 rules:

```text
MAY include:
  - intent_id, fill_id, quote_id, order_id (UUIDs)
  - buyer/seller addresses (public)
  - buyer/seller signature `signature` fields (intent-payload signatures, not key material)
  - simulation_status, simulation_message
  - gas envelope numbers
  - chain_id

MUST redact:
  - any 64+ char 0x-prefixed hex run that resembles a private key
  - admin token header values
  - RPC URL embedded API key (write as .../v2/<REDACTED>)
  - DATABASE_URL credentials
  - any DEADLINE_MS values that leak system uptime if sensitive
    (usually fine — just unix ms timestamps)

MUST NOT appear (their presence = stop condition):
  - tx_hash from a live broadcast
  - eth_sendRawTransaction call traces
  - provider.send_raw_transaction
```

Paste-back template (operator fills + redacts):

```text
V2G-FX-Q1-B4 dry-run result:

ORDERBOOK
  series_id              : 0x... (active)
  buyer / seller addrs   : 0x... / 0x...
  buyer order_id         : <uuid>; status: filled
  seller order_id        : <uuid>; status: filled
  fill_id                : <uuid>
  intent_id              : <uuid>
  signatures accepted    : yes (HTTP 200)
  /simulate response     : HTTP 200, simulation_status="OK"
  log signer/from/target : 0x295005fd…4518 / 0x295005fd…4518 / 0x5a5EBF9A…70f6
  chain_id               : 84532
  any tx_hash present?   : no
  any eth_sendRaw?       : no
  /executor/status post  : unchanged (dryRun=true, realBroadcastEnabled=false)

RFQ
  rfq_id, quote_id, fill_id : ...
  intent_id                  : ...
  (same fields as orderbook section)

Stop conditions hit: none | <list>
```

---

## 6. Cross-links

- `docs/DRY_RUN_INTENT_PACKET_V2G_FX_Q1_B3.md` — parent runbook.
- `docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia cutover.
- `docs/BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` — what happens NEXT (after dry-run is green).
- `docs/BACKEND_GAS_FEES_REBATES_POLICY_V1.md` — fee math behind §2.
- `tests/options_tests.rs` (lines 564, 1565+, 2440+) — reference fixtures the JSONs above were derived from.
