# V2G-V — Admin Dashboard Auth, RBAC, and Threat Model

## Status

- Milestone: **V2G-V** — formal security spec for the admin dashboard
  (frontend) and the backend `/admin/*` route family. **Docs-only.**
  No code changes, no runtime activation, no live mutation.
- Date: 2026-06-01.
- Outcome:
  - Full inventory + classification of every `/admin/*` route the
    backend exposes today.
  - Four-tier RBAC model defined (viewer / operator /
    governance-admin / breakglass) — design only, no enforcement
    code yet.
  - Three-environment access architecture (local/testnet / staging /
    production) recommended.
  - Secret-handling policy carried through from V2G-O / V2G-P0 / V2G-R0
    docs and pinned here as a single source of truth.
  - Threat model with 8 named threats, mitigations, and follow-up
    test list.
- Hard gates respected: no broadcast, no deploy, no backend
  restart, no Docker / Prometheus reset, no `.env` edit, no DB
  writes, no wallet writes, no auth implementation code, no
  Solidity / backend runtime changes.

---

## 1. Admin Route Inventory + Classification

Backend source: `src/api/routes.rs:258-297` (router registration) +
`ensure_admin_access` gate at `src/api/routes.rs:1442`. Every route
below currently passes through the SAME gate (single shared admin
token; no role distinction).

| Route | Method | Side effect | Current gate | RBAC classification |
|---|---|---|---|---|
| `/admin/status` | GET | none | admin-token | **viewer** |
| `/admin/config` | GET | none (secrets redacted) | admin-token | **viewer** |
| `/admin/db` | GET | none | admin-token | **viewer** |
| `/admin/recent` | GET | none | admin-token | **viewer** |
| `/admin/mm/sessions` | GET | none | admin-token | **viewer** |
| `/admin/mm/permissions` | GET | none | admin-token | **viewer** |
| `/admin/execution/summary` | GET | none | admin-token | **viewer** |
| `/admin/rfq/summary` | GET | none | admin-token | **viewer** |
| `/admin/options/summary` | GET | none | admin-token | **viewer** |
| `/admin/options/confirmations` | GET | none | admin-token | **viewer** |
| `/admin/options/events` | GET | none | admin-token | **viewer** |
| `/admin/options/reconciliations` | GET | none | admin-token | **viewer** |
| `/admin/options/executions/:intent_id/lifecycle` | GET | none | admin-token | **viewer** |
| `/admin/fees/summary` | GET | none | admin-token | **viewer** |
| `/admin/fees/events` | GET | none | admin-token | **viewer** |
| `/admin/fees/onchain` | GET | none | admin-token | **viewer** |
| `/admin/fees/volumes` | GET | none | admin-token | **viewer** |
| `/admin/fees/rebates` | GET | none | admin-token | **viewer** |
| `/admin/fees/v2/observability` | GET | none | admin-token | **viewer** |
| `/admin/fees/v2/smoke/readiness` | GET | none (post-restart) | admin-token | **operator-preflight** |
| `/admin/options/events/tick` | POST | **advances event indexer watermark** in DB | admin-token | **operator** |
| `/admin/options/reconciliations/tick` | POST | **runs reconciliation tick** (DB writes) | admin-token | **operator** |

### Classifications

| Class | Description | Examples |
|---|---|---|
| **public-forbidden** | Routes that must NEVER be reachable from the browser, even with a token. | none today — but candidate future routes: rebate-budget setter, merkle-root setter, fee-recipient setter (all of which currently live on Solidity owner-only methods, not on the backend admin API). |
| **read-only admin** | GET-only routes that produce structured snapshots. Safe to expose to anyone in the operations group. | all current GET admin routes (19 of 21). |
| **operator preflight** | Routes that produce calldata / packet payloads for off-band signing. NEVER sign or broadcast themselves. Currently only `/admin/fees/v2/smoke/readiness` belongs here; future RFQ / vault preflight endpoints will join. | `/admin/fees/v2/smoke/readiness` (today, code-ready not live until backend restart). |
| **operator** (mutating) | Routes that mutate **backend** state (worker watermarks, reconciliation runs) but never broadcast on-chain. | `/admin/options/events/tick`, `/admin/options/reconciliations/tick`. |
| **governance-admin** | Routes that produce or verify governance broadcast payloads. None exist today. Designed as: read-only construction of timelock-targeted calldata for off-band timelock submission. | (future — V2G-P operator window, V2G-R5 vault deploy window). |
| **emergency / breakglass** | Routes that fire backend-side panic actions (pause workers, disable broadcasts, expire admin tokens). None exist today. Must require an additional out-of-band confirm header. | (future). |

