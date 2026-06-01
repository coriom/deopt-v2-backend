# V2G-W1 — Admin JWT / RBAC Implementation (Offline, Backwards-Compatible)

## Status

- Milestone: **V2G-W1** — adds the role / identity / auth-mode
  model to the backend offline, behind the existing
  `AdminConfig` surface. No live route handler is migrated.
  Existing ADMIN_API_TOKEN behaviour preserved exactly.
- Date: 2026-06-01.
- Outcome:
  - **1 source file extended** (`src/admin.rs`, additive only).
  - **21 new unit tests** (all `v2gw1_*`, all green).
  - **No new dependencies.** JWT verifier is intentionally a
    stub (`AdminAuthError::JwtNotImplemented`) — a future
    milestone adds the verifier crate + integration tests.
  - **Live backend NOT restarted.** V2G-M2 binary continues to
    enforce shared-token auth via the V2G-W0 constant-time path.
    The new role model lives in the freshly-compiled binary in
    `target/`; promotion to the live process is the V2G-W2
    milestone.
- Hard gates respected: no broadcast, no chain mutation, no
  backend restart, no Docker / Prometheus restart, no `.env`
  edit, no DB writes, no secret printed.

---

## 1. Auth model

Three configured modes, selected via `AdminConfig::auth_mode`:

| Mode | Behaviour | Identity returned | Status |
|---|---|---|---|
| `SharedToken` (default) | V2G-W0 constant-time `X-Admin-Token` compare. | `AdminIdentity { name: "shared-token", role: Operator }` on success. | **LIVE** today (V2G-M2). |
| `Jwt` | (future) per-identity bearer JWT against an internal CA. | TBD — verifier returns `AdminAuthError::JwtNotImplemented` in V2G-W1. | **NOT IMPLEMENTED** — fail-closed. |
| `Disabled` | Local-dev-only escape hatch. NEVER for staging / prod. | `AdminIdentity { name: "local-dev-no-auth", role: Breakglass }`. | Behaves as expected — covered by tests. |

`AdminConfig::auth_mode()` returns the resolved mode; the
underlying field is `Option<AuthMode>` so existing
`AdminConfig::new(enabled, require_token, token)` call sites
continue to compile without touching production config files.

`Debug` for `AdminConfig` now renders the auth mode (`shared-token`,
`jwt`, `disabled`) alongside the existing `<redacted>` / `<unset>`
token marker.

## 2. RBAC roles

`AdminRole` enum, ordered so `>=` reflects authority:

| Role | Value | Authority |
|---|---|---|
| `Viewer` | 0 | read-only admin GETs |
| `Operator` | 1 | preflight + mutating backend POSTs |
| `GovernanceAdmin` | 2 | (future) governance-payload routes |
| `Breakglass` | 3 | (future) emergency endpoints |

Higher roles imply lower — `Breakglass` can do everything an
`Operator` can, etc. Encoded by `AdminRole::implies`.

## 3. Route → role mapping

`required_role_for(method, path)` returns the minimum role
required to call a given endpoint. Pinned from the V2G-V §1
classification:

| Route | Method | Required role |
|---|---|---|
| `/admin/status` | GET | viewer |
| `/admin/config` | GET | viewer |
| `/admin/db` | GET | viewer |
| `/admin/recent` | GET | viewer |
| `/admin/mm/sessions` | GET | viewer |
| `/admin/mm/permissions` | GET | viewer |
| `/admin/execution/summary` | GET | viewer |
| `/admin/rfq/summary` | GET | viewer |
| `/admin/options/summary` | GET | viewer |
| `/admin/options/confirmations` | GET | viewer |
| `/admin/options/events` | GET | viewer |
| `/admin/options/reconciliations` | GET | viewer |
| `/admin/options/executions/:intent_id/lifecycle` | GET | viewer |
| `/admin/fees/summary` | GET | viewer |
| `/admin/fees/events` | GET | viewer |
| `/admin/fees/onchain` | GET | viewer |
| `/admin/fees/volumes` | GET | viewer |
| `/admin/fees/rebates` | GET | viewer |
| `/admin/fees/v2/observability` | GET | viewer |
| **`/admin/fees/v2/smoke/readiness`** | **GET** | **operator** |
| **`/admin/options/events/tick`** | **POST** | **operator** |
| **`/admin/options/reconciliations/tick`** | **POST** | **operator** |

Pinned by test `v2gw1_route_role_mapping_covers_v2gv_classification`.
HTTP method comparison is case-insensitive. Any unmapped route
defaults to `Viewer` — the safest reading for a read-only admin
surface.

