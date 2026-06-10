# E2E_LOCAL_TX_STATUS_CYCLER_RESULT (M-P4c)

**Date:** 2026-06-10
**Milestone:** `E2E-LOCAL-TX-STATUS-CYCLER` (M-P4c)
**Posture:** local/test-only. **Mainnet-refused. No broadcast. No signer.
No AWS / KMS. No `.env` edit. No real wallet. No public RPC.**

## 1. Purpose

Add a strictly local/test-only execution-intent + tx-status fixture so
the frontend Playwright suite can drive UI tx-state transitions
end-to-end against a real backend route instead of relying only on
Playwright `page.route` interception. Production transaction execution
paths are untouched.

## 2. Files changed

| Path | Status | Purpose |
|---|---|---|
| `src/api/local_test_fixtures.rs` | new | guard + status machine + store + handlers + 26 unit tests |
| `src/api/mod.rs` | edited | `pub mod local_test_fixtures;` |
| `src/api/http.rs` | edited | `AppState.local_test_fixtures` + `AppState.local_test_intents` |
| `src/api/routes.rs` | edited | wire 4 routes + 14 HTTP integration tests |
| `src/admin.rs` | edited | `required_role_for` ⇒ `POST /admin/test/*` is Operator |

## 3. Guard design

### `LocalTestFixturesConfig`

Disabled by default. Single factory: `enabled_for_chain_id(chain_id)`
returns `disabled()` iff `chain_id == 8453` (Base mainnet); enabled
otherwise. The runtime gate `assert_enabled(chain_id)` repeats both
checks at every request as defence-in-depth. Both layers map to the
same outcome: a disabled or mainnet-running fixture surface returns
HTTP 404 indistinguishably from a non-existent route.

Three explicit guarantees:

1. **Compile-time field default**: `AppState::with_all_config(…)` sets
   `local_test_fixtures: LocalTestFixturesConfig::disabled()`. The
   production startup path never installs an enabled config.
2. **Factory mainnet refusal**: even a caller asking for an enabled
   config on mainnet gets `disabled()` back.
3. **Runtime mainnet refusal**: every handler calls
   `state.local_test_fixtures.assert_enabled(state.chain_id)` before
   touching the store.

### Admin-Bearer gate (defence-in-depth #4)

`/admin/test/*` is captured by the existing `admin_route_gate`
middleware in `routes.rs`. With `AuthMode::SharedToken` enabled, a
missing `x-admin-token` header returns HTTP 403 — exercised by
`http_admin_gate_blocks_test_endpoint_without_token`. The fixture
inherits the gate for free; no admin-specific configuration is added.

### No env-loader keys

This milestone deliberately adds **no** env-loader keys, **no**
`.env.example` placeholder, and **no** boot-time switch. The fixture
must be enabled via direct mutation of `AppState.local_test_fixtures`
in test code or in the local runbook's binary entry-point only.

Rationale: any startup-config knob risks accidental production
enablement. Direct field mutation in test / runbook code makes the
"who turned this on" question unambiguous in code review.

## 4. Local intent fixture

`LocalTestIntent` is a synthetic-only record:

```text
intent_id:       new uuid
request_id:      "test-<uuid>"
account:         provided or DEFAULT_TEST_ACCOUNT (anvil[0] public)
source_type:     "option_orderbook_fill" | "option_rfq_fill"
status:          LocalTestIntentStatus enum
tx_hash:         synthetic_tx_hash(intent_id)
created_at_ms:   now_ms()
updated_at_ms:   now_ms()
transitions:     [] of {from, to, at_ms}
synthetic:       true   ← discriminator
```

`synthetic_tx_hash(uuid)` returns a deterministic, clearly-marked hash
of the form `0xdeadbee5` + 12 zero bytes + the 16 uuid bytes. The
`deadbee5` prefix is unmistakable on inspection; the embedded uuid
lets test code recompute the hash without DB access. Asserted by
`synthetic_tx_hash_is_clearly_marked` and
`synthetic_tx_hash_does_not_match_a_real_tx_hash_pattern`.