---

## 2. RBAC Roles

Four roles, organised from least to most authority. Operating
principle: **never one role for everything** — split read access
from operator-preflight from mutation from breakglass.

### 2.1 `viewer`

- **Purpose:** dashboard observability. Read every metric, every
  status snapshot, every per-tx fee summary.
- **Backend routes:** all `read-only admin` GETs.
- **Frontend pages:** `/admin` dashboard, but mutation-class
  buttons / operator-mode panels hidden.
- **Auth:** OIDC / Cloudflare-Access identity. No long-lived
  shared secret.
- **Expected operators:** entire engineering + ops team. Largest
  pool.

### 2.2 `operator`

- **Purpose:** generate preflight calldata, fetch operator packets,
  run indexer/reconciliation ticks.
- **Backend routes:** all `viewer` routes + `operator preflight` +
  `operator (mutating)`. Specifically the `/admin/fees/v2/smoke/readiness`
  V2G-M endpoint + the tick POSTs.
- **Frontend pages:** dashboard with the operator-mode panel
  unhidden. RFQ packet generation button enabled (still off-band
  signing). No signing flow in the UI itself.
- **Auth:** OIDC identity in an `operators` group + the explicit
  `OPERATOR_MODE_CONSENT=true` env-side gate (carried from
  V2G-P1).
- **Expected operators:** 3-5 named individuals.

### 2.3 `governance-admin`

- **Purpose:** generate timelock-targeted calldata for governance
  proposals (V2G-P broadcast, V2G-R5 vault deploy, fee schedule
  rotations, merkle-root rotations, rebate-budget changes).
- **Backend routes:** none today (the governance flow is currently
  100% off-band). Future endpoints will produce: redeploy preflight
  reports, rewire preflight reports, fee-profile-set calldata,
  merkle-root-set calldata.
- **Frontend pages:** "Governance Workbench" panel (out of V2G-V
  scope) that displays generated calldata + the timelock target
  + a "copy to off-band signer" affordance. NEVER signs from the
  UI.
- **Auth:** OIDC identity in a `governance` group + hardware-key
  attestation header (e.g. YubiKey U2F response) that the backend
  validates against a registered key list.
- **Expected operators:** 2-3 named individuals.

### 2.4 `breakglass`

- **Purpose:** emergency pauses. Disable broadcasts globally,
  expire all admin tokens, pause indexer workers, flip
  `useFeesManagerV2` back to V1 if FM-V2 misbehaves.
- **Backend routes:** none today. Future emergency endpoints will
  require:
  - the requesting identity to be in the `breakglass` group AND
  - a 2-person quorum (header carries a second identity's signed
    attestation) AND
  - a per-action 10-second confirm window (the endpoint returns a
    "confirm token" first; the actual action requires a second
    request with that token within the window).
- **Frontend pages:** "Breakglass" panel hidden behind a separate
  URL (`/admin/breakglass`) that requires the role at edge.
- **Auth:** OIDC + hardware-key + quorum.

### Role assignment matrix

| Route class | viewer | operator | governance-admin | breakglass |
|---|---|---|---|---|
| read-only admin | ✅ | ✅ | ✅ | ✅ |
| operator preflight | — | ✅ | ✅ | ✅ |
| operator (mutating) | — | ✅ | ✅ | ✅ |
| governance-admin | — | — | ✅ | ✅ |
| breakglass | — | — | — | ✅ |

Role *implies* every lower-authority role's access. There is no
"governance-admin without operator" (they need the same situational
awareness as operators).

---

## 3. Environment Access Model

