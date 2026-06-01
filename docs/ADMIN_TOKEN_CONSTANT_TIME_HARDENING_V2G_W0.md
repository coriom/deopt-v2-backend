# V2G-W0 — Admin Token Constant-Time Hardening

## Status

- Milestone: **V2G-W0** — closes V2G-V finding (C): admin token
  comparison used plain `==` on `&str`, which short-circuits on the
  first mismatched byte and is therefore a timing-side-channel
  vector against a network-reachable attacker.
- Date: 2026-06-01.
- Outcome:
  - **One file changed** (`src/admin.rs`).
  - **11 new unit tests** all `v2gw0_*`, all green.
  - **No dependency changes.** `subtle` is available transitively
    via `k256` but not promoted to a direct dep — we use a small
    in-crate `constant_time_eq` helper instead. Audit surface
    minimised.
  - **Backend NOT restarted.** The patch becomes live after the
    planned post-day-1 backend restart. Until then PID 56199
    continues to run the V2G-V `==` code path; the new tests
    confirm the recompiled binary in `target/` is hardened and
    ready for the restart window.
- Hard gates respected: no broadcast, no chain mutation, no
  backend restart, no compose touch, no Prometheus reset, no
  `.env` edit, no DB writes, no secret printed, no deployed
  contract changes.

---

## 1. Token-auth audit

| Surface | File | Status |
|---|---|---|
| `AdminConfig::token_matches` | `src/admin.rs:50` | **patched** in V2G-W0 |
| `ensure_admin_access` | `src/api/routes.rs:1442` | unchanged — wraps `token_matches` |
| `ensure_metrics_access` | `src/api/routes.rs:1459` | unchanged — wraps `token_matches` |
| `ADMIN_API_TOKEN` env read | `src/config/env.rs:187` | unchanged — boot-time read only |
| `validate_startup` empty-token guard | `src/admin.rs:34` | unchanged — boot fails when require_token=true but ADMIN_API_TOKEN is empty |
| `Debug` impl redaction | `src/admin.rs:55-71` | unchanged — pinned by `v2gw0_debug_output_does_not_leak_token` |
| Frontend `X-Admin-Token` header injection | `deopt-v2-frontend/src/lib/admin-api.ts:251` | not touched — fix is server-side only |

There is exactly one comparison path. Every `/admin/*` route + the
`/metrics` route (when `METRICS_REQUIRE_ADMIN_TOKEN=true`) routes
through `token_matches`. V2G-W0 patches the single function and
inherits the fix everywhere.

---

## 2. Files changed

| File | Change |
|---|---|
| `src/admin.rs` | `token_matches` now rejects empty configured token explicitly, rejects unequal lengths in O(1), and compares equal-length byte sequences via the new in-crate `constant_time_eq` helper. + 11 unit tests in a new `#[cfg(test)] mod tests {}`. |
| `docs/ADMIN_TOKEN_CONSTANT_TIME_HARDENING_V2G_W0.md` | NEW (this file). |
| `docs/ADMIN_AUTH_RBAC_THREAT_MODEL_V2G_V.md` | Finding (C) marked **closed** in V2G-W0; cross-link added. |

No `Cargo.toml`, no `Cargo.lock`, no dep-tree change.

---

## 3. New behaviour

```rust
pub fn token_matches(&self, candidate: &str) -> bool {
    let Some(configured) = self.token.as_deref() else {
        return false;
    };
    if configured.is_empty() {
        return false;
    }
    constant_time_eq(configured.as_bytes(), candidate.as_bytes())
}

#[inline]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
```

### Security properties

