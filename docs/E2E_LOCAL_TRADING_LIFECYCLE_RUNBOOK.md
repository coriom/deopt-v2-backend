# E2E Local Trading Lifecycle Runbook (M-P4)

**Date:** 2026-06-10
**Posture:** local-only. **No mainnet. No Sepolia broadcast. No live RPC.
No production secrets. No `.env` production edits.**

This runbook gets a developer from cold-checkout to "signed envelope
visible in the trading UI" in ~10 minutes against anvil + the DeOpt
backend + the trading MVP frontend.

## 1. Prerequisites

| Tool | Version |
|---|---|
| foundry | latest (`curl -L https://foundry.paradigm.xyz \| bash` then `foundryup`) |
| rust | 1.75+ (`cargo --version`) |
| node | 22+ (`node --version`) |
| docker (optional) | 26+ (only if you prefer dockerised postgres) |
| MetaMask / Rabby / any EIP-1193 wallet | latest |

```bash
# Sanity check
forge --version
cargo --version
node --version
```

## 2. Terminal 1 — Anvil

```bash
anvil --chain-id 31337 --port 8545
```

Anvil starts with deterministic accounts:
- `anvil[0]` = operator (deploy + executor) — public dev key only
- `anvil[1]` = test wallet (you import this into MetaMask in §6)
- `anvil[2]` = optional counterparty

## 3. Terminal 2 — Sol deploy (one-time per anvil session)

```bash
cd ~/DEOPT/deopt-v2-sol
forge build
# Deploy core + configure (uses anvil[0] as deployer)
PRIVATE_KEY=<anvil[0]_private_key> \
forge script script/DeployCore.s.sol \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
forge script script/DeployTestnetAssets.s.sol \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
forge script script/DeployLocalMockFeeds.s.sol \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
forge script script/ConfigureCore.s.sol \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
forge script script/ConfigureMarkets.s.sol \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast
```

Each script prints the deployed addresses. Copy them into the backend
`.env.local` in §4.

**`PRIVATE_KEY` is anvil's public deterministic key — NEVER a real key.**

## 4. Terminal 3 — Backend

```bash
cd ~/DEOPT/deopt-v2-backend

# Copy the template + substitute local values
cp .env.example .env.local

# Edit .env.local — the following overrides are REQUIRED for anvil:
cat <<'EOF'
HOST=127.0.0.1
PORT=8080
CHAIN_ID=31337
NETWORK_NAME=anvil-local
EIP712_CHAIN_ID=31337
EIP712_VERIFYING_CONTRACT=<OptionMatchingEngine address from §3>
RPC_URL=http://127.0.0.1:8545
EXECUTOR_FROM_ADDRESS=<anvil[1] address>
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
PERSISTENCE_ENABLED=false
SIGNATURE_VERIFICATION_MODE=strict
OPTION_MATCHING_ENGINE_ADDRESS=<from §3>
MARGIN_ENGINE_ADDRESS=<from §3>
COLLATERAL_VAULT_ADDRESS=<from §3>
FEES_MANAGER_V2_ADDRESS=<from §3>
PROTOCOL_FEE_VAULT_ADDRESS=<from §3>
EOF

# Run backend
cargo run --release
```

Backend now serves on `http://127.0.0.1:8080`. Verify:

```bash
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/ready
curl http://127.0.0.1:8080/trading/health
```

## 5. Terminal 4 — Frontend

```bash
cd ~/DEOPT/deopt-v2-frontend

# One-time install (only if node_modules is stale)
npm install

# Local env
cp .env.example .env.local

# Edit .env.local:
cat <<'EOF'
NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:8080
NEXT_PUBLIC_CHAIN_ENV=anvil
EOF

npm run dev
```

Frontend now serves on `http://localhost:3000`.

## 6. Wallet setup

In MetaMask / Rabby:
1. Add custom network:
   - Network name: `Anvil`
   - RPC URL: `http://127.0.0.1:8545`
   - Chain ID: `31337`
   - Currency symbol: `ETH`
