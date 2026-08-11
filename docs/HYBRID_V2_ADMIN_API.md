# Hybrid V2 Admin API

Reference for the admin routes served by `src/api/hybrid_v2_admin.rs`
and `src/api/hybrid_v2_execution_admin.rs`. All routes are behind the
`x-admin-token` header gate and refuse Base mainnet (chain id 8453)
at handler entry.

## Auth

Every route calls `ensure_admin(state, headers)` before doing any
work:

- `state.admin_config.enabled == false` → `403 ADMIN_DISABLED`.
- `state.admin_config.require_token == true` and the `x-admin-token`
  header is missing → `403 ADMIN_TOKEN_REQUIRED`.
- Token mismatch (constant-time compare) → `403 INVALID_ADMIN_TOKEN`.

## Mainnet refusal

Every deployment-scoped route calls `refuse_mainnet(chain_id)` right
after resolving the deployment; a chain id of 8453 returns
`403 BASE_MAINNET_FORBIDDEN`.

## Rebuild + reconciliation routes

See `docs/HYBRID_V2_OPERATOR_RUNBOOK.md § 6` and `§ 7` — this
document covers the execution admin routes.

## Execution admin routes

### `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/prepare`

Prepare (and land at the terminal `BROADCAST_DISABLED` phase) an
execution row for the given canonical id.

**Request body** — `#[serde(deny_unknown_fields)]`. Any unknown
field returns `400 INVALID_PREPARE_BODY`. Every downstream execution
field (target, selector, calldata, value_wei, nonce, gas_limit,
max_fee_per_gas, max_priority_fee_per_gas, chain_id) is derived
deterministically from the manifest allowlist + plan builder — the
body cannot influence any of them.

```json
{
  "buyer_envelope": {
    "owner": "0x…",
    "subaccount_id": 1,
    "subkey": "0x…",
    "signer": "0x…",
    "engine": "0x…",
    "action": "0x…",
    "architecture_version": "1",
    "nonce": "1",
    "deadline": "2000000000",
    "owner_recovery_epoch": "0",
    "subaccount_recovery_epoch": "0",
    "payload_hash": "0x…",
    "signature": "0x…"
  },
  "buyer_order": {
    "series_id": "42",
    "side": 0,
    "quantity_1e8": "100000000",
    "price_per_contract_1e8": "50000000",
    "limit_price_per_contract_1e8": "60000000",
    "premium_token": "0x…",
    "time_in_force": 0,
    "role": 0,
    "max_positive_fee_ppm": 100,
    "salt": "0x…"
  },
  "seller_envelope": { … },
  "seller_order":    { … },
  "fill_quantity_1e8": "100000000",
  "buyer_active_series":  ["42"],
  "seller_active_series": ["42"],
  "buyer_order_hash":  "0x…",
  "seller_order_hash": "0x…",
  "series_id":       "42",
  "premium_amount":  "50000000",
  "fee_schedule_epoch": null
}
```

**Responses:**

- `200 OK` — terminal outcome. Body:

  ```json
  {
    "canonical_execution_id": "0x…",
    "terminal_phase": "BROADCAST_DISABLED",
    "failure_class": null,
    "failure_detail": null,
    "plan_hash": "0x…",
    "signing_payload_hash": "0x…",
    "simulation_gas_estimate": 90000,
    "reserved_nonce": 7,
    "attempts": 1
  }
  ```

- `400 INVALID_CANONICAL_ID` — `canonical_execution_id` path
  parameter is not a 0x-prefixed 32-byte hex string.
- `400 INVALID_PREPARE_BODY` — body validation failed.
- `403 ADMIN_DISABLED` / `ADMIN_TOKEN_REQUIRED` / `INVALID_ADMIN_TOKEN`.
- `403 BASE_MAINNET_FORBIDDEN` — deployment is on chain 8453.
- `409 EXECUTION_LOCK_CONTENTION` — a concurrent request holds the
  deployment-scoped operation lock.
