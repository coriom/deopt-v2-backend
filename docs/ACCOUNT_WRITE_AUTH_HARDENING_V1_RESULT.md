# ACCOUNT-WRITE-AUTH-HARDENING-V1 — Result

**Status:** CLOSED — every live option-side mutation route requires a cryptographically authorized EIP-712 envelope; every public perp mutation route fails closed before any business logic or persistence.

**Closed:** 2026-06-21
**Branch:** main
**Chain:** Base Sepolia (84532)
**Mainnet enabled:** no
**Blockchain transactions:** none
**Solidity changes:** none
**Secrets exposed:** none

---

## 1. Audited write-route matrix

| # | Method + Path | Surface | Before (V0) | After (V1) |
|---|---|---|---|---|
| **OPTIONS — LIVE** | | | | |
| 1 | `POST /options/orders` | Direct option order | Optional signature accepted but **ignored** | **Mandatory** `OPTION_ORDER_SUBMIT` envelope |
| 2 | `POST /options/orders/:order_id/cancel` | Cancel option order | No auth, no body | **Mandatory** `OPTION_ORDER_CANCEL` envelope; signer must own the order |
| 3 | `POST /accounts/:address/conditional-orders` | Create TP/SL (single or OCO pair) | URL-path check only | **Mandatory** `CONDITIONAL_ORDER_CREATE` envelope binding account, series, OCO link, every leg field |
| 4 | `DELETE /accounts/:address/conditional-orders/:id` | Cancel TP/SL | URL-path check only | **Mandatory** `CONDITIONAL_ORDER_CANCEL` envelope; cross-wallet cancels rejected |
| 5 | `POST /options/rfqs` | Create option RFQ | None | **Mandatory** `OPTION_RFQ_CREATE` envelope (taker signs) |
| 6 | `POST /options/rfqs/:rfq_id/quotes` | MM submit option-RFQ quote | Optional EIP-712 (Strict mode only) | **Mandatory** `OPTION_RFQ_QUOTE_SUBMIT` envelope (MM signs) — existing settlement EIP-712 quote signature is preserved as `signature` field for downstream verification |
| 7 | `POST /options/rfqs/:rfq_id/accept/:quote_id` | Accept quote | None | **Mandatory** `OPTION_RFQ_ACCEPT` envelope (taker signs) |
| 8 | `POST /options/rfqs/:rfq_id/cancel` | Cancel option RFQ | None | **Mandatory** `OPTION_RFQ_CANCEL` envelope (taker signs) |
| 9 | `POST /options/execution-intents/:intent_id/signatures` | Buyer/seller submits settlement signature | Optional settlement EIP-712 (Strict mode only); buyer + seller could be submitted in one request | **Mandatory** `OPTION_EXECUTION_INTENT_SIGNATURE_SUBMIT` envelope per call. Submitter declares role (`buyer` or `seller`), must equal intent.buyer / intent.seller. Cross-role signature in same request rejected |
| **OPTIONS — SYSTEM-LEVEL (not user-account-bound)** | | | | |
| 10 | `POST /options/execution-intents/:intent_id/simulate` | Operator simulates calldata | None | **Unchanged.** Documented as system-level lifecycle, not per-account; defense rests on intent state machine + `calldata_ready` gate |
| 11 | `POST /options/execution-intents/:intent_id/broadcast` | Operator broadcasts to chain | None | **Unchanged.** Same reasoning as `/simulate` |
| 12 | `POST /options/execution-intents/:intent_id/confirm` | Operator records on-chain confirmation | None | **Unchanged.** Same reasoning |
| **PERPS — NON-LIVE, FAIL-CLOSED** | | | | |
| 13 | `POST /orders` | Submit perp order | Optional EIP-712 | **HTTP 503** before any body parse / business logic |
| 14 | `DELETE /orders/:order_id` | Cancel perp order | None | **HTTP 503** |
| 15 | `POST /rfqs` | Create perp RFQ | None | **HTTP 503** |
| 16 | `POST /rfqs/:rfq_id/quotes` | MM submit perp-RFQ quote | Optional EIP-712 | **HTTP 503** |
| 17 | `POST /rfqs/:rfq_id/accept/:quote_id` | Accept perp quote | None | **HTTP 503** |
| 18 | `POST /rfqs/:rfq_id/cancel` | Cancel perp RFQ | None | **HTTP 503** |
| 19 | `POST /execution-intents/:intent_id/signatures` | Perp settlement signature | Optional EIP-712 | **HTTP 503** |