### 3.1 Local / testnet (Base Sepolia, today)

- **Backend `ADMIN_API_REQUIRE_TOKEN=true`**, single shared admin
  token, value lives in operator `.env.broadcast.v2e_g.local` and is
  pasted into the admin dashboard sessionStorage.
- No OIDC / Cloudflare Access in front of the dashboard. The
  dashboard runs from `localhost:3000` directly against the local
  backend.
- All four RBAC roles collapse to "anyone with the token can do
  anything that the token can do". Acceptable for solo-operator
  development.
- **Posture:** weakest. Acceptable because the chain is testnet,
  the backend has `real_broadcast_enabled=false` for everything
  except explicitly-confirmed broadcast windows, and the operator
  pool is one person.

### 3.2 Staging (post-V2G-V close)

- OIDC SSO (Google Workspace / Auth0 / similar) in front of the
  dashboard.
- Cloudflare Access (or equivalent) restricts the dashboard URL
  to a named group (`@deopt.xyz` domain + 2FA).
- Backend `/admin/*` continues to require the shared token, but
  the token never appears in browser sessionStorage. Instead a
  Next.js server-side proxy holds the token in
  `process.env.BACKEND_ADMIN_TOKEN` and forwards request from an
  identified, SSO-validated session.
- Roles enforced at the Next.js proxy layer based on OIDC group
  membership. No backend-side RBAC code change yet.
- **Posture:** acceptable for staging traffic. Survives shoulder
  surfing, leaked sessionStorage, and untrusted-WiFi exfiltration.

### 3.3 Production

- Cloudflare Access (or AWS Cognito / equivalent) at the edge with
  hardware-key MFA mandatory.
- Backend admin auth promoted from shared token to **per-identity
  JWT** signed by an internal CA. Token includes:
  - `sub` (user identity)
  - `groups` (RBAC roles)
  - `exp` (≤ 1 hour)
  - `aud` (`deopt-v2-backend-admin`)
  - `iss` (`deopt-internal-ca`)
- Backend verifies the JWT signature + groups + expiry per request.
- Next.js server-side proxy is the only direct caller of
  `/admin/*`. Browsers never hold any admin secret.
- Audit log of every `/admin/*` request: `(timestamp, request_id,
  identity, group, route, method, response_status)`. No request
  body, no headers, no path parameters that could carry a secret.
- Breakglass actions additionally require a hardware-key
  attestation + 2-person quorum (see §2.4).
- **Posture:** auditable, revocable per-identity. Mainnet
  prerequisite.

---

## 4. Recommended Production Architecture

```
┌─────────────────┐   OIDC (Google) + Cloudflare Access edge
│   Browser       │   ───────────────────────────────────────►
│   /admin/*      │                                            │
└─────────────────┘                                            ▼
                                                ┌──────────────────────┐
                                                │ Next.js (server-side)│
                                                │ /admin/* middleware  │
                                                │ • verify OIDC group   │
                                                │ • mint JWT for caller│
                                                │ • proxy to backend   │
                                                │ • audit log line     │
                                                └─────────┬────────────┘
                                                          │ JWT
                                                          ▼
                                              ┌────────────────────────┐
                                              │ deopt-v2-backend       │
                                              │ /admin/* handlers      │
                                              │ • verify JWT sig+exp+aud│
                                              │ • role gate per route  │
                                              │ • audit log line       │
                                              └────────────────────────┘
```

### Concrete change set (deferred to V2G-V impl milestone)

1. **Edge**: Cloudflare Access (or equivalent) over the dashboard
   hostname.
2. **Frontend**: introduce `middleware.ts` in `deopt-v2-frontend`
   that:
   - rejects browser requests to `/admin/*` without an active
     OIDC session,
   - never returns the backend admin token to the browser,
   - forwards admin requests via a server-side `/api/admin/*` proxy
     in the same Next app.
3. **Frontend admin-api.ts**: drop `X-Admin-Token` injection from
   the browser; point all `fetch` calls at the same-origin
   `/api/admin/*` proxy.
