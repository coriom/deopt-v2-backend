# V2G-PX — OPTION RFQ Operator Packet Walkthrough

## Status

- Milestone: **V2G-PX** — operator-facing walkthrough that turns
  the V2G-P1 packet-builder primitives into a step-by-step
  procedure for the V2G-P broadcast window. **Docs-only.** No
  broadcast.
- Date: 2026-06-01.
- Companion to:
  - `OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md` (primitives reference)
  - `OPTION_RFQ_LIVE_READINESS_V2G_PX.md` (top-level summary)
  - `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`
  - `deopt-v2-sol/docs/OPTION_RFQ_SMOKE_RUNBOOK_V2G_PX.md`

---

## 1. What the operator packet does

`build_option_rfq_operator_packet(inputs) -> OptionRfqOperatorPacket`
(in `deopt-v2-backend/src/options/rfq_operator_packet.rs`) produces:

| Field | Meaning |
|---|---|
| `digest_hex` | The EIP-712 digest the maker AND taker EOAs must sign offline. |
| `digest_bytes` | Same digest as `[u8; 32]` for pipelining into hardware-wallet APIs. |
| `function_selector_hex` | `0xb52ce6f5` — the `executeRfqTrade` selector. Operator sanity-checks against this before broadcasting. |
| `calldata_hex` | The full ABI-encoded `executeRfqTrade(OptionRfqTrade, buyerSig, sellerSig)` calldata — only populated when both signatures are supplied as input. |
| `payload_summary` | Human-readable summary of the OptionRfqTrade payload (intentId, buyer, seller, optionId, expiry, quantity, premium, buyerIsMaker, deadline). Never contains keys or signatures. |
| `broadcast_confirm_env` | `"OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM"` — the env name the executor binary must check. |
| `broadcast_ready` | `true` only when both sigs are attached. |

The packet builder is **pure compute**:
- Never holds a private key.
- Never makes a network call.
- Never persists DB state.
- Never broadcasts.

---

## 2. Two-phase use pattern

### Phase A — generate the digest (no signatures)

```rust
let inputs = OptionRfqOperatorPacketInputs {
    payload: &option_trade_payload,   // OptionTradePayload — same struct
                                       // shape as ORDERBOOK
    domain: &eip712_domain,           // chainId + verifyingContract =
                                       // new OptionMatchingEngine
    signatures: None,                 // not yet
};
let packet = build_option_rfq_operator_packet(inputs)?;
println!("digest = {}", packet.digest_hex);
// e.g. 0x4f3a…
```

### Phase B — re-build with both signatures

```rust
let signatures = OptionTradeSignatureBundle::new(
    /* buyer_signature */ &maker_or_taker_sig_hex_0,
    /* seller_signature */ &maker_or_taker_sig_hex_1,
)?;
let inputs_with_sigs = OptionRfqOperatorPacketInputs {
    payload: &option_trade_payload,
    domain: &eip712_domain,
    signatures: Some(&signatures),
};
let packet = build_option_rfq_operator_packet(inputs_with_sigs)?;
assert!(packet.broadcast_ready);
let calldata = packet.calldata_hex.expect("populated when sigs present");
// Hand `calldata` + target=new_option_matching_engine to the executor.
```

Phase B is invoked by the operator's broadcast tooling once
**both** maker + taker EOAs have signed the digest from Phase A.
The phases are independent — Phase A can be re-run at any time
without losing state.

---

## 3. Off-band signing flow

The dashboard and backend NEVER hold a private key. The operator
collects signatures via one of:

| Option | How |
|---|---|
| Hardware wallet (Ledger / Trezor) | EIP-712 signing — most production-realistic |
| Air-gapped signing CLI | `cast wallet sign --private-key $PK --no-hash $DIGEST_HEX` (test rigs only) |
| Multi-party signing service | external signer that produces a 65-byte secp256k1 signature |

For the V2G-P first-smoke trade, the V2G-D2 EOA registry
addresses are:

| Role | Address | Notes |
|---|---|---|
| Tier 4 maker | `0x290bd12c93e467bf51c51f5273d35bddb19e9274` | claimed Tier 4 (V2G-D3) — maker rebate −50 ppm |
| Tier 2 taker | `0x77ca9dd6ccce2d692fb23877a2db7178807b0020` | claimed Tier 2 (V2G-D3) — taker fee 125 ppm ORDERBOOK / 94 ppm RFQ |

These addresses are public; their **private keys are NEVER**
named or read by Claude, the backend, the dashboard, or any
script in this repo. The operator holds them in a hardware
wallet or air-gapped store.

---

## 4. Step-by-step procedure (V2G-P first RFQ trade)

### Step 1 — Build the OptionRfqTrade payload