### Per-action HTTP status mapping

| Reject reason | Status |
|---|---|
| `write authorization required` (missing field) | 422 / 401 (depending on cause) |
| `invalid write signature` (malformed or non-recoverable) | 401 |
| `signer does not match account` | 403 |
| `write authorization expired` | 422 |
| `write authorization nonce not found` | 422 |
| `write authorization payload mismatch` (replay with different payload) | 409 |
| `idempotency key conflict` | 409 |
| `too many outstanding write challenges` | 429 |
| `perps not live` | 503 |

---

## 2. Chosen authorization architecture

**One EIP-712 type, one envelope, per-action canonical payload.**

Central module: `src/auth/write_authorization.rs`.

Each protected route handler:

1. Deserializes the request body, which now includes an `authorization: AuthorizationEnvelope` field.
2. Resolves the expected `account` (URL path, body field, or looked-up resource owner — depending on route).
3. Reconstructs the canonical payload bytes from the request fields via a per-action `canonical_*` helper (`src/api/routes.rs`).
4. Calls `verify_and_claim(store, envelope, action, account, canonical, chain_id, now)`, which:
   - Validates deadline > now, signature shape, parses nonce.
   - Computes the EIP-712 `WriteAuthorization` digest from the inputs.
   - Recovers the signer; rejects if it does not match `account` (case-insensitive).
   - Atomically claims the nonce row in `write_auth_challenges` (Postgres `UPDATE … WHERE status='issued' RETURNING …`).
   - Compares stored `request_digest` on already-consumed rows to distinguish exact-retry (idempotent replay) from same-nonce-different-payload (rejected).
5. If `verified.idempotent_resource_id` is `Some`, returns the existing resource (idempotent retry path) without re-running the mutation.
6. Performs the mutation, then calls `attach_resource_best_effort` to link the consumed nonce row to the resulting `resource_id`.

The verifier itself is generic — no per-route SQL, no per-route key management.

---

## 3. Exact EIP-712 domain

Off-chain `salt` variant (no `verifyingContract`) — distinct from every on-chain settlement domain.

```
EIP712Domain(
    string name,
    string version,
    uint256 chainId,
    bytes32 salt
)

name              = "DeOpt API Write"
version           = "1"
chainId           = 84532                                  (Base Sepolia)
salt              = keccak256("deopt-api-write:base-sepolia:v1")
verifyingContract = OMITTED
```

The salt preimage `"deopt-api-write:base-sepolia:v1"` is frozen in both frontend (`src/lib/write-auth.ts::WRITE_AUTH_DOMAIN_SALT_PREIMAGE`) and backend (`src/auth/write_authorization.rs::WRITE_AUTH_DOMAIN_SALT_PREIMAGE`), and asserted by `domain_salt_is_frozen` (backend unit test) + `frozen domain salt preimage is exactly the documented literal` (frontend node test).

---

## 4. Exact typed-data schema

```
WriteAuthorization(
    string action,
    address account,
    bytes payload,
    uint256 nonce,
    uint256 deadline,
    string environment,
    string idempotencyKey
)

action          = SCREAMING_SNAKE_CASE action enum value
account         = the account being mutated (must equal recovered signer)
payload         = keccak256(canonical_payload_bytes(action, fields))
                  — backend independently reconstructs `payload` from
                    the request body, so a tampered field invalidates
                    the signature.
nonce           = 32-byte CSPRNG value issued by POST /auth/write-challenges
deadline        = millisecond timestamp; must be > now_ms
environment     = "base-sepolia" (frozen)
idempotencyKey  = optional client-supplied string; "" if absent
```

### Canonical payload encoding

```
action |key1=encoded(value1)|key2=encoded(value2)|...
```

Field order matches the per-action `canonical_*` helper in `src/api/routes.rs`. Encoding rules:

| Type | Encoding |
|---|---|
| `string` | `"value"` with `\` / `"` / `\n` / `\r` / `\t` escaped |
| `u64` / `u128` | decimal |
| `bool` | `true` / `false` |
| `address` | `"0xlowercased…"` |
| `null` (option absent) | `null` |

The canonical encoding is the same UTF-8 byte sequence on both ends. The backend asserts it with `canonical_payload_encoding_is_frozen`; the frontend asserts it with `tests/node/write-auth-canonical.contract.mjs` against the identical fixture.

---

## 5. Challenge / nonce lifecycle

