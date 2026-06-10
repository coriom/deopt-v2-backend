# DeOpt V2 — E2E Trading Lifecycle Test Plan

**Date:** 2026-06-10
**Posture:** docs-only plan. **No live transaction in this milestone.**
**No source code modified.** Companion to `PRODUCT_READINESS_ROADMAP.md`.

The plan covers local (Anvil + backend + frontend), Sepolia, RFQ, orderbook,
wallet, transaction status, event reconciliation, and failure cases. Each
flow has a deterministic harness and a documented expected end state.

## 1. Local E2E lifecycle (M-P4)

### 1.1 Harness

```
docker-compose (or local processes):
- Anvil at :8545 (Base-derived genesis; pre-funded test accounts)
- Postgres at :5432 (clean schema)
- Backend at :3000 (with `EXECUTOR_PRIVATE_KEY=<anvil[0]>`, `OPTION_NONCE_SYNC_ENABLED=true`, `OPTION_CONFIRMATION_WORKER_ENABLED=true`)
- Frontend at :3001 (Next dev server)
- Playwright + viem test runner
```

### 1.2 Deterministic seed

- `script/DeployCore.s.sol` + `script/ConfigureCore.s.sol` + `script/DeployTestnetAssets.s.sol` + `script/DeployLocalMockFeeds.s.sol` against anvil.
- Manifest pinned at `deployments/local.template.json` (test variant).
- Backend reads the manifest on startup; admin endpoint `/admin/config` confirms wired addresses.

### 1.3 Full-flow scenario

```
Step 1 — Connect wallet
   Playwright connects an injected provider (anvil[1]) to frontend.
   Expected: wallet badge in nav; network banner absent (chain_id matches).

Step 2 — Deposit
   User calls CV.deposit(mUSDC, 10_000e6).
   Expected: GET /accounts/anvil1/balances returns 10_000e6 mUSDC.

Step 3 — Browse option chain
   Visit /markets/ETH/<expiry>/call.
   Expected: option chain renders rows for each strike; oracle mark visible.

Step 4 — Trade ticket (orderbook buy)
   Select a series; size = 1; price = mark + 1%; click Sign.
   Expected: backend issues signing-payload; wallet returns EIP-712 signature;
   backend posts /options/execution-intents/:id/signatures;
   /options/execution-intents/:id/broadcast submits; tx hash returned.

Step 5 — Wait for confirmation
   Expected: confirmation worker reports CONFIRMED within 30 blocks (anvil mines on-demand);
   /executor/transactions/:intent_id shows status=confirmed.

Step 6 — Verify position
   GET /accounts/anvil1/positions returns the open position with size=1.

Step 7 — Exercise (or close)
   Click Exercise (advance anvil clock to >= expiry first).
   Expected: settle calldata signed + broadcast; CV balance updates.

Step 8 — Withdraw
   User calls CV.withdraw(mUSDC, remaining_balance).
   Expected: balance returns to wallet.

Step 9 — Verify R5 drift
   GET /admin/fees/vault/reconciliation returns drift=0.
```

### 1.4 RFQ scenario (local)

```
Step 1 — Connect wallet (taker = anvil[1], maker = anvil[2]).

Step 2 — Taker creates RFQ
   POST /options/rfqs { series, side, size, ttl_seconds }.
   Expected: rfq_id returned.

Step 3 — Maker subscribes (frontend market-maker view; M-P3 may include a simplified MM panel)
   GET /options/rfqs returns the open RFQ.

Step 4 — Maker signs quote
   GET /options/rfqs/:id/quote-signing-payload → signs → POST quote.
   Expected: backend stores signed quote.

Step 5 — Taker accepts
   POST /options/rfqs/:id/accept/:quote_id.
   Expected: execution-intent constructed.

Step 6 — Taker signs execution intent
   GET /options/execution-intents/:id/signing-payload → signs → POST.
   Expected: backend broadcasts.

Step 7 — Wait for confirmation; verify R5 drift = 0.
```

### 1.5 Pass criteria

- Every step's expected state matches actual within timeout (30 s for non-block steps; 30 anvil-mined blocks for confirmation).
- R5 drift = 0 at end of run.
- `signer_denied_total` = 0 (no policy refusals against valid intents).
- `local_signer_on_mainnet_refused_total` = 0 (anvil chain id is not mainnet; check defence-in-depth in mainnet-shape test).

## 2. Sepolia E2E lifecycle (M-P5)

### 2.1 Harness

```
- Existing Sepolia deployment (OME / PFV / FM_V2 / CV / RG / mUSDC anchors)
- Backend pointed at Sepolia RPC + `EXECUTOR_PRIVATE_KEY=<rehearsal-only sepolia key>`
- Frontend pointed at backend
- Playwright + viem (configured for Base Sepolia)
- Pre-funded Sepolia test wallets (taker + maker) with mUSDC
```

### 2.2 Modifications from local

- Block-time + RPC variability: confirmation timeout increased to 90 s.
- Maker bot replaces Playwright maker for RFQ (off-the-shelf test maker — Markovian sampler from a fixed seed).
- Real Base Sepolia RPC; no anvil clock control; tests target post-expiry series only.
- Acceptance: R5 drift = 0 across the full lifecycle (matching prior Sepolia rehearsal property).
- **No mainnet broadcast in this plan. Sepolia only.**

### 2.3 Pass criteria

- Same as §1.5 but tolerances loosened (90 s timeout; ≥ 2 confirmations).

