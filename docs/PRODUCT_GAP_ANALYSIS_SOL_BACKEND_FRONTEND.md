# DeOpt V2 — Product Gap Analysis (Sol / Backend / Frontend)

**Date:** 2026-06-10
**Posture:** docs-only inventory. **No source code modified.**
Companion to `PRODUCT_READINESS_ROADMAP.md`.

## 1. Inspection summary

| Layer | State | Quick read |
|---|---|---|
| Solidity | mature | ~69 production contracts; 30+ deploy scripts; 28 test files (unit / invariant / fuzz / scenario / fork); orderbook + RFQ surface live on Sepolia smoke. |
| Backend | mature for executor / observability / lifecycle | ~1053 unit tests green; ~80 HTTP routes; full orderbook + RFQ + execution-intent + tx-status + indexer + reconciliation + admin surfaces; AWS KMS adapter behind Cargo feature; `/executor/health/v2` health endpoint. |
| Frontend | **admin-only** | ONLY admin dashboard (`src/app/admin/admin-dashboard.tsx`, 4332 lines) + production-readiness section (566 lines) + root page (11 lines). **NO trading UI. NO wallet integration. NO `components/` directory. NO wagmi / viem / ethers.** This is the dominant product gap. |

## 2. Solidity — contracts ready

All ~69 production contracts compile + tests pass.

### 2.1 Ready for product use

| Module | Contracts | Notes |
|---|---|---|
| Collateral | `CollateralVault*.sol` (6 files), `AaveAdapter.sol` (yield) | Deposit / withdraw / internal transfer; R5 invariant pinned. |
| Risk | `RiskModule*.sol` (10 files), `PerpRiskModule.sol` | Margin / collateral / oracle / utils / admin / views. |
| Options | `OptionProductRegistry.sol`, `MarginEngine*.sol` (9 files) | Options matching consumer of MatchingEngine. |
| Perps | `PerpEngine*.sol` (8 files), `PerpMatchingEngine.sol` | NOT_APPLICABLE_AT_LAUNCH; in repo but excluded from product MVP per Q-CD-6. |
| Matching | `OptionMatchingEngine.sol` (special-attention) | EIP-712 orderbook + RFQ; signature verification; nonce; `BadNonce()` / `InvalidSignature()` selectors. |
| Fees | `FeesManagerV2.sol`, `ProtocolFeeVault.sol` | Signed-ppm; rebate-DEFERRED launch invariant. |
| Insurance | `InsuranceFund.sol` | Independent balance counter. |
| Oracle | `OracleRouter.sol`, `ChainlinkPriceSource.sol`, `PythPriceSource.sol`, `PeggedStablePriceSource.sol` | Cross-source-check + staleness. |
| Governance | `ProtocolTimelock.sol`, `RiskGovernor*.sol` | 2-step Ownable. |
| Liquidation | `CollateralSeizer.sol` | Q-CD-12 dependent. |

### 2.2 Functions needed by frontend (UI-facing view functions)

These should be confirmed present + stable in M-P1:

| Surface | View functions needed | Status |
|---|---|---|
| Markets | `OptionProductRegistry.getProduct(productId)`; enumerable product list | likely present; M-P1 confirms |
| Series | `MarginEngine.optionSeries(seriesId)`; series metadata view | M-P1 confirms |
| Quotes | `OracleRouter.price(underlying)`; expiry-aware view | M-P1 confirms |
| Positions | `MarginEngine.position(account, seriesId)`; aggregated equity / IM / MM views via `RiskModule.account*` | M-P1 confirms |
| Account state | `CollateralVault.balanceOf(account, token)`; free collateral view | M-P1 confirms |
| Fees | `FeesManagerV2.previewFee(quote)` for trade preview | M-P1 confirms |

### 2.3 Events needed by backend / indexer

Verified present (event indexer consumes them):

- `Filled` (orderbook fill);
- `RfqFilled` (RFQ fill);
- `BadNonce()` (selector `0x4bd574ec`);
- `InvalidSignature()` (selector `0x8baa579f`);
- `RebateFunded` / `RebatePaid` (INACTIVE at launch).

Likely missing (M-P1 confirms / adds):

- A unified `OrderbookEventsForUI` event tap that includes the EIP-712 hash of the matched order, so the indexer can correlate to the off-chain order.
- An `OptionSeriesCreated` event for the indexer to detect new series listings without polling.
- A `PositionSettled` event for exercise / expiry consumption.

### 2.4 Lifecycle coverage (per option)