```rust
let payload = OptionTradePayload {
    intent_id: option_execution_intent_id_to_b256("<unique-string>")?,
    buyer: AccountId::new("0x77ca9dd6ccce2d692fb23877a2db7178807b0020"), // Tier 2 taker
    seller: AccountId::new("0x290bd12c93e467bf51c51f5273d35bddb19e9274"), // Tier 4 maker
    option_id: <U256 from option_product_registry_option_id(...)>,
    underlying: AccountId::new("0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02"), // mWETH
    settlement_asset: AccountId::new("0x6eAe407f5640B006faC9965182e238582A3B412E"), // mUSDC
    expiry: 1_893_456_000, // post-V2G-P series window
    strike_1e8: 300_000_000_000,
    is_call: true,
    contract_size_1e8: 100_000_000,
    quantity: 1,
    premium_per_contract: 1_000_000, // 1 mUSDC = 1e6 native
    buyer_is_maker: false,            // buyer = taker; seller = maker
    buyer_nonce: <fresh>,             // pull from /admin/options/.../signing-payload
    seller_nonce: <fresh>,
    deadline: u128::from(<UTC now + 600 secs>),
};
```

Expected fee accounting per V2G-T canonical table:

| Side | Tier | ppm (V2G-N RFQ) | Fee on 1e6 premium |
|---|---|---|---|
| Buyer (taker, Tier 2) | 2 | 94 | 94 native (ceil) |
| Seller (maker, Tier 4) | 4 | −50 (preserved under Design A) | 50 native rebate (floor) |

Net protocol fee = 94 − 50 = **44 native units**.
Rebate budget delta = **−50 native units** on mUSDC asset.

### Step 2 — Generate the digest (Phase A)

```bash
# Pseudo-CLI; the actual operator wraps this in a binary that
# imports the packet builder from `deopt-v2-backend`.
operator-tool rfq build-packet \
  --payload payload.json \
  --domain-chain-id 84532 \
  --domain-verifying-contract <new OptionMatchingEngine> \
  --output digest.json

cat digest.json
# {
#   "digest_hex": "0x4f3a…",
#   "function_selector_hex": "0xb52ce6f5",
#   "calldata_hex": null,
#   "payload_summary": "OptionRfqTrade(intentId=…, buyer=0x77ca…0020, seller=0x290b…9274, …)",
#   "broadcast_confirm_env": "OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM",
#   "broadcast_ready": false
# }
```

Operator confirms:
- `function_selector_hex == 0xb52ce6f5` (the RFQ selector — distinct from ORDERBOOK `0x031f77b3`).
- `payload_summary` matches the intended trade (buyer/seller addresses, premium, expiry).
- No `private`, `secret`, `mnemonic`, `0xaa…`, `0xbb…` strings appear in any output.

### Step 3 — Collect maker + taker signatures (off-band)

```bash
# Ledger / Trezor / shell-only signer — never echoes the digest
# input to disk, never prints the private key.
MAKER_SIG=$(<hardware wallet signs digest with maker EOA>)
TAKER_SIG=$(<hardware wallet signs digest with taker EOA>)

# Sanity check the signatures recover to the expected addresses.
# This is a PURE LOCAL operation — no broadcast.
operator-tool rfq recover \
  --digest 0x4f3a… \
  --signature "$MAKER_SIG" \
  # → 0x290bd12c93e467bf51c51f5273d35bddb19e9274 (Tier 4 maker) ✅

operator-tool rfq recover \
  --digest 0x4f3a… \
  --signature "$TAKER_SIG" \
  # → 0x77ca9dd6ccce2d692fb23877a2db7178807b0020 (Tier 2 taker) ✅
```

Hard rule: **if recovery does not return the expected EOA,
ABORT.** The signature is invalid, the digest was altered, or the
EOA was wrong. Do NOT proceed.

### Step 4 — Re-build the packet with signatures (Phase B)

```bash
operator-tool rfq build-packet \
  --payload payload.json \
  --domain-chain-id 84532 \
  --domain-verifying-contract <new OptionMatchingEngine> \
  --buyer-signature "$TAKER_SIG" \
  --seller-signature "$MAKER_SIG" \
  --output broadcast.json

cat broadcast.json
# {
#   "digest_hex": "0x4f3a…",
#   "function_selector_hex": "0xb52ce6f5",
#   "calldata_hex": "0xb52ce6f5…",   ← now populated
#   "payload_summary": "OptionRfqTrade(…)",
#   "broadcast_confirm_env": "OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM",
#   "broadcast_ready": true
# }
```

Operator confirms:
- `broadcast_ready == true`.
- `calldata_hex[0..10] == "0xb52ce6f5"` (the RFQ selector).
- The digest is unchanged from Phase A (signatures were over the
  same digest).

### Step 5 — Broadcast under the confirm gate

```bash
export OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM=true

operator-tool rfq broadcast \
  --calldata "$(jq -r .calldata_hex broadcast.json)" \
  --to <new OptionMatchingEngine> \
  --rpc-url "$BASE_SEPOLIA_RPC_URL" \
  --executor-private-key-env EXECUTOR_PRIVATE_KEY \
  --slow
```