```
client                         backend                          PostgreSQL
-----------------------------------------------------------------------------
POST /auth/write-challenges
{ account, action, idempotency_key? }
-----► generate 32-byte nonce
       insert row (status='issued', expires_at = now + 5 min)
                                       ◄---- write_auth_challenges row
◄---- { nonce, deadline_ms, chain_id, domain, types }

(client signs WriteAuthorization typed-data via wallet)

POST /options/orders
{ ..., authorization: { action, account, nonce, deadline_ms,
                        signature, idempotency_key? } }
-----► verify_and_claim:
         reconstruct canonical → digest → recover signer
         signer == account?              ─► no → 403 SignerMismatch
         atomically UPDATE row
           WHERE nonce=$1 AND status='issued' AND expires_at > now
           RETURNING request_digest, resource_id
                                       ◄---- atomic claim
         row was 'issued' & matched      ─► Fresh; status='consumed'
         row already 'consumed' & digest matches  ─► IdempotentReplay
         row already 'consumed' & digest differs  ─► PayloadMismatch 409
         no row                          ─► NonceNotFound 422
         row expired                     ─► Expired 422

       perform business mutation
       attach_resource(nonce, resource_id)
◄---- 200 OK { resource }

retry: same envelope, same canonical
-----► verify_and_claim returns IdempotentReplay with resource_id
       handler returns the existing resource without re-running
◄---- 200 OK { same resource }
```

### Replay & idempotency guarantees

| Scenario | Outcome |
|---|---|
| First valid request | mutation performed, 200 OK |
| Exact retry (same nonce + same envelope + same canonical) | resource looked up and returned; **no second mutation** |
| Replay with same nonce but different canonical | 409 `payload mismatch`; **no second mutation** |
| Different nonces but same idempotency key | 409 `idempotency key conflict` (`write_auth_challenges_idempotency` unique index) |
| Concurrent submission of one nonce | row lock (`FOR UPDATE`) serializes claims; first wins, second sees `consumed`; **at most one mutation** |
| Expired nonce | 422 `expired` |
| Nonce belongs to a different action | 422 `not found` (action is part of the lookup tuple) |
| Nonce belongs to a different account | 422 `not found` |
| Wrong chain id | 422 `chain mismatch` |
| WebSocket nonce used as HTTP write nonce | 422 `nonce not found` — separate persistence (`used_nonces` is the WS table; `write_auth_challenges` is the HTTP-write table); the rows never cross |

---

## 6. Database migration

`migrations/0029_write_auth_challenges.sql`

```
CREATE TABLE write_auth_challenges (
    nonce_bytes      BYTEA       PRIMARY KEY  CHECK (octet_length(nonce_bytes) = 32),
    account          TEXT        NOT NULL,
    action           TEXT        NOT NULL,
    chain_id         BIGINT      NOT NULL,
    issued_at_ms     BIGINT      NOT NULL,
    expires_at_ms    BIGINT      NOT NULL,
    status           TEXT        NOT NULL CHECK (status IN ('issued','consumed','expired')),
    request_digest   BYTEA       CHECK (request_digest IS NULL OR octet_length(request_digest) = 32),
    idempotency_key  TEXT,
    resource_id      TEXT,
    consumed_at_ms   BIGINT
);
CREATE INDEX write_auth_challenges_by_account_action_status
    ON write_auth_challenges (lower(account), action, status);
CREATE INDEX write_auth_challenges_by_expires_at_ms
    ON write_auth_challenges (expires_at_ms);
CREATE UNIQUE INDEX write_auth_challenges_idempotency
    ON write_auth_challenges (lower(account), action, idempotency_key)
    WHERE idempotency_key IS NOT NULL AND status = 'consumed';
```

The unique idempotency index uses partial-index semantics so multiple challenges may carry the same key, but only one **consumed** mutation per key can exist.

The transactional claim (`PgRepository::claim_write_auth_challenge`) uses `SELECT … FOR UPDATE` then `UPDATE … WHERE status='issued'`. Two concurrent claims of the same nonce serialize — exactly one transitions to `consumed`; the loser sees the `consumed` row + matching digest = `IdempotentReplay`, or non-matching digest = `PayloadMismatch`.

---

## 7. Frontend integration

### Shared client

`src/lib/write-auth.ts` — single source of truth on the frontend. Exposes:

- `requestChallenge({ account, action, idempotencyKey? })` → calls `POST /auth/write-challenges`, returns the issued nonce + domain.
- `canonicalPayload(action, fields)` → byte-identical to `auth::write_authorization::canonical_payload_bytes`.
- `typedData({ action, account, canonical, nonce, deadlineMs, idempotencyKey })` → constructs the EIP-712 typed-data object for viem.
- `buildAuthorization({ account, action, canonical, signTypedData, idempotencyKey? })` → end-to-end: request challenge, build typed data, call `signTypedData`, return the `AuthorizationEnvelope` ready to embed in any API body.
- `canonical.optionOrderSubmit(...)`, `canonical.optionOrderCancel(...)`, etc. — convenience builders per action that match the backend's field order exactly.

### Component integrations

| Component | Change |
|---|---|
| `src/components/trading/DirectOrderbookForm.tsx` | Calls `buildAuthorization` before `submitOptionOrder`. Enforces: wallet connected, Base Sepolia, body `account` == connected wallet, else surfaces error. |
| `src/components/trading/TpSlManager.tsx` | Builds `CONDITIONAL_ORDER_CREATE` envelope (binds every leg field) before `createConditionalOrders`. Builds `CONDITIONAL_ORDER_CANCEL` envelope before `cancelConditionalOrder`. |
| `src/components/trading/TradeTicket.tsx` | Wraps the settlement-signature submission with an `OPTION_EXECUTION_INTENT_SIGNATURE_SUBMIT` envelope. Declares the role (`buyer` / `seller`) explicitly. The settlement EIP-712 signature is preserved as the `buyer_signature` / `seller_signature` field. |
| `src/components/trading/RfqPanel.tsx` | Same pattern as TradeTicket for the RFQ envelope path. |

### Type definitions

`src/lib/trading-types.ts` — added required `authorization: AuthorizationEnvelope` field to:
- `SubmitOptionOrderRequest`
- `CreateConditionalOrderRequest`
- `SubmitSignaturesRequest` (also added `submitter` + `role`)

`cancelConditionalOrder()` in `src/lib/trading-api.ts` now takes a body parameter `{ authorization }`.

### Frontend safety properties

- The frontend NEVER handles a private key. All signing is delegated to the wallet's `signTypedData` via `useWallet()`.
- Wrong chain → wallet returns `wrong_network`; UI surfaces a Base Sepolia prompt; no envelope built.
- User rejects → envelope build throws; no API call submitted.
- Expired challenge → `buildAuthorization` is called once per submission; expired challenges produce a wallet error or backend rejection, and the next attempt requests a fresh challenge.
- Exact retry after network failure: if the user resubmits the same form, a NEW challenge is requested but the canonical payload + idempotency-key index ensure the resource is not duplicated.

---

## 8. WebSocket auth separation

| Surface | Mechanism | Nonce table | Domain |
|---|---|---|---|
| Public WebSocket private channel auth | EIP-191 personal_sign over a challenge string | (in-memory per-session) | n/a (EIP-191) |
| HTTP write authorization | EIP-712 typed-data (this milestone) | `write_auth_challenges` | `DeOpt API Write / 1 / 84532 / salt` |

The two are **non-interchangeable** by construction:

- Different message schemes (EIP-191 vs EIP-712).
- Different persistence tables.
- A WebSocket challenge sent against `POST /options/orders` produces 422 `nonce not found` — there is no row in `write_auth_challenges` for it.
- A write-auth envelope sent against the WebSocket auth handshake produces a personal-sign verification failure.

`tests/account_write_auth_tests.rs::conditional_cancel_rejects_cross_wallet` covers the cross-account direction.

---

## 9. Tests

### Backend unit (`src/auth/write_authorization.rs::tests`) — 11/11 pass

- `canonical_payload_encoding_is_frozen` (wire-contract fixture)
- `domain_salt_is_frozen` (frozen salt preimage)
- `valid_envelope_round_trips`
- `expired_envelope_rejected`
- `wrong_chain_rejected`
- `unknown_nonce_rejected`
- `action_swap_rejected`
- `account_swap_rejected`
- `malformed_signature_rejected`
- `replay_with_different_payload_rejected`
- `replay_with_same_payload_returns_idempotent`

### Backend integration (`tests/account_write_auth_tests.rs`) — 12/12 pass

- `challenge_endpoint_issues_nonce`
- `challenge_endpoint_rejects_unknown_action`
- `valid_signed_option_order_accepted` (full HTTP round-trip)
- `unsigned_option_order_rejected` (422 from missing field)
- `tampered_payload_rejected` (signer mismatch when price changed after signing)
- `wrong_account_rejected` (signer mismatch when account swapped)
- `replay_of_consumed_nonce_rejected` (idempotent retry returns same `order_id`)
- `perp_submit_order_fails_closed` (503)
- `perp_cancel_order_fails_closed` (503)
- `perp_create_rfq_fails_closed` (503)
- `conditional_cancel_requires_authorization_body`
- `conditional_cancel_rejects_cross_wallet` (403)