2. Import account using `anvil[1]` private key (printed in §2). **Use a
   fresh browser profile — do NOT use a wallet that holds real funds.**
3. Switch wallet to the Anvil network.

## 7. Optional Terminal 5 — Prism mock fallback

If the backend is not ready or you want to exercise UI states against
synthetic data:

```bash
cd ~/DEOPT
npx @stoplight/prism mock deopt-v2-backend/docs/openapi/trading-api.openapi.json --port 4010
```

Then point the frontend at `:4010`:

```bash
cd ~/DEOPT/deopt-v2-frontend
NEXT_PUBLIC_TRADING_API_BASE_URL=http://localhost:4010 npm run dev
```

Note: the legacy `/options/execution-intents/*` endpoints are NOT in
the OpenAPI spec — Prism returns 404 for them. Use the real backend
(§4) to exercise the signing flow.

## 8. Route checks

Open `http://localhost:3000` in the browser configured per §6.

Expected for each route:

| URL | Expected |
|---|---|
| `/` | Trading landing + MarketSelector |
| `/markets` | MarketSelector with products from backend |
| `/markets/<productId>` | OptionChain + RfqPanel; TradeTicket renders on series select |
| `/portfolio` | PortfolioSummary + PositionsTable + BalancesCard (partial-real data per M-P2b) |
| `/history` | TradeHistoryTable (empty if no fills yet) |
| `/transactions/<intentId>` | TxStatusTimeline (polls every 2 s; stops on terminal) |
| `/health` | TradingHealthCard (overall_status + chain_id + indexer_lag) |
| `/admin` | Admin dashboard (SEPARATE scope; admin Bearer still required there; NEVER from trading UI) |

Persistent banners on ALL trading routes:
- amber: "⚠ Testnet beta — NOT YET AUDITED. Do NOT deposit real funds. Mainnet trading is disabled."
- if mainnet wallet detected: red "❌ Mainnet detected — Trading on Base mainnet is DISABLED…"

## 9. API checks

While the backend runs:

```bash
# Products
curl http://127.0.0.1:8080/options/products | jq

# Quote preview (use a real series_id from /options/products response)
curl "http://127.0.0.1:8080/options/quotes/preview?series_id=<id>&side=buy&size=1" | jq

# Positions (anvil[1] address)
curl "http://127.0.0.1:8080/accounts/<anvil1_address>/positions" | jq

# Health
curl http://127.0.0.1:8080/trading/health | jq
```

Each returns the `{ status, data, warnings, meta }` envelope. M-P2b
endpoints return `status: "partial"` + `warnings[]` with
`PARTIAL_PREVIEW` or `SOURCE_UNAVAILABLE_FIELD` codes — this is
expected.

## 10. Signing checks

Once a backend operator has created an execution intent (out of M-P4
scope; see `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md
LOCAL_INTENT_FIXTURE_MISSING`):

1. Browse to `/markets/<productId>` → pick a series.
2. Paste the `intent_id` into the TradeTicket's "Execution intent id" field.
3. Click "Sign typed data".
4. The SigningStateModal appears:
   - phase `fetching_payload` — backend `/signing-payload` returns EIP-712 envelope;
   - phase `awaiting_signature` — MetaMask opens for typed-data approval;
   - approve in MetaMask;
   - phase `submitting` — backend `/signatures` receives the buyer or seller signature;
   - phase `submitted` — router pushes to `/transactions/<intentId>`.
5. On `/transactions/<intentId>` the TxStatusTimeline polls every 2 s.

