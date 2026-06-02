# V2G-AUDIT0 — Backend Internal Audit Findings

## Status

- Milestone: **V2G-AUDIT0** — internal audit of the backend's
  admin auth, fee reconciliation, RFQ operator packet, smoke
  readiness, metrics, and config/env hygiene surfaces.
- Date: 2026-06-01.

---

## 1. Summary by severity

| Severity | Count | Blocking pre-mainnet |
|---|---|---|
| Critical | 0 | — |
| High | 2 | both **yes** (mainnet); both close at the next backend restart + V2G-W3 |
| Medium | 3 | recommended close |
| Low | 4 | optional |
| Info | 3 | no |

---

## 2. Findings

### B-H1 — Admin token + V2G-W2 middleware still live on V2G-G era binary; SSR proxy not deployed

- **Severity:** High
- **Status:** open — blocked on the next operator backend restart
- **Component:** running backend (PID 231297, V2G-G era binary).
- **Description:** The constant-time admin token compare (V2G-W0), the RBAC primitives (V2G-W1), the middleware route gate + audit log (V2G-W2), the V2G-S `by_product` / `by_flow` reconciliation buckets, and the V2G-M smoke-readiness endpoint all live in `target/release/deopt-v2-backend` but the running process predates them. The browser still holds the admin token in `sessionStorage` (V2G-V T2/T3).
- **Impact:** Today's `/admin/*` request flow has the V2G-W0 timing fix only in code; the running process still uses `==`. Plus the token is exfiltrable from the browser by an XSS payload.
- **Exploit path:** XSS in the admin dashboard or sessionStorage exfil → admin token → full read access to `/admin/*` + ability to tick mutating POST routes (`/admin/options/events/tick`, `/admin/options/reconciliations/tick`).
- **Evidence:** `ps -p 231297 -o etime` shows the V2G-M2 process; `target/release/deopt-v2-backend` mtime is post-V2G-W2.
- **Recommended fix:** Schedule the next backend restart maintenance window. The restart picks up V2G-W2 middleware (audit log + role gate) and V2G-RX endpoints when implemented. V2G-W3 (Next.js SSR proxy + drop sessionStorage token) closes the browser-side exposure.
- **Blocking:** mainnet — yes. RFQ live + vault cutover — no (those broadcast windows can proceed under V2G-W0 alone).

### B-H2 — Backend has no live audit-log writer for `/admin/*` requests

- **Severity:** High
- **Status:** open — code shipped, not promoted
- **Component:** `src/api/routes.rs::admin_route_gate` (V2G-W2 middleware).
- **Description:** Audit log is emitted via `tracing::info!(target: "deopt.admin.audit", …)` — fine for the local soak. There is no structured retention sink (no JSON file, no DB table, no remote log aggregator) wired to `deopt.admin.audit`. Once the V2G-W2 middleware goes live (next backend restart), the audit lines flow to the standard backend log stream but are not separately retained.
- **Impact:** Forensic audit trail is mixed with general log noise. Hard to query post-incident.
- **Recommended fix:** Add a dedicated log appender (e.g. `tracing_subscriber::Layer` matching `target: "deopt.admin.audit"`) writing to `/var/log/deopt/admin-audit.jsonl` with a retention policy. Spec only — implementation belongs in V2G-W2.1.
- **Blocking:** mainnet — yes. Pre-mainnet ops requires queryable retention.

### B-M1 — V2G-S in-memory dedup key omits `chain_id`

- **Severity:** Medium
- **Status:** accepted (DB layer dedups by `(chain_id, tx_hash, log_index)`; in-memory dedup is defense-in-depth)
- **Component:** `src/fees/onchain_summary.rs::normalize_fee_events`.
- **Description:** The V2G-S dedup key is `(FeeEventModel, tx_hash, log_index, source_contract)`. `chain_id` is on the DB unique index but not on the in-memory key. If a caller mistakenly merges events from two chains in one batch, the in-memory dedup could collapse them.
- **Impact:** Single-chain indexer → zero risk today. Multi-chain support (V2H+) would amplify this. Worth noting now.
- **Recommended fix:** Add `chain_id: u64` to the dedup tuple. Trivial 5-LOC change + 1 new test. Queued as a V2G-S1 hardening item.
- **Blocking:** no.

### B-M2 — RFQ operator-packet `payload_summary` could leak EOAs to logs if a future log writer renders it