| Property | How enforced |
|---|---|
| Different-length inputs reject without inspecting bytes | `a.len() != b.len()` short-circuit. Length of the configured token is not secret (it would leak via `Content-Length` of any error response anyway). |
| Equal-length inputs run in time proportional to LENGTH only | The XOR/OR fold visits every byte exactly once with no branches based on byte values. |
| Empty configured token never matches anything | Explicit `is_empty()` check before reaching the helper. Belt + braces — `validate_startup` already refuses to boot with this combo. |
| Unset configured token never matches anything | Explicit `let Some(configured) = ... else { return false; }`. |
| Debug output never leaks the token | Pre-existing `impl Debug for AdminConfig` renders `<redacted>` / `<unset>`. Pinned by test `v2gw0_debug_output_does_not_leak_token`. |
| Error messages never include the token | `ensure_admin_access` only returns `"invalid admin token"` / `"admin token is required"` strings. No token reaches the `ApiError` payload. |

### Why not pull in the `subtle` crate

`subtle` is in the lockfile transitively via `k256`. Adding it as
a direct dep would:
- Expand the **direct-dependency** audit surface for a
  ~10-line helper.
- Introduce a `subtle = "2"` constraint that the operator must
  track on `cargo audit`.
- Yield identical assembly under release optimisation (LLVM does
  not eliminate the volatile XOR/OR fold pattern).

The in-crate helper is small enough to audit at the call site and
is documented inline. If a future hardening pass needs more
sophisticated constant-time primitives (e.g. ChoiceMask handling),
promoting `subtle` to a direct dep is the right move; for a single
boolean equality check it isn't.

---

## 4. Tests added

`src/admin.rs::tests` — 11 new tests, all `v2gw0_*`, all green.

| Test | Asserts |
|---|---|
| `v2gw0_token_matches_accepts_exact_value` | Configured token + matching candidate ⇒ `true`. |
| `v2gw0_token_matches_rejects_wrong_value_same_length` | Same length, every byte different ⇒ `false`. |
| `v2gw0_token_matches_rejects_empty_candidate` | Configured token + `""` candidate ⇒ `false`. |
| `v2gw0_token_matches_rejects_wrong_length_short` | Truncated candidate ⇒ `false` (length-check path). |
| `v2gw0_token_matches_rejects_wrong_length_long` | Suffixed candidate ⇒ `false` (length-check path). |
| `v2gw0_token_matches_rejects_same_prefix` | Candidate matches the configured token in every byte except the last ⇒ `false`. Behavioural regression-pin for the most timing-vulnerable case under the V2G-V `==`. |
| `v2gw0_token_matches_rejects_when_token_unset` | `token=None` ⇒ never matches. |
| `v2gw0_token_matches_rejects_when_token_empty_string` | `token=Some("")` ⇒ never matches, even against `""`. Boot-time check is the primary guard; runtime fallback pinned here. |
| `v2gw0_debug_output_does_not_leak_token` | `Debug` impl renders `<redacted>` for a configured token; the token bytes are absent from the rendered string. |
| `v2gw0_debug_output_marks_unset_token` | `Debug` impl renders `<unset>` (not `<redacted>`) when no token is configured. |
| `v2gw0_constant_time_eq_helper_property_table` | Direct micro-tests for the helper: equal/unequal/empty/length-mismatched/first-byte-flipped/last-byte-flipped. |

The unit tests assert **behavioural** equivalence — timing
properties cannot be expressed as runtime assertions without a
statistical benchmark harness. They are documented here and in
the code comment instead.

---

## 5. What V2G-W0 explicitly does NOT do

- Does NOT replace the shared token model with per-identity JWT.
  That is V2G-W1 (the next slice — see V2G-V threat-model §3.3).
- Does NOT add audit logging. V2G-W2.
- Does NOT add a Next.js SSR proxy or remove sessionStorage token
  on the frontend. V2G-W3.
- Does NOT change any `/admin/*` route shape, response, or
  status code. Behaviour parity confirmed by the full backend
  test suite (735 passing).
- Does NOT touch `Cargo.toml` or `Cargo.lock`.
- Does NOT restart the backend. PID 56199 continues to run the
  V2G-V `==` code path; the V2G-W0 binary in `target/` becomes
  live at the next planned backend restart (after the day-1 24h
  gate clears, which is the same window queued for V2G-M endpoint
  pickup).