## 3. Frontend wallet lifecycle test

### 3.1 Scenarios

| Scenario | Expected |
|---|---|
| Connect (happy) | wallet badge appears; trading UI enabled |
| Connect wrong network | banner + Switch CTA; Sign disabled |
| Switch network | banner disappears |
| Reject signature in wallet | warn toast; intent remains unsigned (refresh-safe) |
| Disconnect mid-flow | reconnect CTA; partial state cleared |
| Multiple wallets installed | provider picker (out of scope for MVP — first injected wins) |
| Wallet not installed | "Install a wallet" CTA on landing |
| EIP-712 v4 not supported | "Update your wallet" banner |

## 4. Backend transaction status lifecycle test

### 4.1 Scenarios

| Scenario | Expected |
|---|---|
| Submit valid intent | execution-intent CREATED → SIGNING_PAYLOAD_ISSUED |
| Post valid signature | SIGNED |
| Simulate → ok | SIMULATED_OK |
| Broadcast → tx hash | BROADCAST |
| Receipt status=1 | CONFIRMED |
| Receipt status=0 | REVERTED with reason from `RevertReason` parser |
| Receipt timeout | STUCK → eligible for replacement |
| BadNonce revert | nonce-sync remediation triggers; re-broadcast (`OPTION_NONCE_SYNC_ENABLED=true`) |
| Backend restart mid-flow | confirmation worker picks up from last-known state |

### 4.2 Pass criteria

- Every state transition is reachable + observable via `GET /options/execution-intents/:id` + `GET /executor/transactions/:intent_id`.
- No stuck intents at end of run.
- Recovery scenarios produce a single final state per intent.

## 5. Event reconciliation lifecycle test

### 5.1 Scenarios

| Scenario | Expected |
|---|---|
| Single fill → indexer ingests | `GET /options/fills` returns the fill |
| Reorg (anvil only) | reorg + re-ingest produces single canonical fill (no double-credit) |
| Rebate path (INACTIVE) | `PFV.rebateReserve` unchanged |
| Reconciliation tick | `GET /reconciliation/status` returns last_run_at + zero drift |

### 5.2 Pass criteria

- `/reconciliation/status` reports zero drift.
- `/reconciliations` returns matched-pair for every broadcast intent.

## 6. Failure-case test matrix

| Case | Scenario | Expected backend response | Expected UI |
|---|---|---|---|
| Stale quote | `/quotes/preview` returns expired | broadcast attempt → 400 stale_quote | refresh-quote banner |
| Rejected signature | wallet returns reject | intent stays SIGNING_PAYLOAD_ISSUED | warn toast + retry |
| Failed broadcast | RPC returns error | execution-intent BROADCAST_FAILED with reason | red toast + retry |
| Revert | receipt status=0 | execution-intent REVERTED with parsed reason | red toast + details modal |
| Insufficient collateral | `/quotes/preview` returns 400 insufficient_collateral | UI surfaces required vs available + deposit CTA | inline error |
| Signer unavailable | `/executor/health/v2.signer.signer_health_check_status != "ok"` | `should_broadcast` refuses; intent stays SIGNED | persistent banner; Sign disabled |
| RPC unavailable | RPC times out | execution-intent BROADCAST_FAILED with rpc_unavailable | persistent banner |
| Network mismatch | wallet chain id ≠ backend chain id | UI refuses; backend never sees the intent | network-mismatch banner |
| Backend unhealthy | health endpoint reports degraded | UI banner + Sign disabled | yellow banner; read-only UI |
| Backend unreachable | UI HTTP fetch fails | "Trading service offline" banner | red banner |

## 7. Tooling

- Sol: `forge test`; `forge test --match-path 'test/scenario/*'` for scenario suites.
- Backend: `cargo test --all-targets --all-features`; new E2E harness as a backend integration test crate (M-P4).
- Frontend: Playwright (`@playwright/test`) + viem; one cross-browser run (Chromium minimum); JUnit reporter for CI.
- CI: GitHub Actions or self-hosted; matrix per repo; shared deployment artifacts.

## 8. Hard rules

- **No live mainnet tx in any test.**
- **No real KMS / AWS / mainnet RPC in any test.**
- **No real production-EVM addresses in any test.**
- Local: use `EXECUTOR_PRIVATE_KEY=<anvil[0]>` (in-process key acceptable on local).
- Sepolia: use a rehearsal-only Sepolia key (operator-managed; not committed).
- Mainnet shape verification: keep one mainnet-config unit test that asserts `validate_signer_backend` refuses `EXECUTOR_PRIVATE_KEY` when `chainId = 8453` (defence-in-depth pin).

## 9. Pass-fail summary

| Track | M-P4 (local) target | M-P5 (Sepolia) target |
|---|---|---|
| Orderbook lifecycle | green | green |
| RFQ lifecycle | green | green |
| Wallet lifecycle | green | green |
| Tx-status lifecycle | green | green |
| Reconciliation | drift = 0 | drift = 0 |
| Failure cases | every case mapped to expected response | every case mapped to expected response |

## 10. Cross-links

- `PRODUCT_READINESS_ROADMAP.md`
- `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `TRADING_INTERFACE_REQUIREMENTS.md`
- `NEXT_PRODUCT_MILESTONES.md`
- `MAINNET_GO_NO_GO_CRITERIA.md` (for the mainnet-shape defence-in-depth pin)

**End of E2E trading lifecycle test plan.**
