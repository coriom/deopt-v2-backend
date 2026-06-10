# Next-task prompt: E2E-LOCAL-TRADING-FIXES (M-P4b)

Copy/paste this prompt verbatim to initiate M-P4b.

---

```
Workspace root is ~/DEOPT.

Execute E2E-LOCAL-TRADING-FIXES only.

This is M-P4b of the product-readiness roadmap. The goal is to close
two M-P4-identified blockers so the local trading E2E sweep can run
fully automated:
  - B4 NO_TEST_FRAMEWORK — install Playwright in the frontend +
    add 6 smoke tests against a Prism mock backend.
  - B5 BACKEND_TX_STATUS_FIXTURE_MISSING — add a deterministic
    operator-only mock-status cycler endpoint so Playwright can
    drive intents through every state.

Hard prerequisites:
  - M-P4 closed (see E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md).

Do not deploy.
Do not broadcast.
Do not send live transactions.
Do not create Safe transactions.
Do not create AWS resources.
Do not edit production `.env`.
Do not expose secrets.
Do not touch mainnet.
Do not expose admin Bearer to trading UI.
Do not call the new test-cycler endpoint from a production code path.

Strategic context:

External audit deferred until M-P7 closure. M-P4b closes the local
E2E gap so M-P5 (Sepolia E2E) can adopt the same Playwright harness
against a live Sepolia rehearsal backend.

Goal:

Phase A — frontend: add Playwright + viem injected-EIP-1193 fixture.
Add the following packages as devDependencies in
`~/DEOPT/deopt-v2-frontend/package.json`:
  - `@playwright/test`
  - (nothing else; viem is already a runtime dep)
Add the following scripts:
  - `test:e2e` → `playwright test`
  - `test:e2e:install` → `playwright install chromium`
Create `playwright.config.ts` with:
  - `testDir: "tests/e2e"`,
  - `use.baseURL: process.env.E2E_BASE_URL ?? "http://localhost:3000"`,
  - chromium only,
  - 1 worker,
  - timeout 30 000 ms.
Create `tests/e2e/wallet-fixture.ts`:
  - viem `privateKeyToAccount(...)` from a hard-coded anvil[1] test
    private key (deterministic; well-known; not a real key);
  - wraps the account into an EIP-1193-compatible provider object
    that Playwright injects into the browser context via
    `addInitScript`;
  - signs typed-data via the in-process viem account so no real
    wallet popup is required during tests.

Phase B — 6 Playwright smoke tests under `tests/e2e/`:
  1. `landing.spec.ts` — `page.goto("/")` → expect MarketSelector
     visible; expect TestnetUnauditedBanner sticky.
  2. `markets.spec.ts` — `page.goto("/markets")` → expect at least
     one product card visible (from Prism mock).
  3. `portfolio-disconnected.spec.ts` — `page.goto("/portfolio")`
     without injected wallet → expect "Connect your wallet"
     EmptyState.
  4. `tx-status-fallback.spec.ts` — `page.goto("/transactions/test-id")`
     → expect timeline visible + footer with `intent_id: test-id`.
  5. `mainnet-disabled.spec.ts` — inject wallet reporting chainId
     8453 → expect `MainnetDisabledBanner` red sticky visible;
     expect signTypedData refusal (network intercept on
     `/signing-payload`).
  6. `no-admin-bearer.spec.ts` — network intercept on
     `/options/products` GET → expect NO `Authorization` header
     present.
Mock backend posture: each spec starts Prism via a Playwright global
setup hook (`globalSetup`) and shuts down via teardown.

Phase C — backend: add operator-only mock-status cycler endpoint.
Add `POST /admin/test/intent/:intent_id/transition` in
`~/DEOPT/deopt-v2-backend/src/api/routes.rs` (under the existing
`/admin/*` admin-gated group):
  - body: `{ to: "CREATED"|"SIGNING_PAYLOAD_ISSUED"|"SIGNED"|
    "SIMULATED_OK"|"BROADCAST"|"CONFIRMED"|"REVERTED"|"STUCK",
    reason?: string }`,
  - mutates the in-memory execution-intent store directly,
  - returns `{ intent_id, status }`,
  - admin Bearer required (existing middleware enforces this),
  - REFUSES on mainnet — startup check + per-request guard
    (chain_id == 8453 → 403 Forbidden + `MAINNET_TEST_REFUSED`
    error code; LOG the attempt).
Add 8 unit tests:
  - happy-path each terminal transition (CONFIRMED / REVERTED / STUCK);
  - rejects unknown intent → 404;
  - rejects invalid status value → 400;
  - rejects on mainnet chain_id (defence-in-depth) → 403;
  - rejects without Bearer → 401;
  - asserts no signer call;
  - asserts no broadcast;
  - asserts response body carries no secrets.

Phase D — wire B4 + B5 together:
Playwright `mainnet-disabled.spec.ts` exercises Phase C as well —
asserts that calling `/admin/test/intent/<id>/transition` against a
backend running with `CHAIN_ID=8453` returns 403 (defence-in-depth
verification).

Phase E — docs:
Create `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_FIXES_RESULT.md`:
  - Playwright config + harness + 6 specs.
  - Backend cycler endpoint + tests + mainnet refusal proof.
  - Updated test counts (1068 backend + 6 frontend Playwright).
  - How to run locally: `cd deopt-v2-backend && cargo run` +
    `cd deopt-v2-frontend && npm run test:e2e:install && npm run test:e2e`.
  - Blockers remaining for M-P5.

Update `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md`:
  - Mark B4 + B5 as closed.

Phase F — RUN_STATE:
Update `~/DEOPT/RUN_STATE.md` with one concise closure paragraph:
  - Playwright + cycler landed.
  - Test counts.
  - Mainnet refusal proof.
  - Next routing.
No secrets.

Validation:

* `npm install` in frontend (only deps added are
  `@playwright/test`).
* `npx playwright install chromium` (one-time).
* `cargo fmt --check`, `cargo clippy --all-targets --all-features
  -- -D warnings`, `cargo test --all-targets --no-fail-fast`.
* `npx tsc --noEmit`, `npx eslint`, `npx next build`.
* `npm run test:e2e` against running Prism mock + frontend.
* `git diff --check`, `git status`.
* Sensitive-string scan on new tests + cycler endpoint.

Forbidden:
  - no mainnet tx;
  - no Sepolia live tx;
  - no live broadcast;
  - no Safe tx;
  - no governance mutation;
  - no fund movement;
  - no production `.env` edit;
  - no AWS resource creation;
  - no KMS key creation;
  - no real AWS account IDs / KMS key IDs / KMS ARNs;
  - no guessed mainnet executor address;
  - no production signer address guess;
  - no audited claim;
  - no mainnet-ready claim;
  - no admin Bearer in trading-UI XHRs;
  - no @web3modal/* anywhere;
  - no auto-signing in Playwright tests beyond the in-process viem
    account used as a fixture;
  - no real wallet popup in CI.

Hard stops:
  - stop if the cycler endpoint can be called without admin Bearer;
  - stop if the cycler endpoint accepts mainnet chain_id;
  - stop if Playwright tests trigger a real wallet popup (CI must
    use the injected viem fixture);
  - stop if `cargo test` fails;
  - stop if `npx next build` fails;
  - stop if `git diff --check` shows mixed-tabs/spaces issues.

Return final report grouped by:
workspace,
docs/sources inspected,
playwright config + fixture,
playwright specs,
backend cycler endpoint,
backend cycler tests,
mainnet refusal proof,
files changed,
tests run,
docs created,
RUN_STATE update,
validations,
blockers,
next milestone recommendation (M-P2c or M-P5).
```

---

**End of next-task prompt.**