4. **Backend**: replace `ensure_admin_access` shared-token check
   with `ensure_admin_jwt(headers, required_role)` that:
   - parses an `Authorization: Bearer <jwt>` header,
   - verifies signature via internal CA public key,
   - checks `aud == "deopt-v2-backend-admin"`,
   - checks `exp > now`,
   - checks `groups` includes `required_role` (per-route declared),
   - returns the `sub` to the handler for audit logging.
5. **Backend constant-time compare**: even in the transitional
   shared-token mode, replace `self.token == candidate` with
   `subtle::ConstantTimeEq::ct_eq` to close the V2G-V finding §6.
6. **Audit log writer**: a single function called from each
   handler's prelude. Writes one line per request with the schema
   from §7.
7. **Migration plan**: run shared-token and JWT in parallel during
   the cutover window (`ensure_admin_access` accepts either),
   rotate the shared token to a random 256-bit value, drop the
   shared path after the cutover window closes.

V2G-V ships **none** of this — it documents the target. The
shared-token model continues to gate the soak.

---

## 5. Secret-Handling Policy

Carried through from V2G-O / V2G-P0 / V2G-P1 / V2G-R0 / V2G-R1.
Pinned here:

### 5.1 What MUST NOT live anywhere reachable from the dashboard

- Deployer private key (`DEPLOYER_PRIVATE_KEY`).
- Maker / taker smoke EOA private keys.
- Multisig signer keys.
- Backend service-account database password / RPC API keys.
- Cloudflare Access edge cert private key.
- TLS server cert private key.

### 5.2 What may live in the dashboard (read-only views only)

- Public chain addresses (EOAs and contracts).
- Public RPC URL (Base Sepolia).
- Chain ID.
- Read-only event payloads + decoded fields.
- Status flags (`broadcast_enabled=false`, `execution_enabled=true`,
  etc.).

### 5.3 What the backend admin API may return (per route)

- **Never:** any private key, any signing-account secret material,
  the admin token itself.
- **Always redacted** in `/admin/config`: `admin_token`, every
  `*_PRIVATE_KEY` env entry, every `*_SECRET` env entry. Confirmed
  by `admin_config_redacts_secrets` test (`src/api/routes.rs:5908`).
- **Address-only** EOA registry: `tier4_maker_address`,
  `tier2_taker_address` returned, key env vars referenced by NAME
  only (e.g. `key_env_vars.maker = "OPTION_SMOKE_BUYER_PRIVATE_KEY"`),
  never by value.

### 5.4 Operator signing flow (off-band only)

Operators sign maker / taker / governance payloads with their own
hardware wallets or shell-only signing CLIs (`sign_option_execution_intent`,
`sign_perp_trade`, V2G-P1 RFQ operator packet). The dashboard
displays:

- the EIP-712 digest the operator should sign (V2G-P1
  `OptionRfqOperatorPacket.digest_hex`),
- the calldata that the operator should broadcast once the
  signatures are collected (V2G-P1 `calldata_hex`),
- the function selector for sanity-checking,

but the dashboard NEVER:

- holds a private key,
- presents a "Sign and broadcast" button,
- calls `eth_sendTransaction` / `personal_sign` on any wallet,
- writes any signature to backend storage.

---

## 6. Audit Logging

### 6.1 Schema

One JSON line per admin request. No request bodies, no headers
beyond identity, no path parameters that could carry a secret.

```json
{
  "ts": "2026-06-01T18:30:00.123Z",
  "request_id": "01HJK7Z…",
  "identity": "alice@deopt.xyz",
  "groups": ["operator"],
  "route": "/admin/fees/v2/observability",
  "method": "GET",
  "response_status": 200,
  "response_ms": 42,
  "source_ip": "203.0.113.7"
}
```

### 6.2 Sink

- Local / testnet: stdout JSON line, ingested by the standard
  backend log pipeline. No structured retention beyond the local
  loki / grafana stack.
- Staging / production: dedicated `admin_audit` log stream,
  retention ≥ 90 days, write-only IAM role (no operator can
  delete prior entries).

### 6.3 What MUST NOT appear in the audit log