| Step | Sol surface | State |
|---|---|---|
| Create market / series | `OptionProductRegistry.addProduct` | implemented |
| Quote | `OracleRouter.price` + `FeesManagerV2.previewFee` | partial — UI-facing combined view function may be missing |
| Buy | `OptionMatchingEngine.executeMatch` (orderbook) / `OptionMatchingEngine.executeRfq` (RFQ) | implemented |
| Sell | same surfaces | implemented |
| Fill | `Filled` / `RfqFilled` event | implemented |
| Position open | `MarginEngine` opens position record on fill | implemented |
| Exercise | `MarginEngine` exercise path | M-P1 confirms |
| Close / resell | `MarginEngine` close path | M-P1 confirms |
| Liquidation | `CollateralSeizer` + `MarginEngineLiquidationLib` | implemented (untested on Sepolia smoke) |
| Settlement / expiry | `MarginEngine` settle path | M-P1 confirms |

### 2.5 Test coverage gaps

| Area | Existing | Gap |
|---|---|---|
| Unit | risk / liquidation / matching / vault / governance / margin / fees / perp / scripts | ✓ |
| Invariant | liquidation / vault / position index | Cluster 4 launch invariant verifier not in suite |
| Fuzz | options margin engine; perp engine | option pricing / settlement fuzz absent |
| Scenario | option settlement flow; PFV integration; bad debt repayment; oracle failure; perp full liquidation | end-to-end orderbook + RFQ + lifecycle scenario absent |
| Fork | FM_V2 option fork; MarginEngineV2 rewire | mainnet-fork scenario suite deferred (separate from product) |

### 2.6 What should be frozen before frontend wiring

- View-function shape: every endpoint above MUST have stable return types and stable storage layout.
- Event field order: any change after freeze cascades to indexer + frontend.
- ABI: foundry-generated ABIs at the freeze commit are the canonical UI contract.
- `MAX_TAKER_FEE_PPM` / `MAX_MAKER_REBATE_PPM` constants: stable.
- Selectors: every public function selector at freeze becomes the public ABI contract.

## 3. Backend — trading APIs

### 3.1 Existing trading APIs (~80 routes; ~40 trading-relevant)

**Markets / orderbook / series:**
- `GET /markets`
- `GET /orderbook/:market_id`
- `POST /options/series` / `GET /options/series` / `GET /options/series/:id` / `POST /options/series/:id/disable`
- `GET /options/orderbooks/:option_series_id`

**RFQ flow:**
- `POST /options/rfqs` / `GET /options/rfqs`
- `GET /options/rfqs/:id`
- `POST /options/rfqs/:id/quote-signing-payload`
- `POST /options/rfqs/:id/quotes` / `GET /options/rfqs/:id/quotes`
- `POST /options/rfqs/:id/accept/:quote_id`
- `POST /options/rfqs/:id/cancel`

**Orderbook orders:**
- `POST /options/orders` / `GET /options/orders`
- `GET /options/orders/:id`
- `POST /options/orders/:id/cancel`
- `GET /options/orders/:id/fills`

**Execution intents (post-fill broadcast):**
- `GET /options/execution-intents` / `GET /options/execution-intents/:id`
- `GET /options/execution-intents/:id/signing-payload`
- `POST /options/execution-intents/:id/signatures`
- `GET /options/execution-intents/:id/calldata`
- `POST /options/execution-intents/:id/simulate` / `GET /options/execution-intents/:id/simulation`
- `POST /options/execution-intents/:id/broadcast`
- `POST /options/execution-intents/:id/confirm`

**Fills:**
- `GET /options/fills` / `GET /options/fills/:id`

**Account nonces:**
- `GET /accounts/:address/perp-nonce`
- `GET /accounts/:address/option-nonce`

**Executor + health + tx visibility:**
- `GET /executor/status` / `GET /executor/health/v2`
- `POST /executor/tick`
- `GET /executor/transactions` / `GET /executor/transactions/:intent_id`
- `GET /executor/confirmations/status` / `GET /executor/confirmations/:intent_id`
- `POST /executor/confirmations/tick` / `POST /executor/confirm/:intent_id`

**Indexer + reconciliation:**
- `GET /indexer/status` / `POST /indexer/tick`
- `GET /indexer/perp-trades`
- `GET /reconciliation/status` / `POST /reconciliation/tick`
- `GET /reconciliations`
- `GET /reconciliation/intents/:intent_id`

**Admin (~25 endpoints):** `/admin/status`, `/admin/config`, `/admin/db`, `/admin/options/*`, `/admin/mm/*`, `/admin/execution/summary`, `/admin/rfq/summary`, `/admin/options/summary`, `/admin/fees/*`.

**Legacy perp routes (NOT_APPLICABLE_AT_LAUNCH):** `/orders`, `/rfqs/*`, `/execution-intents/*`.

### 3.2 Missing trading APIs