- `500 STORE_ERROR` / `ORCHESTRATION_UNRECOVERABLE`.
- `503 EXECUTION_ORCHESTRATOR_NOT_WIRED` — no live orchestrator
  wired into this `AppState` build. Body includes an availability
  reason:

  ```json
  {
    "error": "EXECUTION_ORCHESTRATOR_NOT_WIRED",
    "detail": "no execution orchestrator wired to this AppState build",
    "availability": {
      "state": "NotConfigured",
      "reason": "EXECUTION_DISABLED"
    }
  }
  ```

  Common reasons:

  - `EXECUTION_DISABLED` — `HV2_EXECUTION_ENABLED` is false.
  - `IncompleteProductionSignerConfig: …` — one of the required
    `HV2_SIGNER_*` env vars is unset.
  - `aws-kms-transport feature not enabled at build time` — build
    was compiled without `--features aws-kms-transport`.

### `GET /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id`

Fetch the sanitized execution row. Redacted so operators can inspect
outcome + phase without leaking `(r, s, v)`. The `recovered_signer`
public address is included because it is not a secret.

Responses: `200 OK` with a `SanitizedExecutionRow` body, `404
UNKNOWN_EXECUTION` if the id is not present, or the standard admin/
mainnet refusals above.

### `GET /admin/hybrid_v2/deployments/:deployment_id/executions?limit=N&offset=M`

Bounded listing. `limit ≤ 1000`, `offset ≥ 0`. Responses: `200 OK`
with a `Vec<SanitizedExecutionRow>` body.

### `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/cancel`

Cancel the row. Refused past `AWAITING_SIGNATURE`.

### `POST /admin/hybrid_v2/deployments/:deployment_id/executions/:canonical_execution_id/retry`

Currently returns `409 RETRY_MUST_ISSUE_NEW_CANONICAL_ID`. Terminal
FAILED rows do not resurrect — the operator re-issues `prepare` with
the original intent, which derives the same canonical id and
converges on the same row.

## Sanitized execution row schema

```json
{
  "canonical_execution_id": "0x…",
  "deployment_id": 12,
  "chain_id": 84532,
  "phase": "BROADCAST_DISABLED",
  "attempts": 1,
  "created_at_ms": 1700000000000,
  "updated_at_ms": 1700000000200,
  "failure_class": null,
  "failure_detail": null,
  "plan_hash": "0x…",
  "calldata_hash": "0x…",
  "signing_payload_hash": "0x…",
  "target": "0x…",
  "selector": "0x1a2b3c4d",
  "value_wei": "0",
  "gas_limit": 108000,
  "max_fee_per_gas_wei": "…",
  "max_priority_fee_per_gas_wei": "…",
  "reserved_nonce": 7,
  "recovered_signer": "0x…",
  "signer_request_idempotency_key": "0x…"
}
```

Excluded (by construction — the sanitizer strips these before
serialization): `signature_r`, `signature_s`, `signature_v`, any
signer-side secret material, raw request body.

## External signer failure classes

| `failure_class` | Meaning | Retryable? |
|---|---|---|
| `PREFLIGHT_REJECTED` | Readiness / drift / cancelled / expired | No — fix upstream |
| `PLAN_BUILD_FAILED` | Manifest mismatch, bad address, wrong chain | No — config regression |
| `NONCE_RESERVATION_FAILED` | RPC + persistence disagreement | Usually yes — new `prepare` |
| `SIMULATION_FAILED_DETERMINISTIC` | On-chain revert with decoded selector | No — fix state |
| `SIMULATION_TRANSPORT_FAILED` | RPC transport blip | Yes — new `prepare` |
| `GAS_POLICY_REJECTED` | Estimate / fee / total cost out of bounds | Wait for gas normal |
| `FIREWALL_REJECTED` | Row tampered / plan mutated | Escalate |
| `SIGNER_UNAVAILABLE` | Signer timeout / 5xx / auth failed / deterministic vendor refusal | Depends on availability sub-reason |
| `SIGNATURE_VERIFICATION_FAILED` | Signer returned `(r,s,v)` that does not recover to `expected_signer_address` over the orchestrator-derived payload | Escalate — signer compromise or bug |
| `LOCK_CONTENTION` | Another op holds the deployment lock | Yes — retry later |
| `STORE_FAILURE` | Postgres error | Check DB health |

## Broadcast routes — 2026-08-11

Added by `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` Package D.
Every route below is gated by `AdminConfig::require_token`, refuses
Base mainnet at handler entry, and returns a structured 503 with the
`hybrid_v2_broadcast_unavailable_reason` when broadcast is disabled
or not wired.

### `POST /admin/hybrid_v2/deployments/:id/executions/:cid/broadcast`