- Request bodies (some endpoints accept query parameters with
  potentially sensitive values — drop them at log time).
- The admin token / JWT / cookie values.
- Decoded fee event payloads (PII surfaces are minimal but
  trader addresses are PII-adjacent — keep them out of the audit
  stream).
- Any value from `process.env` other than `NODE_ENV`.

---

## 7. Threat Model

### T1. Exposed admin route

| Aspect | Detail |
|---|---|
| Threat | An attacker discovers the admin dashboard URL or the backend `/admin/*` URL directly (search engine indexing, DNS scan, leaked Slack link). |
| Today | Backend rejects with `forbidden` when `admin_config.require_token=true`. Dashboard requires the token to be pasted in. Cloudflare Access NOT in front. |
| Production mitigation | Cloudflare Access edge with mandatory OIDC + hardware MFA. Origin firewall rejects every non-Cloudflare-Access-fronted request. |
| Residual risk | Phished OIDC session — mitigated by hardware MFA. |

### T2. Stolen admin token

| Aspect | Detail |
|---|---|
| Threat | Operator pastes the shared token into a debugger / screenshot / Slack message; or a malicious browser extension reads `sessionStorage.deopt.adminToken`. |
| Today | Single shared token, no rotation hooks, sessionStorage exposure. Token equality compared with `==` (non-constant-time — see §C below). |
| Production mitigation | Per-identity JWT, ≤ 1 hour expiry, signed by internal CA, never stored in the browser (server-side proxy holds it). Audit log catches replay across identities. |
| Residual risk | Stolen short-lived JWT during its expiry window. Mitigation: ops rotates the CA signing key on incident; running JWTs invalidate immediately. |

### T3. XSS in the admin dashboard

| Aspect | Detail |
|---|---|
| Threat | A reflected or stored XSS payload runs in `/admin` and exfiltrates `sessionStorage.deopt.adminToken` or sends authenticated requests on behalf of the operator. |
| Today | The dashboard renders backend JSON via React's auto-escaping. No `dangerouslySetInnerHTML` usage in the admin tree (audited V2G-U). All fetched data passes through the type-safe JSON helpers in `src/types/admin.ts`. |
| Production mitigation | Strict CSP (`default-src 'self'`, no inline scripts), Trusted Types, server-side proxy means even an XSS payload cannot ship the admin secret (no browser-side admin secret). |
| Residual risk | A malicious npm package compromise. Mitigation: `npm audit` in CI, lockfile review on every dep bump, no `postinstall` script execution from untrusted deps. |

### T4. CSRF if cookie-based auth replaces the header

| Aspect | Detail |
|---|---|
| Threat | If V2G-V impl switches to cookie-stored JWT, a cross-origin form on attacker-controlled domain can submit POST requests with the auth cookie. |
| Today | N/A — header-based auth is immune to CSRF. |
| Production mitigation | Use `SameSite=Strict` cookies, double-submit-CSRF-token pattern (cookie + matching header set by the SPA via Origin-restricted JS), and reject all POST `/admin/*` requests whose `Origin` header isn't in the allowlist. |
| Residual risk | A misconfigured proxy that strips `Origin`. Mitigation: integration test that asserts the backend rejects POST `/admin/options/events/tick` when `Origin` is missing or unallowlisted. |

### T5. CORS misconfiguration

| Aspect | Detail |
|---|---|
| Threat | `Access-Control-Allow-Origin: *` paired with `Access-Control-Allow-Credentials: true` (a browser-spec invariant violation) — or an over-broad regex match — lets attacker-controlled origins issue authenticated requests. |
| Today | Backend `/admin/*` routes do NOT serve CORS preflight responses (no `OPTIONS` handler). Browser cross-origin admin calls fail. |
| Production mitigation | CORS allowlist constrained to the Next.js dashboard origin only. No wildcard. No `Allow-Credentials` unless the cookie path is strictly required (prefer the proxy pattern). |
| Residual risk | A future endpoint accidentally added with permissive CORS. Mitigation: V2G-V impl adds an integration test that hits every `/admin/*` route with a forbidden `Origin` and asserts rejection. |