- **Severity:** Medium
- **Status:** accepted (current `payload_summary` only emitted via the operator-tool stdout path)
- **Component:** `src/options/rfq_operator_packet.rs::build_option_rfq_operator_packet`.
- **Description:** `payload_summary` includes the buyer + seller EOAs (per design — operator needs to verify they match the V2G-D2 registry). If a future audit-log writer accidentally captures `payload_summary` into a log line, it would leak PII-adjacent EOAs.
- **Impact:** PII-adjacent leak under a future hypothetical change.
- **Recommended fix:** Add a `payload_summary_redacted: bool` field to `OptionRfqOperatorPacket` with helper functions that return either the full or redacted version. Add a unit test asserting `<redacted>` substitution when requested.
- **Blocking:** no.

### B-M3 — `/admin/fees/v2/smoke/readiness` returns the V2G-D2 EOA registry (addresses only) — but the endpoint is role-gated as `operator`, not `viewer`

- **Severity:** Medium
- **Status:** accepted (this is the V2G-V design — operator preflight is the right authority level)
- **Component:** V2G-W1 `required_role_for("GET", "/admin/fees/v2/smoke/readiness")`.
- **Description:** The endpoint returns the Tier 4 / Tier 2 EOA addresses. The V2G-W1 mapping classifies it as `operator` — which is the correct authority because the operator alone can act on a preflight result. A `viewer` should not even see the planned smoke trade.
- **Impact:** None — the role classification is intentional.
- **Recommended fix:** None. Pinning here so external review doesn't re-flag.
- **Blocking:** no.

### B-L1 — Backend env reads via `vm.envOr(...)` swallow misconfigured defaults silently

- **Severity:** Low
- **Status:** accepted
- **Component:** `src/config/env.rs` + various script-side `vm.envOr(...)` calls.
- **Description:** When a required env var is absent or unparseable, the default value silently applies. Some defaults are `false` (safe), some are address(0) (caught later by zero-checks), but the pattern obscures which configuration is loaded.
- **Impact:** Operator may run with unintended defaults.
- **Recommended fix:** Add a startup log that dumps the resolved config (with redacted secrets) at INFO level.
- **Blocking:** no.

### B-L2 — V2G-W2 middleware double-checks auth (defense-in-depth) without telemetry distinguishing middleware vs handler reject

- **Severity:** Low
- **Status:** accepted (intentional)
- **Component:** `src/api/routes.rs::admin_route_gate` + every handler's `ensure_admin_access`.
- **Description:** Both layers refuse with the same error message and the same status code. There's no way to tell from the response whether the middleware or the handler refused.
- **Impact:** Diagnosis-only; not security-relevant.
- **Recommended fix:** None — the dual-check is the V2G-W2 cutover safety net.
- **Blocking:** no.

### B-L3 — Frontend admin token transport is `X-Admin-Token` plaintext header

- **Severity:** Low
- **Status:** accepted (TLS protects in transit; sessionStorage exposure already tracked as B-H1)
- **Component:** `deopt-v2-frontend/src/lib/admin-api.ts:251`.
- **Description:** Token is sent in a plaintext `X-Admin-Token` header. TLS protects in transit; logs / proxies might capture the header value.
- **Recommended fix:** V2G-W3 SSR proxy removes the browser-side token entirely.
- **Blocking:** no.

### B-L4 — V2G-RX `/admin/fees/vault/*` endpoints are spec-only