The executor binary:
1. Reads `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM` and refuses to broadcast unless it equals the literal string `"true"` (per `require_option_rfq_broadcast_confirm`).
2. Reads the executor private key from env (executor EOA is the V2G-D2 executor; NOT the maker/taker EOAs).
3. Signs the outer transaction (gas + nonce) with the executor key.
4. Submits the `executeRfqTrade(...)` call to `<new OptionMatchingEngine>`.

The executor key is the standard V2D-S executor — it never sees
the maker / taker private keys, only the maker / taker
**signatures** that came in via `broadcast.json`.

### Step 6 — Verify post-tx

```bash
# 1. Confirm the tx mined.
cast receipt $RFQ_TX_HASH --rpc-url "$RPC_URL"

# 2. Decode FeeChargedV2 + FeeRebatedV2 events via the admin endpoint.
curl -s -H "X-Admin-Token: $ADMIN_API_TOKEN" \
  "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=$RFQ_TX_HASH" \
  | jq '.per_tx[0]'

# Expected V2G-PX shape:
# {
#   "event_model": "v2",
#   "source_priority": "",
#   "fee_charged_v2_count": 1,
#   "fee_rebated_v2_count": 1,
#   "observed_total_charged": "94",
#   "observed_total_rebated": "50",
#   "net_protocol_fee": "44",
#   "by_product": {"option": "94"},
#   "by_flow": {"rfq": "94"},   ← first time this surfaces live
#   "by_side": {"taker": "94"},
#   "rebated_by_product": {"option": "50"},
#   "rebated_by_flow": {"rfq": "50"}
# }
```

```bash
# 3. Confirm Prometheus surfaces the new flow="rfq" series.
curl -s 'http://127.0.0.1:9090/api/v1/query?query=deopt_option_fee_charged_v2_total{flow="rfq"}'
```

```bash
# 4. Confirm rebate budget decreased by 50 (Tier 4 maker rebate).
curl -s 'http://127.0.0.1:9090/api/v1/query?query=deopt_fees_manager_v2_rebate_budget_native'
# Expected: previous value − 50.
```

---

## 5. Hard rules

| Rule | Why |
|---|---|
| Never log the digest input or signatures via `tracing::info!` outside the `deopt.admin.audit` target | Digests are typically not secret but signatures can be replayed; principle-of-least-exposure. |
| Never call `build_option_rfq_operator_packet` with a payload that has an empty `intent_id` | The packet builder rejects this (via `OptionTradePayload::validate`); operator-tool should refuse too. |
| Never accept a signature recovery that returns `address(0)` | OptionMatchingEngine rejects this on chain too, but operator-tool should catch it first. |
| Never broadcast without verifying `function_selector_hex == 0xb52ce6f5` | A wrong selector (e.g. `0x031f77b3` ORDERBOOK) would route through `applyTrade` instead of `applyRfqTrade`, billing ORDERBOOK ppm instead of RFQ. |
| Never set `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM` to a non-`"true"` value to try to bypass | `require_option_rfq_broadcast_confirm` rejects everything except the literal string `"true"`. |
| Never run V2G-P live without a rehearsed rollback path | See V2G-PX deploy-rewire runbook §5. |
| Never run V2G-P live before V2G-Y governance authority is established for the new contracts | Owners default to deployer EOA; per V2G-Y matrix, ownership must transition to ProtocolTimelock before any subsequent setter call. |

---

## 6. Acceptance criteria for V2G-PX operator-packet close

- [x] V2G-P1 primitives live in target/ (V2G-M2 restart picked them up).
- [x] Two-phase use pattern documented (§2).
- [x] Off-band signing flow documented (§3).
- [x] Step-by-step procedure pinned (§4).
- [x] Hard rules enumerated (§5).
- [ ] Operator-tool CLI implemented (out of scope for V2G-PX — the V2G-P1 module is a library; the CLI is operator infra). Status: documented; deferred to operator-side integration.
- [ ] First RFQ trade landed (operator gate).

---

## 7. Hard-gate compliance

V2G-PX broadcasts nothing. Every example command in this doc is
reference material for the operator at the broadcast window.
Claude never executes any of these against the live backend, the
live chain, or the live `.env`. The hard rule on confirm-flag
enforcement is hard-tested by `v2gw0` / `v2gw1` / `v2gw2` test
suites for the broadcast-confirm helper.

---

## 8. Cross-links

- V2G-P1 primitives: `OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md`
- V2G-PX live readiness: `OPTION_RFQ_LIVE_READINESS_V2G_PX.md`
- V2G-PX deploy/rewire: `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`
- V2G-PX smoke: `deopt-v2-sol/docs/OPTION_RFQ_SMOKE_RUNBOOK_V2G_PX.md`
- V2G-Y governance: `GOVERNANCE_ADMIN_SAFETY_MATRIX_V2G_Y.md`
- V2G-T canonical pack: `DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`