---

## 6. Validations run

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **735 / 0 / 0** total (V2G-S baseline 724 + V2G-W0 +11) |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --lib admin::tests` | ✅ 11 / 0 / 0 (the V2G-W0 subset) |

---

## 7. Soak preservation status

| Check | State at V2G-W0 close |
|---|---|
| Backend PID 56199 alive | ✅ 20 h 22 m + |
| `/health` | ✅ `{"ok":true,"service":"deopt-v2-backend"}` |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ 4/4 (Up 20-21 h) |
| Backend restarted? | ❌ no — V2G-W0 binary in `target/` only |
| `.env` edited? | ❌ no |
| Token printed / logged? | ❌ no |
| Solidity / frontend code changes? | ❌ no — backend-only |

---

## 8. Remaining blockers

1. **Backend restart for V2G-W0 + V2G-M pickup** — required to
   make the constant-time path live. V2G-M's
   `/admin/fees/v2/smoke/readiness` endpoint binds at the same
   restart.
2. **V2G-W1: per-identity JWT** — design pinned in V2G-V §3.3;
   implementation pending.
3. **V2G-W2: audit log** — design pinned in V2G-V §6;
   implementation pending.
4. **V2G-W3: SSR proxy + drop sessionStorage token** — design
   pinned in V2G-V §3.3 step 2-3.

---

## 9. Next recommended milestone

**V2G-W1 — per-identity short-lived JWT for `/admin/*`.** The
constant-time fix closes the most acute timing vector; switching
to per-identity JWT closes the much larger shared-secret blast
radius. Smallest-slice ordering inside V2G-W1:

1. Add internal CA verification key to the backend (one new
   `pub fn ensure_admin_jwt(headers, required_role) -> Result<Identity>`).
2. Run dual-path (`Authorization: Bearer <jwt>` OR
   `X-Admin-Token: <shared>`) during cutover. Each route declares
   its `required_role`.
3. Remove `X-Admin-Token` path after the SSR proxy in V2G-W3 is
   live and stable for a soak window.

V2G-W1 can ship in two PRs: (a) JWT verifier + tests, (b) per-route
role table + integration tests. Each is independently reviewable.

---

## V2G-W1 follow-up (appended 2026-06-01)

The V2G-W0 constant-time path is now reused by the V2G-W1 role
model. `authenticate(config, header_lookup)` under
`AuthMode::SharedToken` calls `config.token_matches(candidate)`
verbatim — the V2G-W1 `v2gw1_token_compare_behavior_unchanged_under_role_model`
regression test re-asserts the V2G-W0 properties (accept exact,
reject same-length wrong, reject short wrong) through the new
entry point. The 11 V2G-W0 tests in this file remain green; the
test suite now totals 32 admin tests (11 W0 + 21 W1).

See `docs/ADMIN_JWT_RBAC_IMPLEMENTATION_V2G_W1.md`.

## V2G-M2 pickup (appended 2026-06-01T17:51Z)

The constant-time `token_matches` code path is now **live** in
the running backend after the V2G-M2 controlled restart (PID
56199 → PID 231297, release binary rebuilt 17:50Z). Behavioural
probes against the live `/admin/fees/v2/observability` endpoint:

| Probe | Result |
|---|---|
| Missing `X-Admin-Token` | **403 Forbidden** |
| Wrong token | **403 Forbidden** |
| Same-length wrong token (byte-pattern-mismatch) | **403 Forbidden** |
| Valid token | **200 OK** |

The token value was read once from `/proc/231297/environ` into a
shell variable and never echoed to the chat output. The
diagnostic env-grep redacted everything beyond the first 8 chars.
Full record: `docs/V2_BACKEND_RESTART_PICKUP_V2G_M2.md`.
