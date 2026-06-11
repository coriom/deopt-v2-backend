# Frontend Create-Intent — Backend Wiring Follow-up (M-P2f)

**Date:** 2026-06-10
**Origin:** M-P2f (`BACKEND-PUBLIC-CREATE-INTENT-ENDPOINT`) closed B7.
**Audience:** frontend developers.
**Posture:** local-only. **No mainnet. No live tx. No real wallet.
No admin Bearer.**

## 1. What changed on the backend

`POST /options/execution-intents` is now a public endpoint (M-P2f).
It accepts the exact request shape the M-P3c frontend client already
sends and returns the flat `{intent_id, request_id, status,
signing_payload_available, next_step, expires_at_ms}` envelope the
existing decoder maps to the success path.

## 2. Does CreateIntentButton need a code change?

**No — the M-P3c client surface is forward-compatible with M-P2f.**
The `createExecutionIntent` client (in `src/lib/trading-api.ts`) was
written to handle both `pending` (404/405/501) and `ok` (200 with
`intent_id`) cases. A live backend now returns the `ok` case and
the existing intent-id auto-fill path activates without any frontend
change.

## 3. Request body — is the current shape correct?

**Mostly.** The current frontend body is:

```ts
{
  series_id: seriesId,
  side,
  size_1e8: size,
  price_1e8: price1e8 || "0",
  buyer:  side === "buy"  ? (address ?? undefined) : undefined,
  seller: side === "sell" ? (address ?? undefined) : undefined,
}
```

The backend now requires **both** `buyer` and `seller` — the
counterparty resolver isn't wired yet. So a current frontend request
will be rejected with HTTP 400 `INVALID_REQUEST` ("seller is
required") for a buy-side click and ("buyer is required") for a
sell-side click.

**Two valid responses:**

* **(a) Surface the error as-is**: the existing `onCreateError`
  handler maps `TradingApiError` to the modal's `error` phase. The
  user sees the message and pastes an intent_id via the legacy path.
* **(b) Add a counterparty input**: extend the TradeTicket with a
  "Counterparty address" field that fills the missing field. Cleaner
  UX but more code.

For now (a) is the recommended path — it requires zero frontend code
change and matches the M-P3c brief's "do not invent product surface
without confirmation" guidance.

## 4. Playwright spec follow-up

`tests/e2e/create-intent.spec.ts` currently route-intercepts
`POST /options/execution-intents` with `404` to exercise the pending
path. With M-P2f the live backend would return 200. Two options:

* **Keep the route-intercept**: the spec stays a pure unit test of
  the pending path; documents the legacy behaviour. No change needed.
* **Add a new spec**: route-intercept with `200` + a synthetic
  intent_id and assert the intent-id auto-fills. Useful for the
  success-path UX regression coverage.

The brief for a small follow-up milestone is at
`FRONTEND_CREATE_INTENT_SUCCESS_PATH_NEXT_TASK.md` (next sub-section).

## 5. FRONTEND_CREATE_INTENT_SUCCESS_PATH_NEXT_TASK (optional, ~30 min)

* Add `tests/e2e/create-intent-success.spec.ts` (route-intercept 200).
* No production code change.
* Assert `data-testid="intent-id-input"` value becomes the synthetic
  uuid after the click.
* Assert the SigningStateModal does NOT show `intent_pending`.
* Acceptance: `npx tsc --noEmit && npx eslint && npx playwright test
  --list` all green.

## 6. Forbidden

* No production code change is required for M-P2f to be usable.
* No admin Bearer in the trading UI.
* No mainnet RPC.
* No live tx, no real wallet private key.
* No `.env` edit.

## 7. Cross-links

* `~/DEOPT/deopt-v2-backend/docs/BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` (M-P2f)
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_CREATE_INTENT_UX_RESULT.md` (M-P3c)
* `~/DEOPT/deopt-v2-frontend/docs/TRADING_CREATE_INTENT_FLOW_RUNBOOK.md` (M-P3c)
* `~/DEOPT/deopt-v2-frontend/src/lib/trading-api.ts::createExecutionIntent`

**End of follow-up.**