**The frontend NEVER triggers broadcast.** The intent state stops at
`SIGNED` (or `SIGNING_PAYLOAD_ISSUED` if you're the first signer)
until the operator manually broadcasts via the operator-side
`/options/execution-intents/:id/broadcast` endpoint.

## 11. Tx status checks

`useTxStatus(intentId)` is a composite poll of:
- `GET /options/execution-intents/:id` → `intent.status` ∈ {CREATED, SIGNING_PAYLOAD_ISSUED, SIGNED, SIMULATED_OK, BROADCAST, CONFIRMED, REVERTED, STUCK}.
- `GET /executor/transactions/:intent_id` → `tx.tx_hash`, `tx.block_number`, `tx.reverted_reason`.

UI behaviours:
- CONFIRMED → row goes emerald; polling stops; tx_hash + block_number rendered.
- REVERTED → red row with `reverted_reason` text.
- STUCK → amber row "operator review pending".
- Non-terminal → polling continues at 2 s cadence.

## 12. Expected warnings / partial envelopes

You'll see these in the UI as amber cards under partial-real panels:

| Code | Where | Message |
|---|---|---|
| `PARTIAL_PREVIEW` | quote / close / exercise preview | "Preview is a deterministic approximation. Full on-chain MarginEngineLens orchestration lands in M-P2c." |
| `SOURCE_UNAVAILABLE_FIELD` | positions / portfolio / balances / previews | per-field "X not yet wired in M-P2b" with specific source named |
| `ORACLE_MARK_NOT_WIRED` | series details | "oracle_mark_1e8 + orderbook_top will be wired in M-P2a follow-on" |

These are M-P2b's documented partial-data warnings. **They are not
errors.** UI renders graceful "approximate" badges.

## 13. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Frontend shows "Network error" | Backend not running or unreachable | Restart backend; verify `NEXT_PUBLIC_TRADING_API_BASE_URL` env |
| Frontend shows "wrong network" banner | Wallet not on anvil (31337) | Switch wallet to Anvil network in §6 |
| `MainnetDisabledBanner` shows | Wallet reports chain 8453 | Switch wallet to Anvil; mainnet is permanently disabled |
| Anvil restart breaks deploys | New deploy addresses | Re-run §3 deploys + update backend `.env.local` |
| Backend "options disabled" 4xx | `OPTIONS_ENABLED=false` env | Set `OPTIONS_ENABLED=true` in `.env.local` (or wait for the default-enabled path) |
| `useTxStatus` polling spinner forever | Intent never reaches terminal state | M-P4: operator has not yet broadcast. M-P4c (2026-06-10): set `state.local_test_fixtures = enabled_for_chain_id(chain_id)` in a local-only binary; drive synthetic state via `POST /admin/test/intent/:id/transition`. See `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md`. |
| `INDEXER_STALE` warning | Indexer lag exceeds threshold | Restart indexer via `POST /indexer/tick` |
| Wallet pop-up says "tx will fail" | EIP-712 envelope shape mismatch | Verify backend `EIP712_VERIFYING_CONTRACT` matches the deployed `OptionMatchingEngine` address |
| `npx next build` fails after changes | `.next/` cache stale | `rm -rf .next && npm run build` |
| `cargo run` fails on missing env | `.env.local` missing required var | Compare to `.env.example`; supply missing values |

## 14. Safety reminders

- This runbook NEVER instructs you to broadcast a real transaction.
- This runbook NEVER asks for a production private key, RPC URL, or
  `DATABASE_URL`.
- The frontend NEVER triggers broadcast — that lives on the backend
  operator side.
- Mainnet is **permanently disabled** in code (`isMainnetEnabled() === false`).
- If you ever see a wallet prompt to send a transaction to Base mainnet
  through DeOpt UI, **stop and report it** — it's a bug.

## 15. Cross-links

- `E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md` (M-P4 result)
- `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md`
- `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (M-P4c — synthetic tx-status fixture)
- `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md` (M-P4c operator runbook)
- `E2E_LOCAL_FIXES_NEXT_TASK.md`
- `~/DEOPT/deopt-v2-sol/LOCAL_REHEARSAL.md`
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_SIGNING_FLOW_RUNBOOK.md`
- `~/DEOPT/deopt-v2-frontend/docs/TRADING_UI_MOCK_API_RUNBOOK.md`
- `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`

**End of E2E local trading lifecycle runbook.**