### T6. SSRF via admin fetches

| Aspect | Detail |
|---|---|
| Threat | An admin endpoint that takes a URL parameter and fetches it server-side could be coerced into fetching internal services (cloud metadata at `169.254.169.254`, internal RDS, etc.). |
| Today | NO admin endpoint accepts a user-supplied URL. All RPC URLs come from `process.env`. No SSRF surface today. |
| Production mitigation | Maintain the invariant: any future admin endpoint that takes a URL parameter MUST validate against a strict allowlist. The Next.js proxy similarly forwards only to the configured backend URL. |
| Residual risk | A regression in a future endpoint. Mitigation: a CI lint that flags any `reqwest::Client::get(user_input)` pattern inside `src/api/routes.rs`. |

### T7. Leaked `.env` / secret in process listing

| Aspect | Detail |
|---|---|
| Threat | Operator pastes `.env` into a chat; process listing leaks the shared admin token; a dump of `/proc/[pid]/environ` exposes secrets. |
| Today | Shared admin token is in `.env.broadcast.v2e_g.local`. The token is sent as plaintext in HTTP header (TLS protects in transit; cleartext on the host). |
| Production mitigation | JWT model — no long-lived shared secret. CA private key lives in HSM. `process.env` contains only the public key for verification. |
| Residual risk | Compromised host. Out of V2G-V scope; covered by infrastructure-security posture. |

### T8. Replay of operator packets

| Aspect | Detail |
|---|---|
| Threat | An operator builds a V2G-P1 RFQ packet (digest + calldata) and the digest is captured in transit / persisted. An attacker tries to re-submit the same calldata to the live OptionMatchingEngine. |
| Today | Mitigated structurally: the RFQ struct includes `buyerNonce`, `sellerNonce`, `deadline`, and `intentId`. The MarginEngine consumes nonces, so the second `executeRfqTrade` call with the same payload reverts. Signatures cannot replay across the ORDERBOOK / RFQ flows (V2G-O `RFQ_TRADE_TYPEHASH` ≠ `TRADE_TYPEHASH`). |
| Production mitigation | (current) Solidity-side nonce + deadline + typehash separation. (added) Backend records the digest of every operator packet it produces; refuses to produce the same digest twice within a window. |
| Residual risk | Operator with timelock authority deliberately re-broadcasting — covered by governance, not by this threat model. |

---

## V2G-W2 progress note (appended 2026-06-01)

V2G-W2 wires the V2G-W1 primitives into the live `/admin/*` route
gate via an `axum::middleware::from_fn_with_state` layer. Audit
logging is emitted as `tracing::info!` / `tracing::warn!` with
`target: "deopt.admin.audit"`, capturing method / path / required
role / granted role / identity / decision / auth_mode. The
audit-log line never carries token bytes / JWT bytes / private
keys. JWT mode remains fail-closed; SharedToken stays
bit-for-bit compatible. 8 new V2G-W2 integration tests pin the
middleware behaviour (viewer/operator routes, missing/wrong token,
JWT fail-closed, Disabled mode, non-admin pass-through, 403 body
token-leakage smoke). T1 / T2 / T7 from §7 of this doc are
materially mitigated (per-identity JWT remains the V2G-W3 step).
Full record: `docs/ADMIN_RBAC_ROUTE_ENFORCEMENT_V2G_W2.md`.

## V2G-W1 progress note (appended 2026-06-01)

The **role model + identity + auth-mode primitives** from §2 and §3
are now implemented in `src/admin.rs` and pinned by 21 unit tests
(`v2gw1_*`). Specifically:

- `AdminRole` enum with `Viewer < Operator < GovernanceAdmin <
  Breakglass` ordering + `implies()` helper.
- `AdminIdentity { name, role }` with `Debug` that redacts the
  principal name.
- `AuthMode { SharedToken, Jwt, Disabled }`; default `SharedToken`
  for backwards compatibility with V2G-W0.
- `required_role_for(method, path)` lookup pinned by tests
  against every route in §1.
- `authenticate(config, header_lookup)` entry point that returns
  an identity under the configured mode.
