# Next-task prompt: FRONTEND-TRADING-MVP-WIRING

Copy/paste this prompt verbatim to initiate M-P3.

---

```
Workspace root is ~/DEOPT.

Execute FRONTEND-TRADING-MVP-WIRING only.

This is M-P3 of the product-readiness roadmap.
The goal is to wire the DeOpt V2 trading MVP frontend against the
backend OpenAPI spec at:

  ~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json

and the frozen Solidity ABI handoff at:

  ~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/

Do not deploy.
Do not broadcast.
Do not send transactions on mainnet.
Default to no live transactions; local + Sepolia harness only at the
explicit step that requires it (which is NOT this milestone — this
milestone wires UI surfaces; M-P4 + M-P5 cover E2E).
Do not create Safe transactions.
Do not create AWS resources.
Do not edit `.env` beyond `.env.example` placeholders.
Do not expose admin Bearer to the trading UI.
Do not import `wagmi`/`viem`/`ethers`/`@web3modal/*` in the admin path.
Do not import production signer addresses.
Do not invent mainnet contract addresses.

Strategic context:

* External audit / bug bounty / contest deferred until M-P7 closure.
* sol product surface frozen at tag `v2-product-freeze-rc1` (commit
  `d133e2c`); ABI handoff at the path above.
* Backend trading API consolidated (M-P2 closed); OpenAPI 3.1 spec at the
  path above; backend implementation pass (M-P2a) runs in parallel.
* Frontend currently has only the admin dashboard
  (`src/app/admin/admin-dashboard.tsx`, 4332 lines) + landing + admin
  RBAC types. **There is NO trading UI. NO wallet integration.** This
  milestone builds it.

Goal:
Wire the frontend trading MVP per
`~/DEOPT/deopt-v2-backend/docs/TRADING_INTERFACE_REQUIREMENTS.md`
and `~/DEOPT/deopt-v2-backend/docs/PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §4`,
consuming the OpenAPI spec + frozen ABI. Land 8 new routes + 12 new
components + a new `src/lib/trading-api.ts` + `src/lib/eip712.ts` +
wallet/network handling + UX state matrix + Strict CSP target +
unaudited/testnet banners.

Required Phase A — inspect current frontend state:
Inspect:

* package.json (verify Next.js + React + TypeScript versions).
* tsconfig + eslint + tailwind config.
* src/app/ (currently only admin/ + root page + layout).
* src/lib/ (currently admin-api.ts + admin-rbac-types.ts only).
* src/types/ (currently admin.ts only).

Confirm:

* No `wagmi` / `viem` / `ethers` / `@web3modal/*` installed yet.
* No `src/components/` directory yet.
* No `src/hooks/` directory yet.

Required Phase B — viem/wagmi decision:
Pick viem@^2 (minimum). Optionally add wagmi@^2 ONLY if a wallet-modal
connector is wanted; otherwise plain EIP-1193 detection is sufficient
for the MVP.

Add to package.json:

* `viem@^2`
* (optional) `wagmi@^2`
* `@tanstack/react-query@^5` for data fetching
* (optional) `zod@^3` for response validation

CI guard expansion:

* `@web3modal/*` MUST remain absent from `src/app/admin/**`.
* `dangerouslySetInnerHTML` MUST remain absent everywhere.
* `wagmi` / `viem` / `ethers` MUST remain absent from `src/app/admin/**`
  (narrow the previous CI guard to the admin path only; trading path
  needs viem).

Required Phase C — TypeScript types from OpenAPI:
Generate TS types from the OpenAPI spec:

  npx openapi-typescript ~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json -o src/lib/trading-api.types.ts

Commit the generated file (or generate at build time; pick one).

Required Phase D — new modules:
Add:

* `src/lib/trading-api.ts` — fetch helpers per endpoint;
  - NO admin Bearer;
  - returns `Envelope<TData>` with status / data / warnings / error / meta;
  - configurable baseURL from `NEXT_PUBLIC_BACKEND_URL` (default `http://localhost:3000`).
* `src/lib/eip712.ts` — typed-data helpers for trade-submit + RFQ-quote
  flows; backend issues the envelope, UI signs via wallet.
* `src/lib/wallet.ts` — provider detection (EIP-1193); React context for
  `{ address, chainId, connect, disconnect }`.
* `src/lib/chain.ts` — chain-id mapping (31337 / 84532 / 8453);
  `ENV_CHAIN_ID` from `NEXT_PUBLIC_CHAIN_ID`.

Required Phase E — hooks:
Add to `src/hooks/` (new dir):

* `useProducts(filter?)`
* `useProductDetails(id)`
* `useProductsBatch(ids)`
* `useSeriesDetails(id)`
* `useOrderbook(id)`
* `useQuotePreview(args)`
* `usePositions(address)`
* `usePortfolio(address)`
* `useBalances(address)`
* `useTradeHistory(address, filter?)`
* `useExecutionIntent(id)`
* `useTxStatus(intentId)`
* `useExercisePreview(args)`
* `useClosePreview(args)`
* `useTradingHealth()`

All hooks return `{ data, error, isLoading, refetch }` per the shape in
`~/DEOPT/deopt-v2-backend/docs/FRONTEND_TRADING_API_HANDOFF.md §11`.
Polling intervals per §6 of the same doc.

Required Phase F — components:
Add to `src/components/` (new dir tree per
`~/DEOPT/deopt-v2-backend/docs/PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md §4.6`):

* `wallet/WalletConnect.tsx`
* `wallet/NetworkBanner.tsx`
* `trading/MarketSelector.tsx`
* `trading/OptionChain.tsx`
* `trading/OrderbookPanel.tsx`
* `trading/RfqPanel.tsx`
* `trading/TradeTicket.tsx`
* `trading/PositionsTable.tsx`
* `trading/PortfolioSummary.tsx`
* `trading/ExerciseAction.tsx`
* `trading/CloseAction.tsx`
* `trading/BalancePanel.tsx`
* `trading/DepositWithdrawWidget.tsx`
* `trading/HistoryTable.tsx`
* `tx/TxStatusDetail.tsx`
* `tx/TxLifecycleTimeline.tsx`
* `ui/Toast.tsx`
* `ui/ConfirmModal.tsx`
* `ui/LoadingState.tsx`
* `ui/ErrorToast.tsx`
* `ui/UnauditedWarningBanner.tsx` (sticky; cannot be dismissed)

Required Phase G — routes:
Add under `src/app/(trading)/`:

* `page.tsx` → "/" landing with MarketSelector
* `markets/page.tsx`
* `markets/[underlying]/page.tsx`
* `markets/[underlying]/[expiry]/[strike]/[cp]/page.tsx` → trade ticket
* `positions/page.tsx`
* `history/page.tsx`
* `account/page.tsx`
* `tx/[intent_id]/page.tsx`

Update `src/app/layout.tsx`:

* Add top nav with wallet button + network banner.
* Add sticky `UnauditedWarningBanner`.
* Initialise the wallet context provider.
* Initialise the react-query provider.

DO NOT touch `src/app/admin/` source (admin scope unchanged in this
milestone; V2G-W3 SSR proxy closure can land in parallel).

Required Phase H — UX state coverage:
Implement the 11 UX states per
`~/DEOPT/deopt-v2-backend/docs/TRADING_INTERFACE_REQUIREMENTS.md §6`:

* loading / pending-signature / pending-transaction / confirmed /
  failed / rejected / stale-quote / insufficient-collateral /
  network-mismatch / signer-or-RPC-unavailable / wallet-disconnected.

Each is a deterministic visual + behavioural state with a clear source
(hook result or wallet provider state).

Required Phase I — tests:
Add Playwright + viem tests under `tests/e2e/`:

* connect-wallet smoke (using an in-test mocked EIP-1193 provider).
* market selector renders products from mock backend.
* trade ticket renders quote preview from mock backend.
* positions table renders from mock backend.
* exercise action posts exercise/preview and renders.
* error toast appears on `INVALID_ADDRESS`.
* network-mismatch banner appears when chain_id mismatched.
* unaudited banner persists across navigation.

Mock backend posture for these tests: run prism (`npx @stoplight/prism
mock ~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`)
against the spec; do NOT hit a real backend in these tests.

Required Phase J — Strict CSP target:
Land a Strict CSP target via `next.config.ts` headers:

* `default-src 'self'`
* no `'unsafe-inline'`, no `'unsafe-eval'`
* `connect-src 'self' <NEXT_PUBLIC_BACKEND_URL>`
* `script-src 'self'`
* `img-src 'self' data:`
* `frame-ancestors 'none'`

Add a CI smoke test that fetches the landing page and asserts the CSP
header presence.

Required Phase K — V2G-W3 SSR proxy plan:
Document (do not yet implement) the V2G-W3 SSR proxy + OIDC/MFA closure
plan in `docs/FRONTEND_V2G_W3_SSR_PROXY_PLAN.md`. Implementation lands
in a parallel milestone.

Required Phase L — package.json scripts:
Update scripts:

* `dev` — Next dev server.
* `dev:mock-backend` — `prism mock ...` against the OpenAPI spec.
* `build` — Next build.
* `start` — Next start.
* `lint` — eslint.
* `test:e2e` — Playwright.
* `test:e2e:install` — `playwright install` (one-time).

Required Phase M — RUN_STATE + result doc:
Create `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_TRADING_MVP_WIRING_RESULT.md`:

* dependencies added.
* routes added.
* components added.
* hooks added.
* CI guards updated.
* test count.
* known limitations.
* next milestone routing to M-P4.

Update `~/DEOPT/RUN_STATE.md` with one concise closure paragraph:

* trading MVP wired.
* viem (+ optional wagmi) added.
* 8 routes + 12 components + N hooks + trading-api lib + eip712 lib.
* unaudited banner sticky.
* tests added.
* next milestone routing.

No secrets.

Validation:

* `npm install` clean.
* `npm run build` clean.
* `npm run lint` clean.
* `npm run test:e2e` (against the mock backend) green.
* CI guards: `@web3modal/*` absent everywhere; `dangerouslySetInnerHTML`
  absent everywhere; admin path absent `wagmi`/`viem`/`ethers`.
* `git diff --check` clean.
* `git status` shows expected new files only.
* Sensitive-string scan: no production EVM addresses; no real RPC URLs;
  no admin Bearer tokens; no DATABASE_URL.

Forbidden:

* no mainnet tx;
* no Sepolia live tx (mock backend only);
* no live broadcast;
* no Safe tx;
* no governance mutation;
* no ownership transfer;
* no guardian mutation;
* no Timelock mutation;
* no fee withdrawal;
* no rebate allocation;
* no fund movement;
* no `.env` edit beyond `.env.example` placeholders;
* no real AWS account creation;
* no real AWS KMS key creation;
* no deployment;
* no canary;
* no private key / admin token / RPC secret / DATABASE_URL / API key
  output;
* no AWS credentials;
* no real AWS account IDs;
* no real KMS key IDs;
* no real KMS ARNs;
* no guessed mainnet executor address;
* no private custody roster disclosure;
* no production signer address guess;
* no invented mainnet deployed contract addresses;
* no audit-started claim;
* no audited claim;
* no admin Bearer attached to trading endpoints;
* no `@web3modal/*` in admin path;
* no `dangerouslySetInnerHTML` anywhere;
* no Solidity modification;
* no backend modification beyond consuming the OpenAPI spec.

Hard stops:

* stop if a task would require a real transaction;
* stop if a task would require a Safe transaction;
* stop if a task would require AWS resource creation;
* stop if a task would require editing `.env` with real values;
* stop if a task would require revealing a secret;
* stop if a trading endpoint requires admin Bearer;
* stop if changing backend or sol is required (escalate as a separate
  M-P2a / sol-bugfix milestone);
* stop if the OpenAPI spec is missing a field needed by a component
  (escalate as a separate M-P2a milestone; do not invent fields
  client-side);
* stop if test coverage cannot be added for a new component;
* stop if CSP target would break existing admin functionality.

Return final report grouped by:
workspace,
repos inspected,
viem / wagmi decision,
generated types status,
routes added,
components added,
hooks added,
new lib modules,
wallet context,
CSP target,
CI guards,
mock backend setup,
tests added,
tests run,
RUN_STATE update,
files changed,
validations,
blockers,
next milestone recommendation.
```

---

**End of next-task prompt.**
