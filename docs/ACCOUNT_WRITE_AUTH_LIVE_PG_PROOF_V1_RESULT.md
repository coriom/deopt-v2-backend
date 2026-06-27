# ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-V1 — Result

**Status:** CLOSED — every HTTP write-authorization invariant is now proven against a real PostgreSQL database under parallel + sequential execution.

**Closed:** 2026-06-27
**Branch:** main
**Chain:** Base Sepolia (84532); no chain transaction occurred
**Mainnet enabled:** no
**Solidity changes:** none
**Deployment:** none
**Secrets exposed:** none (no `.env` value, database URL, RPC URL, private key, signature, password, or bearer token was printed)

---

## 1. Environment + migration proof

| Field | Value |
|---|---|
| PostgreSQL server | 16.14 (Ubuntu 16.14-0ubuntu0.24.04.1) |
| Disposable database | `deopt_auth_proof_v1` |
| Database owner | `deopt` (application role) |
| Migration applied | `0029_write_auth_challenges.sql` (via `PgRepository::run_migrations()` called once per process inside `ensure_migrated` OnceCell) |
| Allow-list gate | `deopt_auth_proof_*` (script `scripts/account-write-auth-pg-proof.sh` refuses any database whose name does not match) |
| Opt-in env var | `WRITE_AUTH_PG_TEST_ALLOW_DISPOSABLE_DB=true` (required by both script and gates) |

Each PG-proof test asserts at top-of-state:
- `state.repository.is_some()`
- `state.persistence_enabled == true`
- `state.write_auth_challenges` is the PgRepository instance — the in-memory fallback is unreachable because `AppState::with_options_config_and_repository` is the only constructor path that sets that field and it always wires the PG-backed store.

---

## 2. Live PostgreSQL proof — parallel

```
running 13 tests
test scenario_6_altered_account_rejected ... ok
test scenario_11_websocket_nonce_cannot_authorize_http_write ... ok
test scenario_7_altered_chain_rejected ... ok
test scenario_5_altered_action_rejected ... ok
test scenario_13_ambiguous_prior_claim_rejects_retry ... ok
test scenario_4_altered_payload_rejected ... ok
test scenario_10_expired_stays_rejected ... ok
test scenario_12_perp_routes_remain_fail_closed ... ok
test scenario_2_concurrent_one_nonce_one_mutation ... ok
test scenario_3_exact_retry_returns_same_resource_id ... ok
test scenario_1_valid_signed_write_creates_exactly_one_mutation ... ok
test scenario_8_idempotency_key_conflict ... ok
test scenario_9_consumed_nonce_survives_reload ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s
```

## 3. Live PostgreSQL proof — sequential (`--test-threads=1`)

```
running 13 tests
test scenario_10_expired_stays_rejected ... ok
test scenario_11_websocket_nonce_cannot_authorize_http_write ... ok
test scenario_12_perp_routes_remain_fail_closed ... ok
test scenario_13_ambiguous_prior_claim_rejects_retry ... ok
test scenario_1_valid_signed_write_creates_exactly_one_mutation ... ok
test scenario_2_concurrent_one_nonce_one_mutation ... ok
test scenario_3_exact_retry_returns_same_resource_id ... ok
test scenario_4_altered_payload_rejected ... ok
test scenario_5_altered_action_rejected ... ok
test scenario_6_altered_account_rejected ... ok
test scenario_7_altered_chain_rejected ... ok
test scenario_8_idempotency_key_conflict ... ok
test scenario_9_consumed_nonce_survives_reload ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.87s
```

Database was dropped and recreated empty (`dropdb --if-exists deopt_auth_proof_v1 && createdb --owner=deopt deopt_auth_proof_v1`) BEFORE each of the two runs, so both runs proved against a truly cold state.

---

## 4. Per-scenario proof matrix

