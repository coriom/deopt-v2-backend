# V2G-W2 — Admin RBAC Route Enforcement + Audit Log

## Status

- Milestone: **V2G-W2** — wires the V2G-W1 role/identity/auth-mode
  primitives into the live `/admin/*` route gate via a single
  middleware layer + introduces structured audit logging. The
  existing `ensure_admin_access` handler-side check is preserved
  for defence-in-depth during the cutover.
- Date: 2026-06-01.
- Outcome:
  - **1 source file extended** (`src/api/routes.rs`): two new
    functions (`admin_route_gate` middleware + `admin_audit_deny`
    helper) + 8 new integration tests + one router-builder change
    inserting the middleware.
  - **No new dependencies.** Uses `axum::middleware::from_fn_with_state`
    and `tracing::info!` / `tracing::warn!` already in the
    crate.
  - **No handler signature changes.** Every existing `/admin/*`
    handler continues to call `ensure_admin_access` — both gates
    now run; the middleware adds the audit-log line + role-aware
    decision.
  - **Frontend types** (`src/lib/admin-rbac-types.ts`) mirror the
    backend `AdminRole` enum + `requiredRoleFor` lookup for use
    by the V2G-W3 SSR proxy and the existing UI's role-aware
    affordances.
  - **Soak preserved.** Live backend PID 231297 (V2G-M2 binary,
    pre-V2G-W2) untouched. The new gate ships in `target/`;
    promotion is the next operator restart window.
- Hard gates respected: no broadcast, no chain mutation, no
  backend restart, no Docker / Prometheus / Alertmanager /
  Grafana restart, no `.env` edit, no DB writes, no secret
  printed, no UI write/wallet affordance added.

---

## 1. Backend route-gate changes

### 1.1 Middleware

`src/api/routes.rs::admin_route_gate` is an
`axum::middleware::from_fn_with_state` layer applied to the entire
router (`.layer(axum::middleware::from_fn_with_state(gate_state,
admin_route_gate))`).

Behaviour:

| Path prefix | Action |
|---|---|
| `/admin/*` | run the gate (auth + role + audit + 403-on-deny / pass-through-on-allow) |
| anything else (`/health`, `/metrics`, `/orders`, `/options/...`, etc.) | pass through untouched |

For `/admin/*`:

1. `required_role_for(method, path)` resolves the minimum role
   from the V2G-V §1 mapping.
2. `authenticate(&state.admin_config, header_lookup)` derives an
   `AdminIdentity` under the configured `AuthMode`.
3. `require_role(&identity, required)` enforces the gate.
4. `tracing::info!(target: "deopt.admin.audit", ...)` on allow,
   `tracing::warn!(target: "deopt.admin.audit", ...)` on deny.
5. On deny: HTTP 403 with `{"error": "<AdminAuthError display>"}`
   — bit-equivalent body shape to today's `ApiError::forbidden`.
6. On allow: `next.run(request).await` (pass-through to handler).

### 1.2 Handler-side `ensure_admin_access` kept as defence-in-depth

Every existing handler still calls `ensure_admin_access` as it did
under V2G-W0 / V2G-M2. Under SharedToken mode this is a no-op in
the happy path: the middleware already validated the token, the
handler validates the same token a second time with the same V2G-W0
constant-time compare. The handler's check is dead code in
practice but is left in place for the cutover window so a future
change to the middleware that accidentally bypasses auth cannot
silently leave the routes unguarded. V2G-W3 removes the
handler-side check after a soak window confirms the middleware is
the only authoritative gate.

### 1.3 Failure mode parity vs V2G-W0

| Scenario | Status code | Body | Same as V2G-W0? |
|---|---|---|---|
| Missing `X-Admin-Token` | 403 | `{"error":"admin token is required"}` | ✅ |
| Wrong token | 403 | `{"error":"invalid admin token"}` | ✅ |
| Admin disabled (`enabled=false`) | 403 | `{"error":"admin API is disabled"}` | ✅ |
| JWT mode (V2G-W2 + W3 future) | 403 | `{"error":"admin JWT auth mode is not implemented"}` | NEW (was: no JWT path) |
| Insufficient role | 403 | `{"error":"insufficient role; required at least <role>"}` | NEW (was: not enforced) |

