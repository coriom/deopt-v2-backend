# E2E Sepolia Read-Only Preflight Runbook (M-P5 Phase A)

**Date:** 2026-06-10
**Audience:** operator running the dry-run preflight before any
Sepolia broadcast.
**Posture:** **read-only. No broadcast. No live tx. No mainnet.
No production `.env` edit. No production secrets in this doc.**

> **IMPORTANT.** This runbook does **NOT** broadcast a transaction.
> It validates that the stack is technically ready. Phase B (live
> broadcast) requires a separate operator sign-off documented in
> `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`.

## 1. Prerequisites

* Working anvil locally (chain 31337) for the M-P4c fixture path.
* Working sol/backend/frontend trees on the M-P2f baseline.
* Operator-supplied Sepolia RPC URL (placeholder; never committed).
* Operator-supplied Sepolia env file `.env.base-sepolia` for the sol
  workspace (already in `.gitignore`) and a SEPARATE
  `.env.sepolia` (NOT `.env`) for the backend.

## 2. Step 1 — Backend dry-run config

```bash
cd ~/DEOPT/deopt-v2-backend

# Use a DEDICATED Sepolia env file. Do NOT edit production .env.
# Fields the operator must supply (placeholders shown):
#
#   CHAIN_ID=84532
#   NETWORK_NAME=base-sepolia
#   EXECUTION_ENABLED=false              # dry-run; keep false
#   EXECUTOR_DRY_RUN=true                # default; keep true
#   EXECUTOR_REAL_BROADCAST_ENABLED=false # keep false during Phase A
#
#   # Optional read-only addresses (M-P2e):
#   OPTION_PRODUCT_REGISTRY=…             # known: §3.2 of RESULT
#   OPTION_MATCHING_ENGINE=…              # known: §3.2 of RESULT
#   OPTION_MARGIN_ENGINE=…                # operator-confirmation required
#   OPTION_COLLATERAL_VAULT=…             # known: §3.2 of RESULT
#   OPTION_ORACLE_ROUTER_ADDRESS=…        # known: §3.2 of RESULT
#   OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS=… # OPERATOR_INPUT_REQUIRED
#   OPTION_MARGIN_ENGINE_LENS_ADDRESS=…    # OPERATOR_INPUT_REQUIRED
#   OPTION_MARGIN_ENGINE_ADDRESS=…         # = OPTION_MARGIN_ENGINE
#
#   EXECUTION_RPC_URL=<operator-supplied>
#
# Start backend (dry-run):
set -a; source .env.sepolia; set +a
cargo run --bin deopt-v2-backend &
```

Watch the logs for:
* `service=deopt-v2-backend` startup line.
* `chain_id=84532` confirmation.
* No `signer_mode=` line indicating a real signer.
* No `aws_kms` activation lines.

## 3. Step 2 — Read-only RPC probes

```bash
# Confirm RPC connectivity (no transaction sent).
curl -s -X POST "$EXECUTION_RPC_URL" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'
# Expect: {"jsonrpc":"2.0","id":1,"result":"0x14a34"}  (84532)

# Bytecode probes — confirms the contract exists at the configured
# address. Repeat for each address in §3.2 of the RESULT doc.
for ADDR in "$OPTION_PRODUCT_REGISTRY" "$OPTION_MATCHING_ENGINE" \
            "$OPTION_MARGIN_ENGINE" "$OPTION_COLLATERAL_VAULT" \
            "$OPTION_ORACLE_ROUTER_ADDRESS"; do
  cast code "$ADDR" --rpc-url "$EXECUTION_RPC_URL" | head -c 10
done
# Expect: "0x6080..."  per address.  An empty "0x" means MISSING.
```

## 4. Step 3 — Backend trading-views read probes

The M-P2e wired endpoints exercise the lens / vault / oracle read
paths. With the backend running on Sepolia + addresses configured:

```bash
# series_details now requires the oracle to be wired:
curl -s "http://localhost:8080/options/series/<series_id>/details" | jq .

# Expect: status="ok" iff oracle is wired; otherwise status="partial"
# with ORACLE_UNAVAILABLE / CONFIG_MISSING warning codes. Either is
# acceptable for Phase A — the goal is to verify NO panic + NO leak.

# portfolio with lens configured:
curl -s "http://localhost:8080/accounts/<TEST_BUYER>/portfolio" | jq .

# trading health:
curl -s "http://localhost:8080/trading/health" | jq .
```

Read every response with `jq` and confirm:
* `meta.chain_id == 84532`.
* No raw `http://` or `https://` URL in any warning message.
* No `EXECUTOR_PRIVATE_KEY` / `DATABASE_URL` / `AWS_*` substring.

## 5. Step 4 — Public create-intent dry-run

```bash
curl -s -X POST "http://localhost:8080/options/execution-intents" \
  -H 'Content-Type: application/json' \
  -d '{
    "series_id": "<active-series-id>",
    "side": "buy",
    "size_1e8": "100000000",
    "price_1e8": "10000",
    "buyer":  "0xc0A76c2A6c6b70C0B065A05E64417886416cc976",
    "seller": "0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3"
  }' | jq .
# Expect: { "intent_id": "<uuid>", "request_id": "<uuid>",
#           "status": "signatures_required",
#           "signing_payload_available": true,
#           "next_step": "request_signing_payload",
#           "expires_at_ms": null }
```

The intent is created in the backend store. **NOTHING IS
BROADCAST.** The intent sits at `SignaturesRequired` until both
parties sign — and even with signatures, broadcast only fires when
`EXECUTOR_REAL_BROADCAST_ENABLED=true` (which Phase A keeps `false`).

## 6. Step 5 — Signing-payload dry-run

```bash
curl -s "http://localhost:8080/options/execution-intents/<intent_id>/signing-payload" | jq .
# Expect: { "domain": { "chainId": 84532, "verifyingContract": "0x…f2D1D85…" },
#           "primaryType": "OptionTrade",
#           "types": { "OptionTrade": [...] },
#           "message": {...} }
```

Verify:
* `domain.chainId == 84532` (NEVER 8453).
* `domain.verifyingContract` matches the configured
  `OPTION_MATCHING_ENGINE`.

**Do NOT sign in Phase A.** The signing flow is exercised in Phase
B only.

## 7. Step 6 — Frontend point at Sepolia backend

```bash
cd ~/DEOPT/deopt-v2-frontend
NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080 npm run dev
# Visit http://localhost:3000 with MetaMask connected to Base Sepolia.
# Confirm:
#   * MainnetDisabledBanner does NOT show.
#   * NetworkBadge shows "base-sepolia".
#   * Trade ticket renders.
#   * Quote preview shows partial-or-ok with oracle warning.
#   * Create intent button enabled when wallet connected on 84532.
```

## 8. Step 7 — Worker preflight

The backend spawns three workers on startup:

* `spawn_option_confirmation_worker` (polls receipts; does not
  broadcast).
* `spawn_option_event_indexer` (reads logs; does not broadcast).
* `spawn_option_reconciliation_worker` (compares state; does not
  broadcast).

Verify in the backend log that none of these triggered an
`eth_sendRawTransaction`. With `EXECUTION_ENABLED=false` the executor
worker is not spawned at all.

## 9. Hard stops

Stop and switch to `E2E_SEPOLIA_FIXES_NEXT_TASK.md` if any of:

* RPC probe returns non-`0x14a34` chain id.
* Bytecode probe returns `0x` (empty) for any configured contract.
* Trading health returns `unhealthy` for reasons unrelated to optional
  partial reads.
* Any response leaks a raw RPC URL, private key, or AWS / KMS hint.
* `chain_id` ever equals `8453` (Base mainnet) — **immediate stop**.

## 10. Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` (Phase B precondition)
* `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md` (anvil/local equivalent)
* `~/DEOPT/TESTNET_RUNBOOK.md` (existing testnet ops runbook)

**End of read-only preflight runbook.**