| # | Scenario | Brief requirement | Proven via |
|---|---|---|---|
| 1 | `scenario_1_valid_signed_write_creates_exactly_one_mutation` | Valid signed write consumes nonce + creates exactly one mutation. | HTTP submit → 200, then independent probe pool counts `option_orders WHERE client_order_id='s1-clid' AND lower(account)=lower($1) == 1` AND `write_auth_challenges WHERE nonce_bytes=$1 AND status='consumed' AND resource_id IS NOT NULL == 1`. |
| 2 | `scenario_2_concurrent_one_nonce_one_mutation` | Two concurrent submissions sharing one nonce → at most one mutation. | Two `tokio::spawn` tasks submit the same envelope concurrently; assert at least one returns 200 AND independent probe confirms `COUNT(*) FROM option_orders WHERE client_order_id='s2-clid' == 1`. |
| 3 | `scenario_3_exact_retry_returns_same_resource_id` | Exact retry returns the same `resource_id`. | Same envelope submitted twice; both 200; both responses' `order_id` field matches; independent probe confirms `COUNT(*) == 1`. |
| 4 | `scenario_4_altered_payload_rejected` | Same nonce + altered payload rejected. | Sign over price=1000000000, submit body with price=9999999999 → 403 SignerMismatch (digest reconstructed server-side from the body's price). |
| 5 | `scenario_5_altered_action_rejected` | Same nonce + altered action rejected. | Issue nonce for `OPTION_ORDER_CANCEL`, sign envelope claiming `OPTION_ORDER_SUBMIT`, call verifier expecting `OPTION_ORDER_SUBMIT` → `WriteAuthError::NonceNotFound` (action is part of the lookup tuple). |
| 6 | `scenario_6_altered_account_rejected` | Same nonce + altered account rejected. | Sign with account_A, call verifier expecting account_B → `WriteAuthError::SignerMismatch`. |
| 7 | `scenario_7_altered_chain_rejected` | Same nonce + altered chain rejected. | Verifier called with `chain_id=1` instead of `84532` → `WriteAuthError::ChainMismatch` (chain id checked before any DB I/O). |
| 8 | `scenario_8_idempotency_key_conflict` | Same idempotency key + altered payload rejected. | Two distinct nonces share `idempotency_key="s8-key"`. First submit → 200, partial unique index on `(lower(account), action, idempotency_key) WHERE status='consumed'` claims the slot. Second submit with different payload → PG raises 23505; repository maps to `ClaimOutcome::IdempotencyKeyConflict`; verifier surfaces `WriteAuthError::IdempotencyConflict` → **HTTP 409 Conflict**. Independent probe confirms `COUNT(*) FROM option_orders WHERE client_order_id IN ('s8a-clid','s8b-clid') == 1` AND `nonce_b.status == 'issued'` (losing nonce row stays redeemable with a different key). |
| 9 | `scenario_9_consumed_nonce_survives_reload` | Consumed nonce survives repository reload. | Consume nonce + attach resource_id="s9-resource". Drop the whole `AppState` (simulating a backend restart). Open a brand-new `PgRepository` against the same DB. Replay the SAME envelope → returns `IdempotentReplay` with the persisted `resource_id`. `was_fresh == false`. |
| 10 | `scenario_10_expired_stays_rejected` | Expired nonce stays rejected after reload. | Issue a nonce with `expires_at_ms` in the past. First verify → `WriteAuthError::Expired`. Drop state; open new `PgRepository`; verify again → still `WriteAuthError::Expired`. The reset to `status='expired'` survives reload. |
| 11 | `scenario_11_websocket_nonce_cannot_authorize_http_write` | WS-only nonce cannot authorize HTTP write. | Sign an envelope with a nonce never inserted into `write_auth_challenges` (modelling a WebSocket EIP-191 nonce). PG store + in-memory store BOTH return `WriteAuthError::NonceNotFound`. The two persistence surfaces are disjoint by design. |
| 12 | `scenario_12_perp_routes_remain_fail_closed` | Perp mutation routes fail closed BEFORE parse/persistence. | All 7 perp routes (`POST /orders`, `DELETE /orders/:id`, `POST /rfqs`, `POST /rfqs/:id/quotes`, `POST /rfqs/:id/accept/:q`, `POST /rfqs/:id/cancel`, `POST /execution-intents/:id/signatures`) return HTTP **503 SERVICE_UNAVAILABLE**. Independent probe confirms `COUNT(*) FROM orders == 0` AND `COUNT(*) FROM rfqs == 0` after all 7 calls. |
| 13 | `scenario_13_ambiguous_prior_claim_rejects_retry` | (Production-fix proof) A nonce consumed without `resource_id` linkage must reject on retry to prevent duplicate mutation. | Manually consume nonce via the verifier WITHOUT calling `attach_resource` (simulates a process crash between the mutation and the resource link). Retry the SAME envelope → verifier returns `WriteAuthError::AmbiguousPriorClaim` (HTTP 409). The caller is forced to request a fresh challenge. |

---

## 5. Production bugs found + fixed

### Bug 1 — `AmbiguousPriorClaim` gap (identified by Phase 1 audit)

**Symptom:** `verify_and_claim` returned `idempotent_resource_id = None` for both `ClaimOutcome::Fresh` (legit first claim) AND `ClaimOutcome::IdempotentReplay { resource_id: None }` (nonce was consumed previously but no resource was ever linked — happens if the prior attempt crashed between mutation and `attach_resource`, or `attach_resource_best_effort` silently failed). The route handler treated both as "first time" and re-executed the mutation, potentially creating a duplicate when no application-level dedup key exists.

**Fix:** Added `was_fresh: bool` field to `WriteAuthVerified` + new `WriteAuthError::AmbiguousPriorClaim` (HTTP 409). After the idempotency-key fallback, if `!was_fresh && idempotent_resource_id.is_none()`, the verifier rejects so the caller requests a new challenge.

**Files:** `src/auth/write_authorization.rs` (variant + flag + reject logic), `src/api/routes.rs` (HTTP status mapping).

**Proven by:** scenario 13.

### Bug 2 — Idempotency-conflict surfaced as HTTP 500 (identified by the first live PG run)

**Symptom:** When a second nonce attempted to consume the same `(lower(account), action, idempotency_key)` already owned by a previously consumed row, the `UPDATE` in `claim_write_auth_challenge` violated the `write_auth_challenges_idempotency` partial unique index → raw SQLSTATE 23505 → mapped blanket-style to `WriteAuthError::Persistence` → HTTP **500 Internal Server Error**. The brief requires a clean **409 Conflict** rejection.

**Fix:** `PgRepository::claim_write_auth_challenge` now inspects the SQL error: 23505 maps to a new `ClaimOutcome::IdempotencyKeyConflict`, the transaction is rolled back, and the verifier surfaces `WriteAuthError::IdempotencyConflict` (already mapped to HTTP 409). The losing nonce row stays in `status='issued'` so the caller can request a new challenge with a different idempotency key. The in-memory store was updated with a symmetric pre-check so dev/test behaviour matches production.

**Files:** `src/db/repository.rs` (error-class inspection + rollback), `src/auth/write_authorization.rs` (new variant + in-memory symmetric check + verifier mapping).

**Proven by:** scenario 8 (asserts HTTP 409, `option_orders` count = 1, losing nonce row status = `issued`).

---

## 6. Harness bugs found + fixed

| # | Symptom | Fix | File |
|---|---|---|---|
| H1 | Initial parallel run hit `PoolTimedOut` because `pg_state` opened a redundant "verifier" PgRepository (5 conns) per test for an in-memory-detection check that was already implied by `state.repository.is_some()`. | Removed the verifier marker-insert; documented the implicit invariant. | `tests/account_write_auth_pg_proof.rs::pg_state` |
| H2 | `scenario_2` spawned two `tokio::spawn` tasks each calling `pg_state` (10 extra conns) to mirror "cross-process posture". With 13 parallel tests × multiple pools, this saturated PG `max_connections=100`. | Share `state` via `Arc::clone` across the two tasks; the row-lock atomicity (proven by single mutation cardinality) lives in PostgreSQL itself, not in the connection topology. | `tests/account_write_auth_pg_proof.rs::scenario_2` |
| H3 | Static `OnceCell<PgPool>` for the probe pool broke after the first test because each `#[tokio::test]` shuts down its own Tokio runtime; the second test panicked with `"A Tokio 1.x context was found, but it is being shutdown"`. | Open a small (`max_connections=2`) per-test probe pool that lives on the test's own runtime. | `tests/account_write_auth_pg_proof.rs::init_probe_pool` |
| H4 | Script's libtest passthrough `cargo test … -- --nocapture "${EXTRA_ARGS[@]}"` collapsed to `0 tests executed` when invoked as `./script -- --test-threads=1`. The user's `--` was forwarded as a positional filter, filtering all tests OUT. | Strip leading `--` from `$@` defensively; both `./script --test-threads=1` and `./script -- --test-threads=1` forms now work. | `scripts/account-write-auth-pg-proof.sh` |
| H5 | Unused import `k256::elliptic_curve::sec1::ToEncodedPoint` triggered warnings in two test files (`VerifyingKey::to_encoded_point` is an inherent method, the trait import is unnecessary). | Removed from both `tests/account_write_auth_tests.rs` and `tests/account_write_auth_pg_proof.rs`. | both test files |

---

## 7. Concurrent-nonce cardinality (scenario 2 detail)

Two `tokio::spawn` tasks submit the SAME envelope concurrently through cloned routers sharing a single PgRepository pool:

```
status_a, status_b ∈ {200 OK, 409 AmbiguousPriorClaim} — at least one must be 200.
COUNT(*) FROM option_orders WHERE client_order_id='s2-clid' = 1   (asserted)
```

PG's `SELECT ... FOR UPDATE` inside `claim_write_auth_challenge` serialises the two claims. The first wins as `Fresh` and performs the mutation. The second sees `status='consumed'`:
- If `request_digest` matches the first → `IdempotentReplay`; if `resource_id` is already attached → returns existing 200; if not yet attached (because mutation A still running) → `AmbiguousPriorClaim` 409.
- Either outcome preserves the cardinality invariant.

---

## 8. Exact-retry behaviour (scenario 3 detail)

```
First  POST /options/orders { ..., authorization: env, client_order_id: "s3-clid" }
       → 200 OK { order_id: X, ... }
Second POST /options/orders { ..., authorization: env, client_order_id: "s3-clid" }   (byte-identical body)
       → 200 OK { order_id: X, ... }                                                  (same resource_id)
COUNT(*) FROM option_orders WHERE client_order_id='s3-clid' = 1                       (asserted)
```

The verifier's `IdempotentReplay { resource_id: Some(X) }` path is exercised; no second mutation runs.

---

## 9. Restart + expiry behaviour (scenarios 9, 10 detail)

| Operation | Outcome | Reload | Outcome after reload |
|---|---|---|---|
| Consume nonce + attach `resource_id="s9-resource"` | row `status='consumed'`, `resource_id='s9-resource'` | drop AppState, open new `PgRepository` | replay envelope → `IdempotentReplay { resource_id: Some("s9-resource") }` |
| Issue nonce with `expires_at_ms` in the past | first verify → `WriteAuthError::Expired`, row transitioned to `status='expired'` | drop AppState, open new `PgRepository` | second verify → still `WriteAuthError::Expired` |

Both invariants survive an explicit process tear-down + cold-start cycle.

---

## 10. Route-bypass audit (Phase 6)

| Route | Auth | Bypass risk | Action |
|---|---|---|---|
| `POST /options/execution-intents/:id/simulate` | none (public) | Gated by `options_config.execution_enabled`. Calls external RPC, does not create new user-account-bound rows; intent buyer/seller were locked at creation when the user's `OPTION_EXECUTION_INTENT_SIGNATURE_SUBMIT` envelope was verified. | Documented; no change in V1 — re-evaluate in `ORDER-LIFECYCLE-OBSERVABILITY-V1`. |
| `POST /options/execution-intents/:id/broadcast` | none (public) | Gated by `options_config.execution_broadcast_enabled && execution_enabled && execution_config.real_broadcast_enabled` — disabled by default in dev. | Documented; no change. |
| `POST /options/execution-intents/:id/confirm` | none (public) | Gated by `execution_enabled`. Idempotent state-transition advance. | Documented; no change. |
| `POST /executor/tick` | none (public) | Gated by `execution_config.execution_enabled && dry_run`. Real on-chain execution path explicitly errors with `"real on-chain execution is not implemented yet; set EXECUTOR_DRY_RUN=true"`. | Documented; no change. |
| `POST /executor/confirmations/tick` | none (public) | Same pattern as `/executor/tick`. | Documented; no change. |
| `POST /reconciliation/tick` | none (public) | Same pattern. | Documented; no change. |
| `POST /admin/*` | admin Bearer (V2G-W2 middleware) | None. | No change. |

None of these surfaces creates new user-account-bound state. They advance already-authorized intent/transaction lifecycle. The 12 user-write surfaces are fully covered by the new EIP-712 envelope; the 7 perp surfaces are fail-closed (scenario 12).

---

## 11. Ignored-test audit (Phase 7)

| Test file | Count | Category | Action |
|---|---|---|---|
| `tests/engine_tests.rs` | 22 | All perp HTTP route tests asserting pre-V1 behaviour against `POST /orders`, `DELETE /orders/:id`, `POST /execution-intents/:id/signatures`. Routes now return HTTP 503 → assertions like "rejects malformed signature" no longer apply. | Kept `#[ignore]` with the message `"Perps non-live after ACCOUNT-WRITE-AUTH-HARDENING-V1; route fails closed with 503. Re-enable in ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1."` Replacement coverage: scenario 12. |
| `tests/rfq_tests.rs` | 2 | `create_rfq_rejects_invalid_side` + `http_quote_endpoint_stores_signature_metadata` — both target perp RFQ HTTP routes. | Kept `#[ignore]`; replacement coverage in scenario 12 + service-level RFQ tests (still 35/35 passing). |
| `tests/options_tests.rs` | 6 | `http_option_order_lifecycle`, `http_option_match_returns_fills_and_fill_endpoints`, `disabled_mode_accepts_unsigned_http_option_rfq_quote`, `option_execution_signature_submission_builds_calldata_without_transaction`, `option_rfq_http_lifecycle`, `strict_http_option_rfq_quote_endpoint_stores_signature_metadata` — all assert pre-V1 unauthenticated HTTP shapes that the new envelope made obsolete. | Kept `#[ignore]` with `"Superseded by tests/account_write_auth_tests.rs after ACCOUNT-WRITE-AUTH-HARDENING-V1"`. Equivalent or stronger coverage: `tests/account_write_auth_tests.rs` (12 tests) + `tests/account_write_auth_pg_proof.rs` (13 tests). |

**No security-sensitive test was ignored to make setup easier.** Every `#[ignore]` either targets a fail-closed perp route (covered by scenario 12) or a pre-V1 HTTP shape that the new envelope deliberately broke (replaced by the auth + PG proof suites).

---

## 12. Files changed (cumulative under `ACCOUNT-WRITE-AUTH-LIVE-PG-PROOF-FINAL-CLOSURE-V1`)

### Backend (modified)

| File | Change |
|---|---|
| `src/auth/write_authorization.rs` | Added `WriteAuthError::AmbiguousPriorClaim`; added `was_fresh` to `WriteAuthVerified`; verifier rejects ambiguous prior claims; added `ClaimOutcome::IdempotencyKeyConflict` + handler in `verify_and_claim`; `InMemoryChallengeStore::claim` symmetric idempotency-key uniqueness check. |
| `src/db/repository.rs` | `claim_write_auth_challenge` now inspects SQL error class — SQLSTATE 23505 on the idempotency partial unique index maps to `ClaimOutcome::IdempotencyKeyConflict` (rolling back the transaction so the losing nonce row stays `issued`); all other SQL errors still map to `WriteAuthError::Persistence`. |
| `src/api/routes.rs` | Added HTTP status mapping `WriteAuthError::AmbiguousPriorClaim → 409 Conflict`. |
| `tests/account_write_auth_tests.rs` | Removed unused `k256::elliptic_curve::sec1::ToEncodedPoint as _` import. |

### Backend (new)

| File | Purpose |
|---|---|
| `tests/account_write_auth_pg_proof.rs` | 13-scenario live-PG proof harness (the file documented in §4). |

### Operator tooling (new)

| File | Purpose |
|---|---|
| `scripts/account-write-auth-pg-proof.sh` | PRECREATED_DB + SELF_MANAGED_DB modes; disposable-name allow-list (`deopt_auth_proof_*`, `deopt_test_*`); opt-in env var; mainnet `CHAIN_ID` block; URL never echoed; libtest passthrough strips leading `--` defensively. |

No frontend file was changed under this milestone.

---

## 13. Validation commands actually executed

```text
# Preflight
cargo fmt --check                                                 # clean
cargo check --lib                                                 # clean
cargo check --bin deopt-v2-backend                                # clean
cargo test --lib auth::                                           # 11/11 pass
cargo test --test account_write_auth_tests                        # 12/12 pass

# Disposable DB lifecycle (twice — once for parallel, once for sequential)
sudo -n -u postgres psql -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='deopt_auth_proof_v1' AND pid <> pg_backend_pid();"
sudo -n -u postgres dropdb --if-exists deopt_auth_proof_v1
sudo -n -u postgres createdb --owner="deopt" deopt_auth_proof_v1
sudo -n -u postgres psql -Atc "SELECT datname FROM pg_database WHERE datname='deopt_auth_proof_v1';"     # → deopt_auth_proof_v1

# Live PG proof
./scripts/account-write-auth-pg-proof.sh                          # 13/13 pass, 0 ignored (parallel)
./scripts/account-write-auth-pg-proof.sh --test-threads=1         # 13/13 pass, 0 ignored (sequential)

# Regression sweep
cargo test --test conditional_orders_tests                        # 12/12 pass
cargo test --test conditional_orders_e2e_tests                    # 12/12 pass
cargo test --test options_tests                                   # 82/82 pass, 6 ignored (documented)
cargo test --tests                                                # all green; engine_tests 21 passed + 22 ignored (perp non-live), rfq 35+2, options 82+6
cargo test --lib                                                  # 1024/1024 pass
git diff --check                                                  # clean
```

Frontend checks were NOT re-run under this milestone because no frontend file was modified after the prior `ACCOUNT-WRITE-AUTH-HARDENING-V1` validation (which left lint/tsc/build/test:node all green).

---

## 14. Safety posture

| Property | Result |
|---|---|
| Secret exposure (`.env`, DB URL, RPC URL, private key, signature, password, bearer token) | None. The `WRITE_AUTH_PG_TEST_DATABASE_URL` was built from `.env` via a stdin-fed python recipe that does not echo it; the script also never echoes the URL on success or failure paths. |
| Chain transaction | None. |
| Deployment | None. |
| Mainnet enablement | None. Script refuses if `CHAIN_ID ∈ {1, 8453}`. |
| Solidity change | None. |
| Mainnet-safe role privileges | The application role does NOT hold `CREATEDB`; the operator's interactive `sudo -u postgres` was used only for the disposable DB lifecycle. |
| Authorization weakening | None. The fix STRENGTHENS replay/idempotency protection (adds `AmbiguousPriorClaim` rejection + clean 409 for idempotency conflict). |

---

## 15. Limitations / deferred

- EIP-1271 smart-contract wallets — deferred (still EOA-only).
- Session keys, delegated trading, bot API keys — deferred.
- Perp re-enablement — `ACCOUNT-WRITE-AUTH-HARDENING-PERPS-V1` will re-enable the 7 perp routes + restore the 24 `#[ignore]`'d perp tests when the perp engine becomes live.
- Operational lifecycle routes (`simulate/broadcast/confirm` + `tick` family) — left as system-level operations gated by config flags; not per-user-account bound.
- Background expiry-row cleanup for `write_auth_challenges` — recommended but not blocking; rows are tiny.

---

## 16. Next recommendation

`ORDER-LIFECYCLE-OBSERVABILITY-V1` — private WebSocket channel publishing `account.conditional_orders` lifecycle events (armed / triggered / cancelled / failed). The write-authorization layer is now closed; the next gap is operator + end-user visibility into the lifecycle of the orders the authorization protects.