- **Severity:** Low
- **Status:** open (V2G-RX-backend implementation pending)
- **Component:** `src/api/routes.rs` (spec'd but not implemented).
- **Description:** The V2G-RX observability spec documents two new admin endpoints (`/admin/fees/vault/snapshot`, `/admin/fees/vault/observability`) but they are not implemented in the backend. The Prometheus exporter for the 9 new vault metrics also depends on these endpoints.
- **Impact:** No vault observability post-V2G-R5 cutover until backend is updated + restarted.
- **Recommended fix:** Implement in the next backend PR (V2G-RX-backend). Ship in the same restart window as V2G-W2 pickup.
- **Blocking:** vault cutover (V2G-R5) — strongly recommended to land before broadcast.

### B-I1 — `cargo audit` not run as part of CI

- **Severity:** Info
- **Status:** accepted
- **Component:** `Cargo.toml` + supply chain.
- **Description:** No automated `cargo audit` in CI. Manual review of dep updates.
- **Recommended fix:** Add `cargo audit` to the backend CI pipeline (queued for external audit infra work).
- **Blocking:** no.

### B-I2 — Backend tests cover happy paths but no fault-injection for indexer ↔ DB race

- **Severity:** Info
- **Status:** accepted
- **Component:** `src/options/event_indexer.rs` + DB writers.
- **Description:** The indexer happy path is tested; under-load fault injection (DB transient failures, RPC timeouts) is not.
- **Recommended fix:** Queued for AUDIT-EXT.

### B-I3 — `OPTION_MATCHING_ENGINE_ADDRESS` env name vs. manifest naming

- **Severity:** Info
- **Status:** accepted
- **Component:** `.env.example` vs `deployments/base-sepolia.manifest.draft.json::contracts.OptionMatchingEngine`.
- **Description:** Env key is `OPTION_MATCHING_ENGINE_ADDRESS`; manifest key is `OptionMatchingEngine`. Both refer to the same artifact.
- **Recommended fix:** None — naming is fine.

---

## 3. Auth + RBAC sweep

| Check | Status |
|---|---|
| Constant-time token compare (V2G-W0) | ✅ in `target/`; not live until restart (B-H1) |
| RBAC primitives (V2G-W1) | ✅ in `target/` |
| Middleware route gate (V2G-W2) | ✅ in `target/` |
| Audit log target `deopt.admin.audit` | ✅ in `target/` (no separate retention sink — B-H2) |
| `Display` on errors redacts tokens | ✅ pinned by V2G-W1 tests |
| `Debug` on `AdminConfig` / `AdminIdentity` redacts secrets | ✅ pinned by V2G-W0 / W1 tests |
| `Authorization: Bearer <jwt>` parsing | ❌ V2G-W1 fail-closed stub — JWT verifier is V2G-W3 |
| Frontend SSR proxy | ❌ V2G-W3 (spec only) |

---

## 4. Reconciliation sweep

| Check | Status |
|---|---|
| DB primary dedup `(chain_id, tx_hash, log_index)` | ✅ |
| In-memory dedup `(model, tx_hash, log_index, source_contract)` | ✅ — but `chain_id` omitted (B-M1) |
| V1/V2 mixed event handling (source_priority = "v2") | ✅ pinned by V2G-S tests |
| Replay-once / replay-twice / replay-thrice idempotent | ✅ |
| Overlapping block-range scan safe | ✅ |
| `by_product` / `by_flow` / `rebated_by_product` / `rebated_by_flow` exposed | ✅ (in `target/`; live after restart) |
| Reorg-aware filtering | ⏳ deferred to V2G-T impl milestone |

---

## 5. RFQ operator packet sweep

| Check | Status |
|---|---|
| RFQ typehash matches contract | ✅ pinned by `option_rfq_trade_typehash_matches_contract` |
| RFQ digest ≠ ORDERBOOK digest for same payload | ✅ pinned |
| Selector matches `executeRfqTrade` | ✅ pinned |
| Cross-flow replay impossible | ✅ pinned |
| `require_broadcast_confirm` accepts only literal `"true"` | ✅ pinned (4 negative + 1 positive) |
| `payload_summary` has no private-key / signature material | ✅ pinned |
| Buyer / seller addresses in summary | ✅ — see B-M2 redaction recommendation |

---

## 6. Metrics + observability sweep

| Check | Status |
|---|---|
| `old`/`unknown` consumer counter labels classified | ✅ V2G-G |
| `FeeOldConsumer` / `FeeUnknownConsumer` alerts present | ✅ V2G-G |
| Rebate budget gauge per asset | ✅ V2G-G |
| Vault observability metrics | ⏳ V2G-RX backend impl pending (B-L4) |
| FeeVaultDrift / ReserveShortfall / RebatesPaused / HookFailure / BootstrapMissing alerts | ⏳ V2G-RX observability spec; not deployed |

---

## 7. Config / env hygiene sweep

| Check | Status |
|---|---|
| `OLD_PERP_ENGINE` not configured as active | ✅ env keeps it as observability-only |
| `admin_config_redacts_secrets` test green | ✅ |
| Env files in `.gitignore` | ✅ checked |
| `cargo audit` in CI | ❌ B-I1 |

---

## 8. Implementation status of small safe fixes

V2G-AUDIT0 implements no source-level fixes in this run. The
findings above are all either:

- Pending operator-side action (backend restart for B-H1),
- Spec-level for a follow-up milestone (V2G-RX-backend for B-L4, V2G-S1 for B-M1, V2G-W2.1 for B-H2),
- Accepted by design (B-M3, B-L1, B-L2, B-L3, B-I1, B-I2, B-I3).

---

## 9. Cross-links

- Threat model: `ADMIN_AUTH_RBAC_THREAT_MODEL_V2G_V.md`
- W2 route gate: `ADMIN_RBAC_ROUTE_ENFORCEMENT_V2G_W2.md`
- V2G-S reconciliation: `FEE_RECONCILIATION_IDEMPOTENCY_V2G_S.md`
- V2G-RX observability spec: `PROTOCOL_FEE_VAULT_OBSERVABILITY_SPEC_V2G_RX.md`
- Audit gate decision: `~/DEOPT/AUDIT_GATE_DECISION_V2G_AUDIT0.md`