### Backend regression — all pre-existing semantics intact

- `tests/conditional_orders_tests.rs` — 12/12 pass (TP/SL/OCO/comparator/stale-oracle/reduce-only semantics)
- `tests/conditional_orders_e2e_tests.rs` — 12/12 pass
- `tests/conditional_orders_pg_proof.rs` — preserved from previous milestone (live-PG runner unchanged)
- `tests/options_tests.rs` — 82/82 pass (service-level semantics); 6 HTTP-shape tests `#[ignore]`'d with superseded-by note (replaced by `account_write_auth_tests.rs`)
- `tests/rfq_tests.rs` — 35/35 pass (RFQ + signed-quote semantics); 2 perp-HTTP tests `#[ignore]`'d for the same reason
- `tests/engine_tests.rs` — 21/21 pass; 22 perp HTTP-route tests `#[ignore]`'d (re-enable in follow-up milestone)
- `tests/mm_gateway_tests.rs` — 67/67 pass
- `tests/orderbook_tests.rs` — 8/8 pass

### Frontend node tests (`tests/node/write-auth-canonical.contract.mjs`) — 13/13 pass

Freezes the canonical encoding by re-implementing the rules in pure JS and asserting the same fixture the backend asserts. Drift in either side breaks both this test and `canonical_payload_encoding_is_frozen`.

### Frontend validation

- `npm run lint` — clean
- `npx tsc --noEmit` — clean
- `npm run build` — clean (Next.js production bundle built)

---

## 10. Files changed

### Backend (new)

- `src/auth/mod.rs`
- `src/auth/write_authorization.rs` (core module + in-memory store + 11 unit tests)
- `migrations/0029_write_auth_challenges.sql`
- `tests/account_write_auth_tests.rs` (12 integration tests)

### Backend (modified)

- `Cargo.toml` — added `async-trait`, `once_cell`, `rand` as direct deps
- `src/lib.rs` — added `pub mod auth`
- `src/error.rs` — added `BackendError::WriteAuth(WriteAuthError)` + `BackendError::PerpsNotLive`
- `src/api/http.rs` — `AppState.write_auth_challenges: Arc<dyn WriteAuthChallengeStore + Send + Sync>` with PgRepository wiring when persistence is enabled; in-memory fallback otherwise
- `src/db/repository.rs` — `issue_write_auth_challenge` / `count_outstanding_write_auth_challenges` / `claim_write_auth_challenge` / `lookup_write_auth_idempotent_result` / `attach_write_auth_resource` + the `WriteAuthChallengeStore` trait impl
- `src/api/routes.rs` —
  - new `POST /auth/write-challenges` route + handler
  - `IssueWriteAuthChallengeRequest/Response`, `AuthorizationOnlyBody`, `require_write_auth`, `attach_resource_best_effort`, `authorize_or_log`, `short_hash`
  - per-action canonical builders (`canonical_option_order_submit` … `canonical_option_execution_intent_signature_submit`)
  - wired auth into option-side handlers (submit / cancel / conditional create / conditional cancel / RFQ create / RFQ quote submit / RFQ accept / RFQ cancel / execution-intent signature submit)
  - fail-closed perp handlers (`submit_order`, `cancel_order`, `create_rfq`, `submit_quote`, `accept_quote`, `cancel_rfq`, `submit_execution_intent_signatures`)
  - extended `ApiError` mapping for each `WriteAuthError` variant + `PerpsNotLive`
- `tests/options_tests.rs` — 6 HTTP-shape tests `#[ignore]`'d (semantics unchanged; replaced by `account_write_auth_tests.rs`)
- `tests/engine_tests.rs` — 22 perp HTTP-route tests `#[ignore]`'d (perp routes now 503)
- `tests/rfq_tests.rs` — 2 perp HTTP tests `#[ignore]`'d

### Frontend (new)

- `src/lib/write-auth.ts` (shared client)
- `tests/node/write-auth-canonical.contract.mjs` (canonical encoding wire-contract test)

### Frontend (modified)