Drives `BroadcastOutbox::resume(cid)`. Body: `{}` (deny_unknown_fields
— any extra field returns 400). Response body includes the current
`phase`, `tx_hash`, `provider_classification`, `failure_class`,
`failure_detail`, and a `note` documenting that first-submission via
plan+signed reconstruction is deferred to
`BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1`.

### `GET /admin/hybrid_v2/deployments/:id/executions/:cid/broadcast_status`

Returns a `SanitizedBroadcastRow`: canonical id, phase, tx hash,
envelope hash, envelope bytes hash, submission attempt count,
first / last submission timestamps, provider classification, receipt
fields, gas fields, confirmation count, canonicality state, reorg
count, failure class + detail, terminal timestamp. Excludes
signature bytes, raw envelope bytes, provider connection details.

### `GET /admin/hybrid_v2/deployments/:id/broadcast_pending?limit=N`

Lists rows in observable phases (Submitted, Pending,
SubmissionUnknown, MinedSuccess, Confirming, Reorged). Returns a
`SanitizedBroadcastRow` array. `limit` bounded to `[1, 500]`.

### `POST /admin/hybrid_v2/deployments/:id/executions/:cid/broadcast_recheck`

Drives `BroadcastConfirmationWorker::tick_single(cid)`. Body: `{}`.
Response returns the new `phase`. Never invokes any signer function
and never sends a raw transaction; only receipt / tx-by-hash /
block header lookups.

### `POST /admin/hybrid_v2/deployments/:id/executions/:cid/broadcast_resend_same_bytes`

Currently returns 503 with the wired-broadcast state — an honest
deferral pending the plan hydrator scheduled for
`BACKEND-HYBRID-V2-BASE-SEPOLIA-EXECUTION-E2E-V1`. The
`BroadcastOutbox::resend_same_bytes` API itself is complete +
tested; only the admin-route reconstruction step is deferred.

Body: `{}`. Guard responses: 404 when no broadcast row exists; 409
`RESEND_WRONG_PHASE` when the row is not `SubmissionUnknown` /
`Dropped`; 409 `RESEND_BUDGET_EXHAUSTED` when
`submission_attempt_count > submission_retry_max`.

### `POST /admin/hybrid_v2/deployments/:id/executions/:cid/broadcast_manual_intervention`

Operator escalation to `MANUAL_INTERVENTION_REQUIRED`. Body:
`{ "action": "MARK_MANUAL", "detail": "<free-form operator note>" }`
(deny_unknown_fields). Only `MARK_MANUAL` is accepted.

## Broadcast failure classes

| `failure_class` | Meaning | Retryable? |
|---|---|---|
| `PROVIDER_HASH_MISMATCH` | Provider `Accepted` with divergent tx hash | No — critical, investigate provider |
| `NONCE_CONFLICT_NONCE_TOO_LOW` | On-chain nonce past the reserved value | No — inspect signer nonce |
| `NONCE_CONFLICT_NONCE_TOO_HIGH` | Reserved nonce is future | No — wait / re-plan |
| `NONCE_CONFLICT_REPLACEMENT_UNDERPRICED` | Higher-fee pending tx already occupies the slot | No — investigate |
| `NONCE_CONFLICT_OUR_TX_*` | Investigator classified the situation | Case-by-case |
| `PROVIDER_REJECTED` | Provider JSON-RPC error | Case-by-case |
| `TRANSPORT_AMBIGUOUS` | Timeout / Transport / Unavailable | Yes — worker resume path |
| `SERIALIZATION_FAILED` | Local envelope build error | No — data bug |
| `FIREWALL_REJECTED` | Firewall revalidation rejected | Escalate |
| `RECEIPT_HASH_MISMATCH` | Receipt tx hash disagrees with envelope hash | No — critical |
| `CORRELATION_MISSING` | Depth threshold reached but no matched indexer row | Investigate indexer |
| `ADMIN_MANUAL_INTERVENTION` | Operator-issued escalation | Manual |

## Cross-references

- Runbook Section 17: `docs/HYBRID_V2_OPERATOR_RUNBOOK.md § 17`.
- Runbook Section 19 (broadcast operations):
  `docs/HYBRID_V2_OPERATOR_RUNBOOK.md § 19`.
- V1 closure:
  `docs/BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1.md`.
- External signer closure:
  `docs/BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1.md`.
- Broadcast closure:
  `docs/BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1.md`.
- Broadcast security review:
  `docs/BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1_SECURITY_REVIEW.md`.