- `require_role(identity, required)` helper.

JWT mode is intentionally stubbed (`AdminAuthError::JwtNotImplemented`)
— fail-closed. Real JWT verifier + route handler migration are
the V2G-W2 / V2G-W3 milestones. The live backend (V2G-M2 PID
231297) is not affected; existing `ensure_admin_access`
shared-token gate is untouched.

Full record: `docs/ADMIN_JWT_RBAC_IMPLEMENTATION_V2G_W1.md`.

## Constant-time compare finding (C) — CLOSED by V2G-W0

**Original finding (V2G-V close):** `src/admin.rs:50` used
`self.token.as_deref() == Some(candidate)`. `==` on `&str`
short-circuits on the first mismatched byte and is a timing
side-channel vector.

**Status:** **closed in V2G-W0** — see
`docs/ADMIN_TOKEN_CONSTANT_TIME_HARDENING_V2G_W0.md`.

`token_matches` now rejects empty / unset tokens explicitly,
rejects unequal lengths in O(1), and compares equal-length byte
sequences via an in-crate `constant_time_eq(a, b)` XOR/OR fold so
the running time depends only on the input length, not on the
position of the first mismatched byte. Backend test suite passes
**735 / 0 / 0** (V2G-S baseline 724 + V2G-W0 +11). No dependency
changes (the helper is small enough to audit in place; `subtle`
remains a transitive-only dep via `k256`). The patched binary
becomes live at the next planned backend restart.

---

## 8. Allowed Dashboard Actions

| Action | Allowed | Why |
|---|---|---|
| Read status / config / db / metrics / fee summaries / lifecycle / events | ✅ | Core observability use case. |
| Query `/admin/fees/onchain?tx_hash=…` | ✅ | Read-only per-tx fee summary. Replay-safe (V2G-S). |
| Load V2 Fee Observability snapshot | ✅ | Read-only per-tx + global metrics. |
| Load V2 Fee Smoke Readiness (preflight) | ✅ when role ≥ operator | Read-only preflight packet — no signing, no broadcast. |
| Generate RFQ operator packet | ✅ when role ≥ operator | Pure compute — digest + calldata. Does NOT broadcast. (Available after V2G-V impl milestone via the future `Build operator packet` panel.) |
| Display the EIP-712 digest for off-band signing | ✅ when role ≥ operator | Read-only display. The operator copies the digest to a hardware wallet / shell signer. |
| Run `/admin/options/events/tick` | ✅ when role ≥ operator | Mutates backend indexer watermark only — never on-chain. |
| Run `/admin/options/reconciliations/tick` | ✅ when role ≥ operator | Same as above. |
| Sign anything in the browser | ❌ | Hard gate. |
| Broadcast anything from the browser | ❌ | Hard gate. |
| Reveal the admin token / JWT | ❌ | Hard gate. |
| Display any private key, mnemonic, or secret material | ❌ | Hard gate. Per V2G-O / V2G-P0 / V2G-P1. |
| Edit `.env` from the browser | ❌ | Hard gate. |
| Edit a deployed contract field from the browser | ❌ | Hard gate. |
| Pause workers (breakglass) | ❌ from the browser; requires the breakglass quorum flow | Hard gate. |

---

## 9. Test Plan (to add when V2G-V impl ships)

### 9.1 Backend (`deopt-v2-backend`)

| Test | Asserts |
|---|---|
| `admin_token_constant_time_compare` | Replaces `==` with `ct_eq`; pin by attempting a length-mismatched token (must always take ~same time to refuse). |
| `admin_jwt_rejects_missing_aud` | JWT with wrong `aud` rejected. |
| `admin_jwt_rejects_expired_token` | JWT past `exp` rejected. |
| `admin_jwt_rejects_unknown_signer` | JWT signed by a key not in the trusted set rejected. |
| `admin_jwt_role_gate_per_route` | Each route's required role enforced. `viewer` cannot hit POST tick routes. `operator` can. |
| `admin_audit_log_records_every_request` | Capture audit lines; assert one line per request with the schema from §6. |
| `admin_audit_log_redacts_token` | Audit log line contains identity but never the JWT/cookie value. |
| `admin_cors_rejects_unallowed_origin` | Cross-origin POST `/admin/*` rejected with 403. |
| `admin_config_redacts_secrets` (existing) | already green — pin invariant. |

