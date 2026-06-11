# BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT (M-P2f)

**Date:** 2026-06-10
**Milestone:** `BACKEND-PUBLIC-CREATE-INTENT-ENDPOINT` (M-P2f)
**Posture:** **No mainnet. No broadcast. No signer call. No AWS / KMS.
No `.env` edit. No real funds. No production secrets.**

## 1. Purpose

Close **B7 PUBLIC_CREATE_INTENT_ENDPOINT_PENDING** by adding a
public/user-wallet `POST /options/execution-intents` endpoint. The
frontend M-P3c `CreateIntentButton` already targets this URL and
gracefully degrades to a pending-fallback notice when the endpoint
is absent; this milestone activates the happy path.

## 2. Files changed

| Path | Status |
|---|---|
| `src/options/service.rs` | edited (+`create_user_initiated_execution_intent_from_quote`) |
| `src/api/trading.rs` | edited (+DTOs +handler +24 tests) |
| `src/api/routes.rs` | edited (collapsed GET-only route into GET+POST entry) |
| `docs/openapi/trading-api.openapi.json` | edited (+1 path +2 schemas) |
| `docs/BACKEND_PUBLIC_CREATE_INTENT_ENDPOINT_RESULT.md` | new |
| `docs/FRONTEND_CREATE_INTENT_UX_BACKEND_WIRING_FOLLOWUP.md` | new |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |

## 3. Current B7 gap (resolved)

The frontend `createExecutionIntent(...)` client (M-P3c) treats
`HTTP 404 / 405 / 501` on `POST /options/execution-intents` as
`BACKEND_ENDPOINT_PENDING` and surfaces an amber notice + the legacy
intent-id paste path. With M-P2f the endpoint returns `HTTP 200` with
`{intent_id, request_id, status, signing_payload_available, next_step,
expires_at_ms}` — the existing client's `"intent_id" in ok` decoder
maps this to the success path **without any frontend code change**.

## 4. Endpoint implemented

```
POST /options/execution-intents
Content-Type: application/json
(No Authorization header. No admin Bearer.)

Request:
{
  "series_id": "S-1",
  "side": "buy" | "sell",
  "size_1e8": "100000000",
  "price_1e8": "10000",
  "buyer":  "0x…",            // 40-hex EVM address, required
  "seller": "0x…",            // 40-hex EVM address, required
  "quote_id": "…",            // optional, forward-compat hook
  "client_request_id": "…"    // optional correlation id
}

Response (200):
{
  "intent_id": "<uuid>",
  "request_id": "<uuid or echoed client_request_id>",
  "status": "signatures_required",
  "signing_payload_available": true,
  "next_step": "request_signing_payload",
  "expires_at_ms": null
}
```

## 5. Request / response shape

Flat (not the `Envelope<T>` shape) by design — the frontend's
`createExecutionIntent` client looks for `intent_id` on the top-level
object. This preserves wire-shape compatibility with the M-P3c client
and avoids a frontend release coupling.

## 6. Intent store / service integration

The handler delegates to a new service function
`create_user_initiated_execution_intent_from_quote(state, series,
buyer, seller, side, size_1e8, price_1e8)` that:

1. Calls `ensure_enabled(state)` (refuses when options service is
   disabled).
2. Validates buyer + seller via `validate_account` (existing helper).
3. Refuses zero size / zero price / self-trade.
4. Refuses inactive series.
5. Computes nonces via the existing `option_execution_nonces` path
   (non-strict mode falls back to (0, 0) when RPC unconfigured).
6. Builds the intent via the existing `build_option_execution_intent`
   primitive — same call site as
   `create_option_orderbook_execution_intent`. The intent is
   `OptionExecutionSourceType::OptionOrderbookFill` with a synthetic
   `source_id` (uuid).
7. Inserts via `insert_option_execution_intent` into either
   `PgRepository::insert_option_execution_intent` (when persistence
   enabled) or the in-memory `OptionSeriesStore`.
8. Returns the inserted `OptionExecutionIntent` (status =
   `SignaturesRequired`).

## 7. Signing payload compatibility

The intent is written to the same `option_execution_intents` table
the existing `GET /options/execution-intents/:intent_id/signing-payload`
+ `POST /options/execution-intents/:intent_id/signatures` endpoints
read from. The downstream signing flow is **untouched**:

* The user receives `intent_id` from the create response.
* Frontend calls `GET /options/execution-intents/:intent_id/signing-payload`
  to retrieve the EIP-712 envelope.
* User signs via wallet.
* Frontend posts the signature to
  `POST /options/execution-intents/:intent_id/signatures`.
* Operator-side broadcast pipeline takes over.

## 8. Tx status compatibility

The intent's status starts at `SignaturesRequired`. The existing
`GET /options/execution-intents/:intent_id` + `GET /executor/transactions/:intent_id`
endpoints surface the status to the frontend's `useTxStatus` hook
unchanged. The M-P4c local-test cycler (which uses a SEPARATE
in-memory store) is **untouched** — the production tx-status read
path and the cycler remain isolated by design.

## 9. Public / admin boundary

The endpoint is wired without any admin middleware:

```rust
.route(
    "/options/execution-intents",
    post(crate::api::trading::create_execution_intent)
        .get(list_option_execution_intents),
)
```

The route does **not** start with `/admin/`, so the
`admin_route_gate` middleware does not apply. The handler signature
extracts only `State<AppState>` + `Json<CreateExecutionIntentRequest>`
— no `HeaderMap`, no `AdminIdentity` extractor. Asserted by
`create_intent_endpoint_has_no_admin_bearer_requirement` (a bare call
without any auth headers produces a successful response).

The pre-existing admin/test endpoints (`/admin/test/execution-intents`
from M-P4c) remain admin-Bearer-gated; their behaviour is unchanged.

## 10. Tests added (24)

| Test | Coverage |
|---|---|
| `create_intent_success_returns_intent_id_and_request_id` | happy path |
| `create_intent_echoes_client_request_id_when_provided` | correlation id |
| `create_intent_rejects_invalid_side` | side ∉ {buy,sell} |
| `create_intent_rejects_zero_size` | size_1e8 = 0 |
| `create_intent_rejects_zero_price` | price_1e8 = 0 |
| `create_intent_rejects_non_numeric_size` | size_1e8 = "lots" |
| `create_intent_rejects_missing_buyer` | required field |
| `create_intent_rejects_missing_seller` | required field |
| `create_intent_rejects_malformed_buyer_address` | bad hex |
| `create_intent_rejects_malformed_seller_address` | bad hex |
| `create_intent_rejects_self_trade` | buyer == seller |
| `create_intent_unknown_series_returns_series_not_found` | series_id unknown |
| `create_intent_inactive_series_returns_quote_unsupported` | Disabled status |
| `create_intent_persists_intent_via_signing_payload_consumer` | intent visible to downstream readers |
| `create_intent_response_body_never_leaks_secrets` | secret-leak guard |
| `create_intent_status_starts_at_signatures_required` | initial state |
| `create_intent_quote_id_is_accepted_but_ignored` | forward-compat hook |
| `create_intent_does_not_set_calldata_or_signatures` | safety guard |
| `create_intent_endpoint_has_no_admin_bearer_requirement` | public surface |
| `create_intent_size_too_large_is_rejected_safely` | u128::MAX, no panic |
| `create_intent_returns_distinct_intent_ids_per_request` | uuid uniqueness |

(Three M-P4c tests previously labelled `http_create_intent_*` still
pass — those probe the M-P4c local-test fixture, not this public
endpoint. The two surfaces are isolated.)

## 11. Tests run

| Check | Result |
|---|---|
| `cargo build --lib` | exit 0 |
| `cargo test --lib --no-fail-fast` | **947 passed** (was 926 at M-P3c → +21 net new in the public surface; +3 in the M-P4c surface already existed) |
| `cargo test --all-targets --no-fail-fast` | **1203 passed** (was 1182 → +21) |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --no-deps --all-features -- -D warnings` | clean |
| `python3 -m json.tool docs/openapi/trading-api.openapi.json` | valid JSON |
| `git diff --check` | clean |
| Sensitive-string scan | zero leaks |

## 12. OpenAPI / docs

`docs/openapi/trading-api.openapi.json` — added 1 path (`POST
/options/execution-intents`) + 2 schemas
(`CreateExecutionIntentRequest`, `CreateExecutionIntentData`). The
existing GET on the same path is preserved (Axum's `.get().post()`
chaining wires both verbs on a single route entry).

## 13. RUN_STATE update

`/home/corio/DEOPT/RUN_STATE.md` — M-P2f closure paragraph prepended.

## 14. Files changed (full list)

See Section 2.

## 15. Validations

See Section 11.

## 16. Blockers

| Blocker | Status |
|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | closed (M-P4c) |
| B2 ON_CHAIN_RPC_NOT_WIRED | closed (M-P2e) |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | closed (M-P3c) |
| B4 NO_TEST_FRAMEWORK | partially closed (M-P4b) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | closed (M-P4c) |
| B6 LOGO_NOT_IN_NAV | closed (M-P4b) |
| B7 PUBLIC_CREATE_INTENT_ENDPOINT_PENDING | **CLOSED (this milestone)** |

## 17. Next milestone recommendation

**Recommended next:** `E2E-SEPOLIA-TRADING-LIFECYCLE` (M-P5). All
blockers gating M-P5 are now closed. The Sepolia rehearsal is
dry-run first, operator-approval-gated for any live broadcast. Brief
at `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK.md`.

Optional intermediate task: a small frontend follow-up to switch the
Playwright `create-intent.spec.ts` route-intercept from `404` to
`200` (now that the backend supports the success path). Documented at
`docs/FRONTEND_CREATE_INTENT_UX_BACKEND_WIRING_FOLLOWUP.md`.

## 18. Cross-links

* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e)
* `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (M-P4c)
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_CREATE_INTENT_UX_RESULT.md` (M-P3c)
* `~/DEOPT/deopt-v2-frontend/docs/TRADING_CREATE_INTENT_FLOW_RUNBOOK.md` (M-P3c)
* `docs/openapi/trading-api.openapi.json` (M-P2f update)
* `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK.md` (M-P5 brief)

**End of M-P2f result.**