The store (`LocalTestIntentStore`) is an in-process `HashMap<Uuid,
LocalTestIntent>` held behind `Arc<Mutex<…>>`. Never persisted, never
round-tripped through `PgRepository`, never visible to the production
`option_execution_transactions` or `execution_transactions` tables.

## 5. Tx-status cycler

Status vocabulary (deliberately distinct from production):

```text
Created  →  Pending  →  Confirmed   (terminal)
                    →  Failed       (terminal)
                    →  Reverted     (terminal)
                    →  Stuck        →  Pending | Failed
```

Allowed transitions encoded in `LocalTestIntentStatus::can_transition_to`
and verified by `allowed_transitions_match_spec`. Terminal states
refuse every onward transition (verified for all 18 forbidden pairs).

## 6. HTTP routes

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/admin/test/execution-intents` | Operator | create a synthetic intent |
| `GET` | `/admin/test/intent/:intent_id` | Viewer | read a synthetic intent |
| `POST` | `/admin/test/intent/:intent_id/transition` | Operator | apply a status transition |
| `GET` | `/trading/test/tx-status/:intent_id` | (none) | frontend-facing synthetic tx status |

All four routes call `assert_or_404(&state)` first. Disabled or
mainnet `state.chain_id` → 404. The `/trading/test/*` route is
deliberately unauthenticated: the route returns 404 unless the test
fixture is installed, so leaving it open in production code is
equivalent to the route not existing.

## 7. Public tx-status consumption

The frontend reads synthetic status via
`GET /trading/test/tx-status/:intent_id`. The envelope is a small,
deliberately distinct shape:

```json
{
  "source": "local_test_fixture",
  "synthetic": true,
  "intent_id": "<uuid>",
  "request_id": "test-<uuid>",
  "account": "0x…",
  "source_type": "option_orderbook_fill",
  "status": "pending",
  "tx_hash": "0xdeadbee5…",
  "created_at_ms": 0,
  "updated_at_ms": 0,
  "transitions": [{ "from": "created", "to": "pending", "at_ms": 0 }]
}
```

The `source: "local_test_fixture"` discriminator and the `synthetic:
true` flag are intentional defence-in-depth signals. A downstream
consumer can never mistake a fixture row for a real broadcast result.

The production endpoint `/executor/transactions/:intent_id` is
**unchanged**. It continues to read from `PgRepository` /
`option_execution_transactions` / `execution_transactions` only.
Asserted by `executor_transactions_unchanged_when_fixture_intent_created`.

## 8. Tests (40 added)

### Unit tests in `local_test_fixtures::tests` (26)

1. `local_test_fixtures_disabled_by_default`
2. `local_test_fixtures_disabled_constructor_is_disabled`
3. `local_test_fixtures_refuses_mainnet_chain_id`
4. `local_test_fixtures_enabled_for_sepolia`
5. `local_test_fixtures_enabled_for_anvil`
6. `assert_enabled_returns_err_when_disabled_on_sepolia`
7. `assert_enabled_returns_err_on_mainnet_even_when_flag_is_true`
8. `assert_enabled_returns_ok_when_enabled_on_sepolia`
9. `status_parse_roundtrip`
10. `status_parse_rejects_unknown`
11. `status_terminal_set_is_correct`
12. `allowed_transitions_match_spec`
13. `store_create_returns_intent_with_request_id`
14. `synthetic_tx_hash_is_clearly_marked`
15. `store_transition_full_cycle_created_to_confirmed`
16. `store_transition_pending_to_failed`
17. `store_transition_pending_to_reverted`
18. `store_transition_stuck_recovers_to_pending`
19. `store_rejects_invalid_transition_created_to_confirmed`
20. `store_rejects_transition_from_terminal_state`
21. `store_unknown_intent_returns_persistence_error`
22. `map_account_accepts_valid_hex`
23. `map_account_defaults_to_anvil_zero`
24. `map_account_rejects_garbage`
25. `map_source_type_defaults_and_validates`
26. `synthetic_tx_hash_does_not_match_a_real_tx_hash_pattern`

### HTTP integration tests in `api::routes::tests` (14)

27. `http_create_intent_returns_404_when_fixtures_disabled`
28. `http_create_intent_returns_404_on_mainnet_chain_id`
29. `http_transition_returns_404_when_fixtures_disabled`
30. `http_tx_status_read_returns_404_when_fixtures_disabled`
31. `http_create_intent_returns_200_when_enabled_sepolia`
32. `http_full_lifecycle_create_pending_confirmed_on_anvil`
33. `http_invalid_transition_returns_400`
34. `http_transition_unknown_intent_returns_404`
35. `http_transition_malformed_uuid_returns_404`
36. `http_transition_unknown_status_returns_400`
37. `executor_transactions_unchanged_when_fixture_intent_created`
38. `http_response_envelope_never_leaks_secrets`
39. `http_get_intent_returns_intent_when_known`
40. `http_admin_gate_blocks_test_endpoint_without_token`

All 40 pass; full backend lib suite **880 tests green**
(840 prior + 40 new). No production tests required modification.

## 9. Validations

* `cargo build --lib` exit 0.
* `cargo test --lib --no-fail-fast` ⇒ 880 passed, 0 failed.
* `cargo fmt --all -- --check` (see Section 11 for results).
* `cargo clippy --lib --no-deps --all-features -- -D warnings`
  (see Section 11 for results).
* `git diff --check` clean.
* Sensitive-string scan over the new source + docs: zero matches for
  `EXECUTOR_PRIVATE_KEY`, `DATABASE_URL`, `AWS_ACCESS_KEY_ID=`,
  `AWS_SECRET_ACCESS_KEY=`, `arn:aws:kms:`, production EVM-address
  patterns. The synthetic-hash prefix (`0xdeadbee5`) is the only
  hex-looking string in this change.

## 10. Blockers update

| Blocker | Prior status | After M-P4c |
|---|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | open | **CLOSED** (synthetic intent fixture available) |
| B2 ON_CHAIN_RPC_NOT_WIRED | open | unchanged → M-P2e |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | open (blocked by B1) | unblocked → next frontend milestone |
| B4 NO_TEST_FRAMEWORK | partially closed | unchanged (chromium download remains operator-side) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | open | **CLOSED** |
| B6 LOGO_NOT_IN_NAV | closed | unchanged |

## 11. Next milestone recommendation

**Recommended next:**
`FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING` (see Section 12
handoff). The wiring is small: replace the existing
`page.route("**/options/execution-intents/*/signing-payload", …)`
interception in `sign-rejected.spec.ts` with a real
`/admin/test/execution-intents` + `/admin/test/intent/:id/transition`
drive, plus a new spec that polls `/trading/test/tx-status/:id`
through Created → Pending → Confirmed.

**Then:** `BACKEND-TRADING-API-IMPLEMENTATION-PHASE-5` (M-P2e) for
the remaining 6 partial endpoints + env-loader keys.

## 12. Frontend handoff

A precise next-task prompt for the frontend lives at
`docs/FRONTEND_PLAYWRIGHT_TX_STATUS_CYCLER_WIRING_NEXT_TASK.md`.

## 13. Cross-links

* `E2E_LOCAL_AUTOMATION_RUNBOOK.md`
* `E2E_LOCAL_FIXES_RESULT.md`
* `E2E_LOCAL_TRADING_BLOCKERS_AND_FIXES.md`
* `E2E_LOCAL_TRADING_LIFECYCLE_RUNBOOK.md`
* `E2E_LOCAL_TX_STATUS_CYCLER_RUNBOOK.md` (this milestone)

**End of result doc.**