| Endpoint | Why needed | Owner |
|---|---|---|
| `GET /options/products` | Frontend needs an enumerable list of available products (underlying / call/put / expiry inventory) | M-P2 |
| `GET /options/products/:product_id` | Detail view (current series; oracle source; fee profile) | M-P2 |
| `GET /options/quotes/:option_series_id` | UI trade-ticket needs a server-side fee + oracle preview without client RPC | M-P2 |
| `GET /accounts/:address/positions` | UI positions table; aggregated by series with notional / mark / pnl | M-P2 |
| `GET /accounts/:address/portfolio-summary` | UI top-banner: total notional, equity, free collateral, IM, MM | M-P2 |
| `GET /accounts/:address/option-history` | UI history tab: every fill, cancel, exercise, settle for the account | M-P2 |
| `GET /accounts/:address/balances` | UI deposit / withdraw widget; per-token balances | M-P2 |
| `POST /accounts/:address/exercise` | UI exercise action (server constructs the calldata + EIP-712 payload) | M-P2 (could be intent-mediated like execution-intents) |
| `POST /accounts/:address/close` | UI close action (server-side intent construction) | M-P2 |
| `GET /quotes/preview` | Trade-ticket live preview (fee + oracle + slippage + IM impact) | M-P2 |
| `GET /events/sse` or `WS /events` | UI live event stream (fill / cancel / RFQ-quote arrived) | M-P2 (optional; polling acceptable for MVP) |

### 3.3 Required API response schemas

M-P2 produces a typed OpenAPI 3.1 or JSON schema bundle for every trading
endpoint, with explicit:

- types (string, integer, decimal-as-string for chain values);
- u256 / i256 represented as decimal string with documented precision;
- timestamp encoding (RFC3339 + unix-ms);
- error response shape (`{ code, message, details? }`);
- pagination (`{ items, next_cursor, total? }`);
- common headers (`X-Request-Id`, `X-Trace-Id`).

### 3.4 Frontend consumption gaps

The admin frontend uses `src/lib/admin-api.ts` (one file, ~admin only).
For trading, the frontend needs a separate `src/lib/trading-api.ts` that
must NOT use the admin Bearer token; trading calls are user-anchored
(EIP-712 signature on every signing payload returned by backend).

## 4. Frontend — trading readiness

### 4.1 Current pages

Only `src/app/admin/` (admin dashboard) + root `src/app/page.tsx` (11 lines)
+ root `src/app/layout.tsx`. No trading routes.

### 4.2 Missing trading pages

| Route | Purpose | Components |
|---|---|---|
| `/` | Landing + market list | `MarketSelector` |
| `/markets` | Market catalogue | `MarketCard`, `MarketSelector` |
| `/markets/:underlying` | Option chain for an underlying (e.g. `/markets/ETH`) | `OptionChain`, `ExpirySelector`, `StrikeSelector`, `CallPutToggle` |
| `/markets/:underlying/:expiry/:strike/:cp` | Trade ticket for a specific series | `TradeTicket`, `OrderbookPanel`, `RfqPanel` |
| `/positions` | User positions table | `PositionsTable`, `PortfolioSummary`, `ExerciseAction`, `CloseAction` |
| `/history` | User history | `HistoryTable` |
| `/account` | Account state (balances, deposits, withdrawals) | `BalancePanel`, `DepositWithdrawWidget` |
| `/tx/:intent_id` | Transaction status detail | `TxStatusDetail`, `TxLifecycleTimeline` |

### 4.3 Missing components

- `WalletConnect` (provider abstraction; M-P3 picks viem + a small wallet-modal-free connector or wagmi);
- `MarketSelector`;
- `OptionChain` (call/put toggle, expiry/strike grid);
- `OrderbookPanel`;
- `RfqPanel`;
- `TradeTicket` (size, price, slippage, IM impact, preview, sign, broadcast);
- `PositionsTable` + `PortfolioSummary`;
- `ExerciseAction` + `CloseAction`;
- `TxStatusDetail` + `TxLifecycleTimeline`;
- `HistoryTable`;
- `BalancePanel` + `DepositWithdrawWidget`;
- `ErrorToast` / `ConfirmModal` / `LoadingState`;
- `NetworkBanner` (chain-id check; testnet badge);
- `UnauditedWarningBanner` (visible until M-P7 closure + external audit completes).

### 4.4 Wallet / network handling

- Pick one client-side chain library (recommend `viem@^2`; mature, modular, well-typed; pairs with wagmi if a modal connector is wanted but not required).
- Wallet-side EIP-712 signature flow: backend produces the EIP-712 payload (existing endpoints `/options/rfqs/:id/quote-signing-payload`, `/options/execution-intents/:id/signing-payload`); frontend submits a personal-sign-equivalent typed-data request via the connected wallet; signature is posted back to backend.
- Network-id pinning: refuse to enable Trade button if `wallet.chainId != ENV_CHAIN_ID`.
- Stale-quote detection: backend returns a `quote_expires_at_ms`; UI must refuse to broadcast if expired.

### 4.5 Required UX states