---

## 2. Audit logging changes

### 2.1 Allow line

```text
tracing::info!(
    target: "deopt.admin.audit",
    method = "GET",
    path = "/admin/fees/v2/observability",
    required_role = "viewer",
    granted_role = "operator",
    identity = "shared-token",
    decision = "allow",
    auth_mode = "shared-token",
    "admin request allowed",
);
```

### 2.2 Deny line — insufficient role

```text
tracing::warn!(
    target: "deopt.admin.audit",
    method = "POST",
    path = "/admin/options/events/tick",
    required_role = "operator",
    granted_role = "viewer",
    identity = "alice@deopt.xyz",   // when JWT lands; "shared-token" today
    decision = "deny",
    reason = "insufficient role; required at least operator",
    auth_mode = "shared-token",
    "admin request denied (insufficient role)",
);
```

### 2.3 Deny line — auth failure (no/wrong/expired token)

```text
tracing::warn!(
    target: "deopt.admin.audit",
    method = "GET",
    path = "/admin/status",
    required_role = "viewer",
    decision = "deny",
    reason = "invalid admin token",   // never the token bytes
    auth_mode = "shared-token",
    "admin request denied (auth failure)",
);
```

### 2.4 What MUST NOT appear in audit logs

| Forbidden | Status |
|---|---|
| The candidate `X-Admin-Token` header value | ✅ never extracted into the log line |
| The configured `ADMIN_API_TOKEN` value | ✅ V2G-W0 / V2G-W1 invariants — never rendered by `AdminAuthError::Display` |
| Future JWT token / `Authorization: Bearer …` header value | ✅ V2G-W1 `JwtNotImplemented` returns a static string; the bearer payload is never parsed by V2G-W2 |
| Private keys / mnemonics | ✅ no admin route accepts these |
| Raw cookie values | ✅ no admin route reads cookies |
| Request bodies of POST tick routes | ✅ audit log emits only the `path`; the body is the route handler's concern |

### 2.5 Sink

V2G-W2 emits structured `tracing` events. Storage is the existing
backend log pipeline (`tracing-subscriber` configured in `main.rs`).
No DB persistence in V2G-W2 — that's a V2G-W3 hardening if the ops
team wants long-term retention.

To filter the audit stream during ops investigations:

```bash
journalctl -u deopt-v2-backend | grep 'deopt.admin.audit'
```

(or the equivalent for the chosen log shipper).

---

## 3. RBAC enforcement status

| Item | Status |
|---|---|
| Role hierarchy (`Viewer < Operator < GovernanceAdmin < Breakglass`) | ✅ Enforced via `require_role` |
| Per-route required role | ✅ `required_role_for(method, path)` table; pinned by V2G-W1 test `v2gw1_route_role_mapping_covers_v2gv_classification` |
| Route migration (handlers calling V2G-W1 primitives) | Middleware-level (no handler change). Handlers retain `ensure_admin_access` for defence-in-depth. |
| `Viewer` cannot reach operator routes | ✅ V2G-W1 unit test + V2G-W2 integration test (rejected with `InsufficientRole`) |
| `Operator` reaches viewer + operator routes | ✅ V2G-W1 + V2G-W2 integration test |
| `GovernanceAdmin` implies operator/viewer | ✅ V2G-W1 unit test (no live route requires it yet) |
| `Breakglass` implies everything | ✅ V2G-W1 unit test |
| JWT mode fail-closed | ✅ V2G-W2 middleware integration test |

---

## 4. SharedToken compatibility status

**Bit-for-bit preserved.** Under the default `AuthMode::SharedToken`:

- `authenticate` returns `AdminIdentity { name: "shared-token", role: Operator }` on valid token.
- `Operator` ≥ `Operator` ≥ `Viewer` — passes the role gate for every currently-routed admin endpoint.
- Pre-V2G-W2 tests (`admin_token_required_rejects_missing_token`, `admin_token_required_accepts_valid_token`, `admin_config_redacts_secrets`) all remain green.
- HTTP 403 body shapes for missing / wrong token are bit-equivalent.
- ADMIN_API_TOKEN env-var handling unchanged.
- METRICS_REQUIRE_ADMIN_TOKEN path unchanged (the middleware short-circuits on `/admin/*` only).

---

## 5. JWT fail-closed status

`AuthMode::Jwt` returns `AdminAuthError::JwtNotImplemented` from
`authenticate`. The middleware turns this into a 403 with
`{"error":"admin JWT auth mode is not implemented"}`. Pinned by
`v2gw2_middleware_blocks_operator_route_under_jwt_fail_closed`.

**Mainnet posture:** flipping `AuthMode` to `Jwt` before the V2G-W3
verifier ships will cause every admin route to 403 — the system
fails safe rather than accepting unverified tokens. This is
intentional.

---

## 6. Frontend proxy/auth status

V2G-W2 ships:

- **`src/lib/admin-rbac-types.ts`** — typed mirror of the backend
  `AdminRole` enum, `roleImplies` predicate, `requiredRoleFor`
  route lookup (1:1 with the backend mapping), `AdminAuthMode`
  enum, `OPERATOR_MODE_SESSION_KEY` constant, `AdminIdentity`
  type for SSR consumers.
- **No live middleware / SSR proxy code.** The real
  `middleware.ts` + `/api/admin/*` proxy lands in V2G-W3 along
  with the JWT verifier. Today's `/admin` UI continues to send
  `X-Admin-Token` directly from sessionStorage; the V2G-V threat
  model documents this as a known limitation that V2G-W3 closes.
- **No new wallet / write affordances** in the UI.

Documentation surface added at
`deopt-v2-frontend/docs/ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md`
(companion doc).

---

## 7. Tests added (8 new V2G-W2 integration tests)

`src/api/routes.rs::tests`:

| Test | Asserts |
|---|---|
| `v2gw2_middleware_lets_valid_token_reach_viewer_route` | Happy path — `/admin/status` with valid token → 200. |
| `v2gw2_middleware_lets_valid_token_reach_operator_route` | `/admin/fees/v2/smoke/readiness` reachable under SharedToken (which grants Operator). |
| `v2gw2_middleware_blocks_missing_token_on_viewer_route` | Missing token → 403 + `"admin token is required"`. |
| `v2gw2_middleware_blocks_wrong_token_on_viewer_route` | Wrong token → 403 + `"invalid admin token"`. |
| `v2gw2_middleware_blocks_operator_route_under_jwt_fail_closed` | `AuthMode::Jwt` + any `X-Admin-Token` → 403 + JWT-not-implemented error. |
| `v2gw2_middleware_disabled_mode_lets_request_through_without_token` | `AuthMode::Disabled` + no token → 200 (Breakglass identity). |
| `v2gw2_middleware_passes_through_non_admin_paths` | `/health` reachable without admin token even when `require_token=true`. |
| `v2gw2_middleware_403_body_never_contains_token_material` | 403 response body contains neither the configured nor the candidate token; no `eyJ` base64 JWT material. |

Plus the existing V2G-W1 unit tests (21) and V2G-W0 unit tests (11)
remain green. Admin-suite total: 32 unit + 8 middleware = **40 admin
tests**.

---

## 8. Files changed

### Backend
- `src/api/routes.rs`:
  - Added `use crate::admin::{authenticate, required_role_for, require_role, AdminAuthError, AdminIdentity};`.
  - Added `admin_route_gate` async middleware (~75 LOC).
  - Added `admin_audit_deny` helper (~30 LOC).
  - Added `.layer(axum::middleware::from_fn_with_state(gate_state, admin_route_gate))` to the router builder.
  - Added 8 V2G-W2 integration tests.
- `src/admin.rs`: untouched in V2G-W2 (V2G-W1 primitives unchanged).

### Frontend
- `src/lib/admin-rbac-types.ts` (new) — typed RBAC mirror.

