# FRONTEND_CREATE_INTENT_UX — Next Task (M-P3c)

**Date written:** 2026-06-10
**Origin milestone:** M-P2e (`BACKEND-TRADING-API-IMPLEMENTATION-PHASE-5`)
closed B2 (`ON_CHAIN_RPC_NOT_WIRED`).
**Target milestone:** `FRONTEND-CREATE-INTENT-UX` (M-P3c).
**Posture:** local-only. **No mainnet. No live tx. No real signing.
No admin Bearer in trading UI. No `.env` edit.**

---

## 1. Background — what M-P2e shipped

* **6 trading endpoints fully wired** to the read-only
  `trading_views` orchestration surface.
* **Env-loader keys added** for the five public contract addresses
  required by the reads:
  - `OPTION_MARGIN_ENGINE_LENS_ADDRESS`
  - `OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS`
  - `OPTION_COLLATERAL_VAULT_ADDRESS`
  - `OPTION_ORACLE_ROUTER_ADDRESS`
  - `OPTION_MARGIN_ENGINE_ADDRESS`
* **B2 ON_CHAIN_RPC_NOT_WIRED — CLOSED.**

All status transitions:

| Endpoint | When backend has addrs + RPC | When config gap |
|---|---|---|
| series_details | `ok` (oracle mark populated) | `partial` + CONFIG_MISSING/ORACLE_UNAVAILABLE |
| quote_preview / close_preview | `partial` + oracle mark populated | `partial` + ORACLE_UNAVAILABLE |
| account_positions | `ok` (per-position mark) | `partial` + CONFIG_MISSING |
| account_portfolio | `ok` (equity/im/mm/free) | `partial` + CONFIG_MISSING |
| exercise_preview | `ok` (full settlement breakdown) | `partial` + SETTLEMENT_PREVIEW_UNAVAILABLE |

## 2. Goal of M-P3c

Add a create-intent UI flow to the trading frontend so the user can
mint an execution intent directly from a quote preview, without
pasting a uuid. Closes B3 (`FRONTEND_CREATE_INTENT_UX_MISSING`).

## 3. Scope

* **In scope:**
  - New `<CreateIntentButton>` in the trade ticket / RFQ card.
  - Wire the existing `POST /options/intents/create-from-quote`
    endpoint (or its current equivalent — see Section 4).
  - Surface the returned `intent_id` to the existing
    `useSigningPayload` + `useTxStatus` hooks so the signing modal
    opens automatically.
  - Optimistic UI: spinner + "Creating intent…" while the POST is
    in-flight.
  - Error envelope rendering: `INVALID_REQUEST`,
    `SERIES_NOT_FOUND`, `INSUFFICIENT_BALANCE`, `INSUFFICIENT_COLLATERAL`,
    `QUOTE_UNSUPPORTED`, `QUOTE_STALE`.

* **Out of scope:**
  - Modifying any backend route handler.
  - Adding chain interaction beyond the existing `wallet.signTypedData`
    flow.
  - Any admin / `/admin/test/*` endpoint usage in the trading UI.

## 4. Backend surface to consume

The backend already exposes `POST /options/execution-intents` (via
the option execution intent service). Verify the exact path + body
shape against `docs/openapi/trading-api.openapi.json` before wiring
— if the endpoint is named differently, **stop and ask**; do not
invent a route.

Expected request body:

```json
{
  "series_id": "S-1",
  "side": "buy" | "sell",
  "size_1e8": "100000000",
  "price_1e8": "500000000",
  "buyer": "0x…",
  "seller": "0x…"
}
```

Expected response envelope:

```json
{
  "status": "ok",
  "data": { "intent_id": "<uuid>", "status": "CREATED" },
  "warnings": [],
  "meta": { "chain_id": …, "request_id": "…", "generated_at_ms": … }
}
```

## 5. Forbidden

* NO `Authorization: Bearer …` header from the trading UI.
* NO admin Bearer token in any UI code path.
* NO mainnet RPC, no Sepolia broadcast, no real wallet private key.
* NO direct frontend broadcast.
* NO production endpoint mutation — wire only.
* NO assumption the backend is on mainnet — `chains.ts` already
  hard-gates this.

## 6. UI sketch

In the existing trade ticket (`src/app/(trading)/markets/[productId]/page.tsx`):

```tsx
<CreateIntentButton
  series={series}
  side={side}
  size={size}
  price={price}
  onIntentCreated={(intentId) => {
    setSigningIntentId(intentId); // existing useSigningPayload hook handle
  }}
/>
```

In a new `src/components/trading/CreateIntentButton.tsx`:

```tsx
"use client";
export function CreateIntentButton({ … }) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<TradingApiError | null>(null);
  const { account } = useWallet();

  async function onClick() {
    setPending(true);
    setError(null);
    try {
      const resp = await createExecutionIntent({
        series_id: series.series_id,
        side,
        size_1e8: size,
        price_1e8: price,
        buyer: side === "buy" ? account : counterparty,
        seller: side === "sell" ? account : counterparty,
      });
      onIntentCreated(resp.data.intent_id);
    } catch (e) {
      setError(e as TradingApiError);
    } finally {
      setPending(false);
    }
  }
  // … render
}
```

## 7. Tests

* Playwright dual-mode (re-use the M-P4c fixture pattern):
  - Backend fixture mode: drive a real create through the backend,
    assert the signing modal opens.
  - Fallback mode: route-intercept `POST /options/execution-intents`
    with a synthetic intent_id.
* Unit tests for the create-intent hook:
  - Maps each error code to the correct user-facing message.
  - Does not attach an Authorization header.

## 8. Acceptance criteria

* `<CreateIntentButton>` is reachable from the trade ticket UI.
* On click, the button calls `POST /options/execution-intents` (or
  the verified equivalent) and surfaces the response intent_id to the
  signing modal.
* Errors render the existing `<ErrorState>` component with the
  backend's `code` + `message`.
* No `Authorization: Bearer …` header leaks from the trading UI
  (asserted by extending `no-admin-bearer.spec.ts`).
* `npx tsc --noEmit`, `npx eslint src/`, `npx next build` all exit 0.

## 9. Cross-links

* `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e)
* `~/DEOPT/deopt-v2-backend/docs/openapi/trading-api.openapi.json`
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_TRADING_SIGNING_RESULT.md`
* `~/DEOPT/deopt-v2-frontend/docs/TRADING_TX_STATUS_WIRING.md`
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_RESULT.md` (M-P4d)

**End of next-task prompt.**