### 9.2 Frontend (`deopt-v2-frontend`)

| Test | Asserts |
|---|---|
| `admin_dashboard_does_not_persist_jwt_to_storage` | sessionStorage / localStorage never contains a JWT. |
| `admin_dashboard_proxies_through_next_api` | All admin fetch calls go through `/api/admin/*`, never directly to the backend origin. |
| `admin_dashboard_xss_smoke` | Render a backend response with `<script>` and HTML tags; assert React escapes it. |
| `admin_middleware_rejects_unauthenticated_request` | Middleware short-circuits `/admin/*` if OIDC session absent. |

---

## 10. Cross-links

- **Canonical V2 fee audit pack**: `docs/DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md` — covers fee math, accounting, monitoring, deployment status, and remaining blockers.
- **Frontend production-readiness panel**: `deopt-v2-frontend/docs/ADMIN_V2_FEE_OBSERVABILITY_UI_V2G_U.md` — the surface this threat model protects.
- **Frontend UI notes for V2G-V**: `deopt-v2-frontend/docs/ADMIN_AUTH_RBAC_UI_NOTES_V2G_V.md` — companion doc for the proxy / middleware / hidden-mutation-button design.
- **OPTION RFQ operator packet** (off-band signing surface): `docs/OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md`.
- **ProtocolFeeVault design** (future treasury surface): `docs/PROTOCOL_FEE_VAULT_DESIGN_V2G_R.md`.
- **Runbook (perp v2 fee alerts)**: `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`.
- **Reconciliation idempotency (V2G-S)**: `docs/FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md` — the replay-safety property the admin endpoints inherit.

---

## 11. Soak Preservation

| Check | State |
|---|---|
| Backend PID 56199 alive | ✅ |
| `/health` | ✅ |
| Prometheus `/-/healthy` | ✅ |
| Compose containers up | ✅ 4/4 |
| Backend restart? | ❌ no |
| `.env` edit? | ❌ no |
| DB writes? | ❌ no |
| Solidity / backend / frontend code changes? | ❌ no — V2G-V is docs-only |

---

## 12. Validation

V2G-V touches only `docs/` (one backend doc + one frontend doc).

| Command | Result |
|---|---|
| `git diff --check` (each repo) | ✅ no whitespace errors |
| Cargo / forge / npm tests | not run — docs-only |

---

## 13. Remaining Blockers

1. **Implementation** of every recommendation in §3.3 (Next.js
   server-side proxy, JWT, role-aware backend gate, audit log,
   constant-time compare, CORS-allowlist tests). This is the V2G-W
   impl milestone.
2. **OIDC / Cloudflare-Access** edge setup for the staging
   dashboard. Ops infrastructure work.
3. **Hardware-key MFA** for governance-admin + breakglass roles.
   Infrastructure work.
4. **CI lint** for SSRF / CORS regressions referenced in T5 / T6.

---

## 14. Next Recommended Milestone

**V2G-W — implement the V2G-V threat model.** Smallest-first
ordering:

1. **Constant-time compare** in `src/admin.rs::token_matches`.
   Self-contained, easy to ship in a docs-light PR. Closes T2 +
   the (C) finding.
2. **Audit-log writer** prelude in `src/api/routes.rs` admin
   handlers (single helper called from each admin handler).
   Closes §6.
3. **Next.js `middleware.ts`** + `/api/admin/*` proxy + drop
   sessionStorage token. Closes T2 + T3 substantially. No backend
   change required for the first iteration.
4. **JWT + per-route role gate** in the backend, dual-path with
   shared token during the cutover. Closes T1 + T2 + T7.
5. **CORS allowlist + integration tests** for T4 + T5.
6. **Edge** (Cloudflare Access + OIDC) deployment for staging.

V2G-W can ship in slices; V2G-V's job is just to pin the design so
the V2G-W PRs have a single reference doc to argue against.