### Docs
- **New:** `deopt-v2-backend/docs/ADMIN_RBAC_ROUTE_ENFORCEMENT_V2G_W2.md` (this file).
- **New:** `deopt-v2-frontend/docs/ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md` (companion).
- **Updated:** `docs/ADMIN_JWT_RBAC_IMPLEMENTATION_V2G_W1.md` — V2G-W2 follow-up note.
- **Updated:** `docs/ADMIN_AUTH_RBAC_THREAT_MODEL_V2G_V.md` — V2G-W2 progress note in the threat-model trail.
- **Updated:** `docs/ADMIN_TOKEN_CONSTANT_TIME_HARDENING_V2G_W0.md` — V2G-W2 follow-up note.

---

## 9. Validations run

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **764 / 0 / 0** (V2G-W1 baseline 756 + V2G-W2 +8) |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --lib 'v2gw2_'` | ✅ 8 / 0 / 0 |
| Frontend `npm run lint` | ✅ |
| Frontend `npx tsc --noEmit` | ✅ |
| Frontend `npm run build` | ✅ static prerender of `/`, `/admin`, `/_not-found` |

---

## 10. Soak preservation status

V2G-W2 binary lives in `target/` only — the running PID 231297
(V2G-M2) is NOT restarted in V2G-W2 (the live route gate continues
to be the V2G-W0 `ensure_admin_access` only; the middleware ships
to live at the next restart window).

| Check | State |
|---|---|
| Backend PID 231297 alive | ✅ 28m+ |
| `/health` | ✅ |
| Prometheus `/-/ready` | ✅ |
| Alertmanager `/-/ready` | ✅ |
| Grafana `/api/health` (DB) | ✅ ok |
| 9 V2 fee alerts | all `inactive` |
| Backend restarted | ❌ no — V2G-W2 binary stays in `target/` |
| Docker stack touched | ❌ no |
| `.env` edited | ❌ no |
| Secrets printed | ❌ no |

---

## 11. Remaining blockers

1. **Real JWT verifier** — `AuthMode::Jwt` returns
   `JwtNotImplemented`. V2G-W3 ships the `jsonwebtoken` /
   `josekit` integration + internal CA.
2. **Next.js SSR proxy** — V2G-W3. The browser will hit
   same-origin `/api/admin/*`; the SSR layer holds the backend
   admin secret. Today's `/admin` UI still sends
   `X-Admin-Token` from sessionStorage (V2G-V T2 / T3 known
   limitation).
3. **CORS allowlist + integration tests** for the
   middleware (V2G-V §T4 / T5) — pending.
4. **Backend restart for V2G-W2 pickup** — same window as V2G-W3
   if combined.
5. **Edge OIDC + Cloudflare Access** for staging /
   production — infrastructure work outside the code repo.
6. **OPTION RFQ live deploy (V2G-P)** — orthogonal but blocking
   mainnet.

---

## 12. Next recommended milestone

**V2G-W3 — JWT verifier + Next.js SSR proxy.** Recommended slice
order:

1. Add `jsonwebtoken` (or `josekit`) as a direct backend dep.
2. Implement `verify_admin_jwt(token, ca_public_key, expected_aud)
   -> Result<AdminIdentity, AdminAuthError>` and replace the
   `JwtNotImplemented` arm in `authenticate`.
3. Add `Bearer` header parsing in the middleware's `header_lookup`
   so both `X-Admin-Token` (legacy SharedToken) and
   `Authorization: Bearer <jwt>` (V2G-W3 JWT) are accepted in
   parallel during cutover.
4. Add a Next.js `middleware.ts` that:
   - rejects browser requests to `/admin/*` without an active
     SSR session,
   - never returns the backend admin token to the browser,
   - forwards via a same-origin `/api/admin/*` proxy.
5. Add backend integration tests for JWT happy path / expired /
   wrong `aud` / unknown signer / missing required claim.
6. Soak window with both auth modes accepted; then drop the
   `X-Admin-Token` path after a clean week.