## 4. Compatibility with ADMIN_API_TOKEN

`AuthMode::SharedToken` is the default. The new
`authenticate(config, header_lookup)` function:

- Returns `AdminAuthError::AdminDisabled` if `admin.enabled =
  false` (matches today's `ensure_admin_access`).
- When `require_token = false`, returns
  `AdminIdentity { name: "shared-token-unrequired", role:
  Operator }` so the role gate still has an identity to use
  even on test environments that didn't configure a token.
- When `require_token = true`:
  - Missing `X-Admin-Token` header ⇒ `SharedTokenHeaderMissing`.
  - Wrong token ⇒ `SharedTokenInvalid` (V2G-W0 constant-time
    compare).
  - Valid token ⇒ `AdminIdentity { name: "shared-token", role:
    Operator }`.

The existing `ensure_admin_access` function in `src/api/routes.rs`
is **untouched** — production behavior for every `/admin/*`
endpoint is bit-for-bit unchanged. V2G-W1 only adds the new
primitive; no route handler calls it yet.

## 5. Implementation status

| Component | Status |
|---|---|
| `AdminRole` enum + ordering + `implies()` | ✅ done, tested |
| `AdminIdentity` struct with redacted Debug | ✅ done, tested |
| `AuthMode` enum (SharedToken / Jwt / Disabled) | ✅ done, tested |
| `AdminConfig::auth_mode` + getter + setter | ✅ done, tested |
| `AdminConfig::Debug` shows auth mode (token still redacted) | ✅ done, tested |
| `required_role_for(method, path)` lookup | ✅ done, tested |
| `authenticate(config, header_lookup)` entry | ✅ done, tested |
| `require_role(identity, required)` helper | ✅ done, tested |
| `AdminAuthError` taxonomy + `Display` redaction | ✅ done, tested |
| **JWT verifier (real implementation)** | ❌ **stubbed** as `JwtNotImplemented` — fail-closed |
| **Route handlers migrated to role gate** | ❌ deferred to V2G-W2 |
| **`audit_log` writer** | ❌ deferred (V2G-V §6) |

## 6. Tests added (21)

| Test | Asserts |
|---|---|
| `v2gw1_role_authority_ordering` | `Viewer < Operator < GovernanceAdmin < Breakglass`; `implies()` semantics. |
| `v2gw1_default_auth_mode_is_shared_token` | Existing `AdminConfig::new(...)` defaults to `SharedToken`. |
| `v2gw1_route_role_mapping_covers_v2gv_classification` | Every V2G-V §1 routing decision is enforced; uppercase + lowercase method input both work. |
| `v2gw1_authenticate_shared_token_accepts_valid` | Identity = `shared-token`, role = `Operator`. |
| `v2gw1_authenticate_shared_token_rejects_missing_header` | `SharedTokenHeaderMissing`. |
| `v2gw1_authenticate_shared_token_rejects_wrong_value` | `SharedTokenInvalid`. |
| `v2gw1_authenticate_when_admin_disabled` | `AdminDisabled`. |
| `v2gw1_authenticate_shared_token_when_require_token_false` | Returns the "unrequired" identity so role gate works on test envs. |
| `v2gw1_authenticate_jwt_mode_is_unimplemented` | Fail-closed `JwtNotImplemented` even with a JWT-looking header. |
| `v2gw1_authenticate_disabled_mode_returns_breakglass_identity` | `local-dev-no-auth` / `Breakglass` — explicit local-dev path. |
| `v2gw1_require_role_viewer_can_access_viewer_route` | viewer ⇒ viewer. |
| `v2gw1_require_role_viewer_cannot_access_operator_route` | viewer ⇒ operator: `InsufficientRole`. |
| `v2gw1_require_role_operator_can_access_operator_route` | operator ⇒ operator. |
| `v2gw1_require_role_operator_can_access_viewer_route` | operator ⇒ viewer (implication). |
| `v2gw1_require_role_governance_admin_can_access_operator_route` | governance-admin ⇒ operator. |
| `v2gw1_require_role_breakglass_can_access_everything` | Breakglass passes all four levels. |
| `v2gw1_auth_error_messages_dont_leak_tokens` | Every `AdminAuthError` Display string is checked against candidate + configured token + base64 `"eyJ"` flavour. |
| `v2gw1_insufficient_role_error_does_not_leak_granted_role` | Only the *required* role is rendered to the wire; the *granted* role is kept private. |
| `v2gw1_admin_identity_debug_redacts_name` | `Debug` for `AdminIdentity` renders `<redacted>` for `name`, exposes `role`. |
| `v2gw1_admin_config_debug_reports_auth_mode` | `Debug` for `AdminConfig` reports the auth mode without leaking the token. |
| `v2gw1_token_compare_behavior_unchanged_under_role_model` | Regression-pin: shared-token `authenticate` exercises the V2G-W0 constant-time path for accept / same-length-wrong / short-wrong. |

## 7. Files changed

| File | Change |
|---|---|
| `src/admin.rs` | Additive: `AdminRole`, `AdminIdentity`, `AuthMode`, `AdminAuthError`, `required_role_for`, `authenticate`, `require_role` + 21 unit tests. `AdminConfig` gained `auth_mode: Option<AuthMode>` (defaults to `None` ⇒ `SharedToken`). `Debug` impl gains an `auth_mode` field. No other call sites required. |
| `docs/ADMIN_JWT_RBAC_IMPLEMENTATION_V2G_W1.md` | NEW (this file). |
| `docs/ADMIN_AUTH_RBAC_THREAT_MODEL_V2G_V.md` | V2G-W1 progress note. |
| `docs/ADMIN_TOKEN_CONSTANT_TIME_HARDENING_V2G_W0.md` | V2G-W1 progress note. |

## 8. Validations run

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean (post-`cargo fmt`) |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **756 / 0 / 0** (V2G-W0 baseline 735 + V2G-W1 +21) |
| `cargo build --all-targets --all-features` | ✅ |
| `cargo test --lib admin::tests` | ✅ 32 / 0 / 0 (11 V2G-W0 + 21 V2G-W1) |
| Frontend checks | not run — frontend not touched |

## 9. Soak preservation status

V2G-W1 is offline-source-only. Read-only post-edit verification:

| Check | State |
|---|---|
| Backend PID 231297 alive | ✅ (V2G-M2 process, unchanged) |
| `/health` | ✅ `{"ok":true,...}` |
| Prometheus `/-/ready` | ✅ |
| Alertmanager `/-/ready` | ✅ |
| Grafana `/api/health` | ✅ `database=ok` |
| 9 V2 fee alerts | all `inactive` |
| Backend restarted | ❌ no — V2G-W1 binary in `target/` only |
| Docker stack touched | ❌ no |
| `.env` edited | ❌ no |
| Secrets printed | ❌ no |

## 10. Remaining blockers

1. **JWT verifier** — `AuthMode::Jwt` returns `JwtNotImplemented`.
   Real implementation needs an internal CA + verifier crate
   (`jsonwebtoken`, `josekit`, or equivalent). Deferred to a
   future milestone.
2. **Route migration** — every `/admin/*` route handler still
   uses `ensure_admin_access`. Migrating them to call
   `authenticate` + `require_role` is the V2G-W2 milestone.
3. **Audit log writer** (V2G-V §6) — pending.
4. **Next.js SSR proxy + drop sessionStorage token**
   (V2G-V §3.3 / V2G-W3) — pending.
5. **CORS allowlist + integration tests** (V2G-V §T4 / T5) —
   pending.
6. **Edge OIDC + Cloudflare Access for staging / production**
   (V2G-V §3.2 / §3.3) — infrastructure work outside the code
   repo.

## 11. Next recommended milestone

**V2G-W2 — migrate route handlers to the role gate + add the
audit-log writer.** Smallest-slice ordering:

1. Add a single `ensure_admin_role(headers, required) ->
   Result<AdminIdentity>` helper in `src/api/routes.rs` that
   wraps `authenticate` + `require_role` + maps `AdminAuthError`
   to the existing `ApiError` taxonomy.
2. Migrate one read-only GET route (e.g. `/admin/status`) as a
   pilot. Behaviour identical, but call site is now
   `let identity = ensure_admin_role(&state, &headers,
   required_role_for("GET", "/admin/status"))?;`.
3. Add an `audit_log` writer module called from the gate prelude
   that emits one JSON line per request with `(ts, request_id,
   identity, route, method, response_status, response_ms)`.
   Identity is logged; token / JWT bytes are NOT.
4. Migrate the remaining routes (one PR per route family).
5. Once all routes are migrated, deprecate `ensure_admin_access`.

V2G-W2 is independently shippable from the actual JWT verifier
because the V2G-W1 model already returns useful identities
under `SharedToken` mode. JWT lands as V2G-W3 alongside the
Next.js SSR proxy.