| State | UI surface | Detection |
|---|---|---|
| Loading | spinner | route data fetch in flight |
| Pending signature | "approve in your wallet" modal | `awaitingSignature = true` |
| Pending transaction | "broadcast in flight" toast + tx-status link | execution-intent submitted, no confirmation |
| Confirmed | green toast + success state | confirmation worker reports confirmed |
| Failed | red toast + revert reason | backend reports revert |
| Rejected | warn toast + retry | wallet returned reject |
| Stale quote | banner: "Quote expired — refresh" | `quote_expires_at_ms` past |
| Insufficient collateral / balance | inline error + deposit-suggestion CTA | `free_collateral < required` |
| Network mismatch | network-mismatch banner | `wallet.chainId != ENV_CHAIN_ID` |
| Signer / RPC unavailable | "Backend trading service is offline" banner | health endpoint reports unhealthy |
| Wallet disconnected | reconnect CTA | wallet provider missing |

### 4.6 Route structure recommendation

```
src/app/
├── (trading)/
│   ├── page.tsx                                  → "/" landing
│   ├── markets/
│   │   ├── page.tsx                              → "/markets"
│   │   └── [underlying]/
│   │       ├── page.tsx                          → "/markets/:underlying"
│   │       └── [expiry]/[strike]/[cp]/
│   │           └── page.tsx                      → trade ticket
│   ├── positions/page.tsx
│   ├── history/page.tsx
│   ├── account/page.tsx
│   └── tx/[intent_id]/page.tsx
├── admin/                                        → existing admin
├── layout.tsx                                    → top nav + wallet button + network banner
└── globals.css

src/components/
├── trading/
│   ├── TradeTicket.tsx
│   ├── OptionChain.tsx
│   ├── OrderbookPanel.tsx
│   ├── RfqPanel.tsx
│   ├── PositionsTable.tsx
│   └── ...
├── wallet/
│   ├── WalletConnect.tsx
│   └── NetworkBanner.tsx
├── tx/
│   ├── TxStatusDetail.tsx
│   └── TxLifecycleTimeline.tsx
└── ui/
    ├── Toast.tsx
    ├── ConfirmModal.tsx
    └── LoadingState.tsx

src/lib/
├── trading-api.ts        ← NEW — user-anchored; no admin Bearer
├── admin-api.ts          ← existing
└── eip712.ts             ← NEW — typed-data signing helpers
```

### 4.7 Frontend dependency additions (M-P3 picks)

- `viem@^2` (preferred over ethers for typed-data ergonomics + tree-shake);
- Optional `wagmi@^2` if modal connector is wanted;
- `swr@^2` or `@tanstack/react-query@^5` for data fetching (M-P3 picks);
- No `@web3modal/*` in admin path (CI guard remains).

**IMPORTANT:** the admin path's `wagmi`/`viem`/`ethers`/`@web3modal/*` absence CI guard MUST be moved to admin path scope only; trading path needs viem at minimum.

## 5. Documentation gap

Currently committed: `SPEC.md`, `ARCHITECTURE_MAP.md`, `INVARIANTS.md`,
`PARAMETERS.md`, `ROLE_MATRIX.md`, `TEST_MATRIX.md`, `DEPLOYMENT_PLAN.md`,
`README.md` (sol).

Missing for beta:

- Public README (root and per-repo) with non-audited / testnet warning;
- Quickstart;
- Testnet guide (faucet, RPC, connect wallet, deposit, trade);
- User guide (option mechanics, fees, exercise, settlement);
- Market-maker guide (RFQ flow, quote-signing, EIP-712 envelope);
- Developer API docs (OpenAPI export from backend);
- Architecture overview (high-level);
- Risk disclosures;
- FAQ;
- Troubleshooting;
- Known limitations / out-of-scope.

See `PUBLIC_DOCS_BETA_CHECKLIST.md`.

## 6. Existing tracked work to preserve

The audit-side handoff layer (`MAINNET_AUDIT_*_FINAL.md` × 7), AWS KMS
operator setup pack (× 5), mainnet manifest preflight pack (× 6), signer
rehearsal plans, custody cluster closures (× 4), governance migration
plan, custody policy — all preserved. No deletions, no rewrites. They
reactivate at M-P7 closure.

## 7. Cross-links

- `PRODUCT_READINESS_ROADMAP.md`
- `TRADING_INTERFACE_REQUIREMENTS.md`
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md`
- `PUBLIC_DOCS_BETA_CHECKLIST.md`
- `NEXT_PRODUCT_MILESTONES.md`
- `SOL_PRODUCT_SCOPE_FREEZE_AND_VIEW_FUNCTIONS_NEXT_TASK.md`
- `~/DEOPT/deopt-v2-sol/README.md`
- `~/DEOPT/deopt-v2-frontend/FRONTEND_CONTEXT.md`

**End of product gap analysis.**