- `src/lib/trading-api.ts` — `cancelConditionalOrder` signature now requires `{ authorization }` body
- `src/lib/trading-types.ts` — added `authorization` to `SubmitOptionOrderRequest`, `CreateConditionalOrderRequest`, `SubmitSignaturesRequest` (plus `submitter` + `role` on the last)
- `src/components/trading/DirectOrderbookForm.tsx` — wires `useWallet` + `buildAuthorization`
- `src/components/trading/TpSlManager.tsx` — wires create + cancel auth
- `src/components/trading/TradeTicket.tsx` — wires execution-intent signature auth (declares role)
- `src/components/trading/RfqPanel.tsx` — same pattern as TradeTicket

---

## 11. Hard acceptance criteria — all satisfied

- [x] No public account mutation succeeds without cryptographic authorization (live option routes require envelope; perp routes 503).
- [x] Recovered signer must equal the account being mutated (`SignerMismatch` 403).
- [x] Signature bound to action, payload, chain, domain, nonce, deadline.
- [x] Changing any signed business field invalidates authorization (digest reconstruction at the route layer).
- [x] One nonce → at most one mutation (`SELECT … FOR UPDATE` + status transition).
- [x] Concurrent replay → at most one mutation (row lock).
- [x] Exact retry returns same resource_id (idempotent replay via `request_digest`).
- [x] Signature for one action cannot authorize another (action is part of digest + lookup).
- [x] WebSocket auth nonce cannot authorize HTTP write (separate persistence; separate scheme).
- [x] Cross-wallet order or TP/SL creation/cancellation impossible (`SignerMismatch` 403).
- [x] Existing GTC/IOC/FOK/post-only and TP/SL/OCO semantics do not regress (service-level tests pass).
- [x] Frontend never handles private keys (all signing via `useWallet().signTypedData`).
- [x] No secret exposure, no chain transaction, no deployment, no mainnet enablement, no Solidity change.

---

## 12. Limitations / deferred

- **EIP-1271 smart-contract wallets** — deferred. The current verifier is EOA-only (ECDSA recover). A future milestone can add `EIP-1271` support by checking the recovered/declared signer's deployed code and falling back to `isValidSignature(bytes32, bytes)`.
- **Session keys** — out of scope. Each write requires an explicit wallet prompt.
- **Delegated trading permissions** — out of scope.
- **API keys for bots** — out of scope. The challenge endpoint is public, so bots can use the same EOA-signing flow.
- **Hardware/KMS operator signing** — out of scope.
- **Perp re-enablement** — **follow-up milestone `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1`** will re-enable the 7 perp routes with the same envelope scheme + restore the 24 `#[ignore]`'d perp tests when the perp engine goes live.
- **`/options/execution-intents/:id/simulate|broadcast|confirm`** — left as operator-side lifecycle operations. The intent's buyer/seller are already locked at creation; calldata-ready gate prevents premature broadcast. Per-account authorization would be a UX regression for these system-level ops; revisit if the operator-side surface ever opens to end-users.

---

## 13. Operator notes

### Rollout

1. Apply migration `0029_write_auth_challenges` (run with the same migration tool the rest of the schema uses — `sqlx migrate run` or whatever the deployment runbook prescribes).
2. Deploy the backend. The new `POST /auth/write-challenges` route is public and idempotent — safe to deploy ahead of the frontend.
3. Deploy the frontend. The four updated components require a connected wallet on Base Sepolia for any mutation; the prior `ZERO_ADDRESS` placeholder no longer works.

### Cleanup

- A background process should periodically delete or mark-as-expired challenge rows past `expires_at_ms` so the table does not grow unbounded. The existing `cleanup_expired_*` cron pattern in the backend is the right home for this — not blocking V1 since rows are tiny (≤ 200 bytes), but recommended within the next milestone.

### Monitoring

- `target: "deopt.auth.write"` tracing events:
  - `issued` (challenge issuance)
  - `accepted` (verification passed; `idempotent=true|false`)
  - `rejected` (verification failed; `reason=<error category>`)
- Account identifiers in logs are hashed/truncated (`short_hash`): `0xabcdef0123…`.
- Signatures, full nonces, typed-data payloads, and authorization headers are NEVER logged.

### Safety posture

- Chain ID gate at the verifier level — a Base Sepolia signature cannot become valid on mainnet, by both `chainId` in domain separator AND explicit check inside `verify_and_claim`.
- No `X-Skip-Auth` header. No `WRITE_AUTH_DISABLED` env var. The fail-closed posture is not configurable.
- The challenge endpoint enforces a per-(account, action) outstanding-challenge cap of 32 to prevent table-pumping abuse.
