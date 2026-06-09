# Mainnet backend executor — signer service design (V1)

**Posture:** DESIGN / DOC ONLY. No source code modified. No `.env` edited. No
chain transactions. No KMS keys created. No vendor accounts created.
**Closes milestone:** `MAINNET-BE-SIGNER-SERVICE-DESIGN`.
**Sources of truth (re-anchored, not duplicated):**
- `MAINNET_CUSTODY_POLICY.md §6 + §7` — principle BE-5 (no raw key in backend
  memory), §6.6 transaction policy precheck, §6.7 audit log fields, §7.4
  `RemoteSigner` trait pattern.
- `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §1 + §5` — Pattern C
  decided; §5.1 backend code touch points; §5.2 signer microservice
  responsibilities; §6.1 milestone naming.
- `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md` + `SHOULD_BROADCAST_DESIGN_NOTE.md`
  — `should_broadcast` decision point + reject codes + `econ_data_available`.
- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia cutover precedent.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2.1 + §7.1` — KMS audit
  correlation (`kms_request_id`) + signer-level metric label policy.
- `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md §1.2` — Track 2 acceptance
  criteria + auditor anchor.

**Hard rule for this design:** the signer service is a **request validator
and signature emitter**. It does NOT decide business economics. The backend
runs `should_broadcast` (gap-list C-4, shipped) and passes an **approval
fingerprint** to the signer; the signer enforces a cryptographic +
transaction-allowlist policy and emits a signature. The chain is the final
backstop (`R-6` accounting bright line; FeesManagerV2 + PFV invariants).

---

## 1. Scope

### 1.1 In scope (launch)

- Sign EIP-1559 transactions for the option execution path on Base mainnet
  (`chain_id = 8453`).
- Two function selectors only: `executeTrade` (orderbook) and
  `executeRfqTrade` (RFQ).
- Single target contract: `NEW_OME` (the canonical OptionMatchingEngine
  address resolved at deploy and pinned in the signer's allowlist).
- Single signer identity: `BACKEND_EXECUTOR_OPTION` (per Cluster 2 §2.1
  — distinct EOA from any future `BACKEND_EXECUTOR_PERP`).

### 1.2 Out of scope at launch (explicit)

- Perp real-broadcast (per Cluster 2 §2.3; perp scaffold hard-stop at
  `src/execution/executor.rs:54-58` remains in force).
- Governance / Timelock / Safe transactions (signed by hardware wallets per
  custody policy §3 + §5, not by this service).
- ERC-20 approvals / transfers from BE.
- Arbitrary contract calls (no `/sign?msg=…` or `/sign_raw_tx` surface).
- Liquidation flow (`should_broadcast` step 8 carve-out is
  `liquidation-out-of-scope` until Q-CD-12 numeric fill).
- ETH-bearing calls (signer rejects any `value > 0`).

### 1.3 Backend / signer / KMS split

```
┌────────────────────────────────────────────────────────────────────────────┐
│                     deopt-v2-backend  (this repo)                          │
│                                                                            │
│   options/service.rs::broadcast_option_execution_intent_with_provider      │
│        │                                                                   │
│        ▼                                                                   │
│   options/broadcast_policy.rs::should_broadcast(intent, ctx)               │
│        │  → Approve(reason)                                                │
│        ▼                                                                   │
│   execution/remote_signer.rs::SignerClient::sign_option_execution_tx       │
│        │  HTTPS / mTLS                                                     │
└────────┼───────────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                  signer microservice  (separate repo / crate)              │
│                                                                            │
│  inbound: mTLS server                                                      │
│        │                                                                   │
│        ▼                                                                   │
│  §3 request schema validation  (this doc §3)                               │
│        │                                                                   │
│        ▼                                                                   │
│  §4 transaction policy precheck (this doc §4 + custody §6.6)               │
│        │                                                                   │
│        ▼                                                                   │
│  §5 signer-policy ↔ should_broadcast fingerprint bind                      │
│        │                                                                   │
│        ▼                                                                   │
│  §6 KMS / HSM / MPC adapter  (vendor-neutral trait)                        │
│        │                                                                   │
│        ▼                                                                   │
│  signature response (§3.2) + structured audit log (§7)                     │
└────────┼───────────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────────────────────┐
│            KMS / HSM / MPC provider  (Q-CD-5 vendor PENDING)               │
│                                                                            │
│  - non-extractable secp256k1 key (BACKEND_EXECUTOR_OPTION)                 │
│  - IAM-gated `sign_digest` only                                            │
│  - kms_request_id propagated to audit log                                  │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Current signer state (Phase A finding)

| Aspect                  | Sepolia today                                     | Mainnet target                                       |
| ----------------------- | ------------------------------------------------- | ---------------------------------------------------- |
| Key location            | `EXECUTOR_PRIVATE_KEY` in `.env`                  | Non-extractable KMS / HSM key                        |
| Signer call site        | `ExecutorSigner::from_private_key(secret)` (`src/execution/signer.rs:26`) + `sign_prehash` (`:46`) | `RemoteSignerClient::sign_option_execution_tx(...)` (new) |
| Process memory          | Raw 32-byte `k256::ecdsa::SigningKey`             | Bytes never enter backend process                    |
| Allowlist enforcement   | Backend-side gas safety check + new `should_broadcast` policy gate | Backend `should_broadcast` (decision) + signer §4 (cryptographic + tx-allowlist) |
| Mainnet startup         | Currently would accept `EXECUTOR_PRIVATE_KEY` on chain id 8453 → **forbidden** by custody policy §6.1 BE-5 + §7.4 | Backend MUST REFUSE start when `chain_id == 8453 AND executor_private_key.is_some()` |
| Auditor anchor          | n/a                                               | Q-26 (key non-extractable from KMS), Q-27 (§6.6 policy layer), Q-29 (mTLS authn) |

**Current gap (re-confirmed during Phase A):**
- Raw key in process memory (custody policy §6.1 BE-5 violation if used on
  mainnet).
- No `RemoteSigner` trait — call sites at `src/options/service.rs:1166` and
  `:1213` directly construct `ExecutorSigner`.
- No `BACKEND_SIGNER_ENDPOINT` env key.
- No mainnet startup refusal of `EXECUTOR_PRIVATE_KEY`.
- No `kms_request_id` field plumbed through backend logs (planned per
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §7.1`).

---

## 3. Request / response schema

### 3.1 Request

```json
{
  "request_id":                "uuid v4 (signer-generated correlation id)",
  "intent_id":                 "uuid v4 (option_execution_intents.intent_id)",
  "source_type":               "option_orderbook_fill | option_rfq_fill",
  "chain_id":                  8453,
  "operator_environment":      "mainnet | sepolia | localdev",
  "target_contract":           "0x… (must match signer's allowed_target)",
  "function_selector":         "0x4-byte (executeTrade | executeRfqTrade)",
  "calldata_hash":             "0x32-byte keccak256 of the full calldata bytes",
  "calldata_length":           "decimal",
  "transaction_to":            "0x… (== target_contract)",
  "transaction_value":         "0   (signer rejects > 0)",
  "gas_limit":                 "decimal (≤ signer's gas_limit_cap)",
  "max_fee_per_gas":           "decimal wei (EIP-1559)",
  "max_priority_fee_per_gas":  "decimal wei (EIP-1559)",
  "nonce":                     "decimal (== backend's expected nonce; signer cross-checks)",
  "simulation_block":          "decimal (block number simulation ran against)",
  "simulation_hash":           "0x32-byte (block hash at simulation_block; freshness anchor)",
  "deadline":                  "decimal seconds since epoch (must be in future at sign time)",
  "policy_decision_id":        "uuid v4 (should_broadcast decision row id)",
  "policy_fingerprint":        "0x32-byte keccak256( policy_decision_id || calldata_hash || nonce || simulation_block || deadline )",
  "policy_decision_at_ms":     "decimal (UTC ms when should_broadcast returned Approve)",
  "caller_identity":           "service IAM role / mTLS SPIFFE ID (signer-verified, not body)"
}
```

Notes:
- `request_id` is fresh per call (no replay).
- `calldata_hash` binds the call to the exact bytes; mismatch on signer
  recomputation = `calldata-bind-mismatch` reject.
- `policy_fingerprint` is the cryptographic binding between
  `should_broadcast`'s Approve decision and the request the signer is asked
  to sign. **Stale policy + tampered calldata both produce the same
  fingerprint mismatch.**
- `caller_identity` is **not** read from the request body. The signer takes
  the authenticated identity from the mTLS / IAM layer.
- Request size MUST be capped (e.g. 16 KiB) to bound DoS.

### 3.2 Response (Approve)

```json
{
  "request_id":           "uuid v4 (echo)",
  "signer_address":       "0x… (recovered post-sign; must equal allowed BE EOA)",
  "signature": {
    "y_parity":           0 | 1,
    "r":                  "0x32-byte",
    "s":                  "0x32-byte"
  },
  "signature_type":       "eip1559_recoverable_secp256k1",
  "signed_tx_hash":       "0x32-byte (keccak256 of RLP-encoded signed tx)",
  "kms_key_ref":          "<redacted>  // e.g. 'option-be-2026q3:v1' — opaque label, never the ARN/secret",
  "kms_request_id":       "<provider-issued audit correlation id>",
  "audit_log_id":         "<signer-side audit log row id>",
  "created_at_ms":        "decimal (UTC ms when signature was emitted)",
  "policy_decision_id":   "uuid v4 (echo; correlation)"
}
```

### 3.3 Response (Reject)

```json
{
  "request_id":           "uuid v4 (echo)",
  "denial_code":          "<one of the codes in §4.2>",
  "denial_detail":        "<non-sensitive string; no secrets>",
  "audit_log_id":         "<signer-side audit log row id>",
  "rejected_at_ms":       "decimal (UTC ms)",
  "policy_decision_id":   "uuid v4 (echo, if present)"
}
```

### 3.4 What MUST NOT appear in any request or response

- Private keys (KMS or otherwise).
- Seed phrases / mnemonics.
- KMS provider IAM credentials.
- DATABASE_URL.
- Backend admin tokens.
- RPC provider API keys.
- Provider account identifiers beyond an opaque, rotation-stable label.
- Personal contact details (signer operators).

---

## 4. Signer service policy (transaction allowlist + audit logging)

This section codifies `MAINNET_CUSTODY_POLICY.md §6.6 + §6.7` at the
signer service boundary. The §6.6 layer is the chain-side backstop the
auditor inspects (Q-27).

### 4.1 Allow conditions (ALL must hold)

| #   | Check                                                                             |
| --- | --------------------------------------------------------------------------------- |
| A1  | `chain_id ∈ allowed_chain_ids` (mainnet build: `{8453}` only).                    |
| A2  | `target_contract ∈ allowed_targets` (mainnet: `{NEW_OME}`).                       |
| A3  | `function_selector ∈ allowed_selectors` (`{executeTrade, executeRfqTrade}`).      |
| A4  | `transaction_value == 0`.                                                         |
| A5  | `gas_limit ≤ gas_limit_cap` (per-build configured; mirrors `OPTION_EXECUTION_BROADCAST_GAS_LIMIT`). |
| A6  | recomputed `keccak256(calldata) == calldata_hash`.                                |
| A7  | `nonce` matches expected (signer reads BE on-chain nonce + pending count).        |
| A8  | `deadline > now`.                                                                 |
| A9  | `policy_fingerprint` recomputed from request fields equals the supplied value.    |
| A10 | `policy_decision_at_ms` ≥ `now - STALE_POLICY_MAX_AGE_MS` (see §5.2).             |
| A11 | `request_id` not seen before (in-memory + persisted dedupe; TTL ≥ deadline + 5m). |
| A12 | `intent_id` has not produced a successful `signed_tx_hash` already.               |
| A13 | `caller_identity ∈ allowed_callers` (backend service IAM/SPIFFE).                 |
| A14 | rate limit not exceeded (per minute / per hour / per intent_id).                  |
| A15 | post-sign: signature recovers to BE EOA exactly (P-3 sanity).                     |

### 4.2 Hard reject codes

Stable strings consumed by audit logs, metrics, and operator dashboards.

```
chain-not-allowed         A1 fail
target-not-allowed        A2 fail
selector-not-allowed      A3 fail
value-not-zero            A4 fail
gas-cap                   A5 fail
calldata-bind-mismatch    A6 fail
nonce-mismatch            A7 fail
deadline-expired          A8 fail
policy-fingerprint        A9 fail (calldata changed after policy approved)
policy-stale              A10 fail (must re-run should_broadcast)
duplicate-request-id      A11 fail
duplicate-intent-signed   A12 fail (idempotency: refuse second sign for same intent_id)
caller-unauthorized       A13 fail
rate-limit                A14 fail
post-sign-from-mismatch   A15 fail (alerting condition: KMS key vs expected EOA disagreement)
kms-unavailable           upstream KMS error
kms-timeout               upstream KMS timeout
internal                  signer-side bug; log + page
```

A reject from A14/A15 emits a **critical** alert per
`MAINNET_CUSTODY_POLICY.md §10.1` + `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3.1`.

### 4.3 Audit log fields (per request, success OR reject)

Lifted from custody policy §6.7 + extended for the §3 schema:

```yaml
event: signer_sign_request
timestamp_utc: <iso8601>
audit_log_id: <signer-issued uuid>
request_id: <uuid>
intent_id: <uuid>
source_type: option_orderbook_fill | option_rfq_fill
chain_id: 8453
target_contract: <0x…>
function_selector: <0x4-byte>
calldata_hash: <0x32-byte>
nonce: <decimal>
gas_limit: <decimal>
value_wei: 0
policy_decision_id: <uuid or null>
policy_fingerprint: <0x32-byte>
caller_identity: <iam/spiffe id>
result: signed | rejected
denial_code: <enum from §4.2 if rejected>
signer_address: <0x… if signed>
kms_request_id: <provider audit id if KMS was reached>
kms_key_ref: <opaque label>
latency_ms: <decimal>
secret_in_log: false
```

### 4.4 Redaction rules (binding)

The audit log, response bodies, metrics, traces, and error messages MUST
NOT contain:

- private keys (KMS or otherwise)
- seed phrases / mnemonics
- provider account ARNs that resolve to a key material location
- DATABASE_URL / backend admin tokens / RPC API keys
- raw signed transaction bytes (use `signed_tx_hash`; the raw RLP can be
  reconstructed by the backend from the public response + calldata)
- caller body fields besides those listed in §3.1 (no logging of full request bodies)

The signer's structured logger MUST set `secret_in_log: false` only after a
sanitizer pass that strips any key-like 0x40-hex / 0x64-hex / base64 blob
matching candidate-secret heuristics.

---

## 5. `should_broadcast` ↔ signer-service integration

### 5.1 Decision-fingerprint binding

`should_broadcast` returns `ShouldBroadcastDecision::Approve(reason)` per
`broadcast_policy.rs`. The backend, on Approve:

1. Persists a `broadcast_policy_decisions` row keyed by a freshly-generated
   `policy_decision_id` (UUID v4). Row contains: the full reason,
   `econ_data_available` flag, mode (Mainnet | Sepolia), the snapshot of
   relevant context fields (chain_id, target, selector, gas_limit,
   calldata_hash, nonce, deadline, simulation_block, simulation_hash,
   intent_id, source_type, asset).
2. Computes the **policy fingerprint:**
   ```
   keccak256( policy_decision_id ‖ calldata_hash ‖ nonce ‖ simulation_block ‖ deadline )
   ```
3. Passes both `policy_decision_id` and `policy_fingerprint` to the signer
   request.

The signer recomputes the same keccak from the request fields and rejects
on mismatch (`policy-fingerprint`). This binds the signed transaction to
the exact policy decision, the exact calldata, and a freshness anchor.

### 5.2 Stale-policy window

| Knob                       | Recommended default | Reason                                                              |
| -------------------------- | ------------------- | ------------------------------------------------------------------- |
| `STALE_POLICY_MAX_AGE_MS`  | 30_000 ms           | RM snapshot + nonce + sim freshness window from custody policy §6.5 |
| `RM_SNAPSHOT_MAX_AGE_MS`   | 5_000 ms (already in policy ctx) | already enforced by should_broadcast `stale-rm`        |
| `DEADLINE_MIN_HEADROOM_MS` | 2_000 ms            | min slack to allow tx to land before chain-side deadline check       |

If the signer receives a request older than `STALE_POLICY_MAX_AGE_MS`, it
rejects `policy-stale`. The backend MUST re-run `should_broadcast` and
issue a fresh decision_id + fingerprint.

### 5.3 Fail-closed default

| Scenario                                                                | Outcome                                                                        |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `should_broadcast` returned Reject                                      | Backend does NOT contact signer.                                               |
| `should_broadcast` returned Approve but `policy_decision_id` missing    | Signer rejects (`policy-fingerprint`).                                         |
| Signer reachable but KMS upstream timeout                               | Signer responds 5xx; backend transitions intent to `broadcast_failed` with `signer-kms-timeout`. NO local-fallback path. |
| Mainnet env contains `EXECUTOR_PRIVATE_KEY`                            | Backend REFUSES TO START at startup (custody §6 BE-5 + §7.4 inversion).        |
| Sepolia env contains `EXECUTOR_PRIVATE_KEY` AND `BACKEND_SIGNER_ENDPOINT` | Backend prefers `BACKEND_SIGNER_ENDPOINT` and logs a warn-once that the env key is ignored. |
| Local dev (`operator_environment = localdev`)                           | `LocalDevSigner` allowed iff `chain_id ∈ {31337, 84532}` AND an explicit `EXECUTOR_ALLOW_LOCAL_SIGNER=true` flag is set. |

The optional **testnet-only local signer** path is gated behind a single
explicit env flag (`EXECUTOR_ALLOW_LOCAL_SIGNER`) so that any accidental
flip on mainnet still fails-closed on the chain-id check at startup.

### 5.4 Mainnet-specific startup invariant

On startup, backend asserts the conjunction `chain_id == 8453 ⇒
executor_private_key.is_none() AND backend_signer_endpoint.is_some()`.
Violation → exit non-zero with a structured error; no secret printed.

---

## 6. KMS / HSM / MPC provider abstraction

### 6.1 Internal signer trait surface

```rust
// src/execution/remote_signer.rs  (Phase 1 PR, separate milestone)

#[async_trait::async_trait]
pub trait RemoteSigner: Send + Sync {
    /// Sign an EIP-1559 transaction prehash. The implementation MUST
    /// uphold the §4 transaction allowlist and §5 policy-fingerprint
    /// bind before invoking KMS.
    async fn sign_option_execution_tx(
        &self,
        request: SignerRequest<'_>,
    ) -> Result<SignerResponse, SignerError>;

    /// Stable address derivation; idempotent. Used by startup guard to
    /// cross-check the recovered BE EOA against the configured allowlist.
    async fn derive_executor_address(&self) -> Result<AccountId, SignerError>;

    /// Liveness + identity check; no signature emitted.
    async fn health_check(&self) -> Result<HealthReport, SignerError>;

    /// Two-phase rotation (design surface; provider-specific impl).
    async fn rotate_key_prepare(&self) -> Result<RotationHandle, SignerError>;

    /// Disable a key handle (emergency revoke). Provider-specific impl.
    async fn disable_key(&self, handle: KeyHandle) -> Result<(), SignerError>;
}
```

`SignerRequest` and `SignerResponse` mirror the §3 schemas.
`SignerError` carries the `denial_code` enum + an opaque cause string.

### 6.2 Three concrete implementations envisaged

| Impl                | Backend behaviour                                                                                                | Allowed in mainnet? |
| ------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------- |
| `LocalDevSigner`    | Wraps existing `ExecutorSigner::from_private_key`. Used for unit/integration tests + Sepolia rehearsal harness.  | NO. Enabled only when `EXECUTOR_ALLOW_LOCAL_SIGNER=true` AND `chain_id ∈ {31337, 84532}`. |
| `RemoteSignerClient` | mTLS client of the signer microservice (per §1.3 topology). The default mainnet impl.                            | YES.                |
| `KmsHsmMpcSigner`   | In-process embedding of the §6.3 provider-neutral trait for environments where the microservice and adapter live in the same binary (allowed but discouraged; trades operational simplicity for blast-radius). | Allowed but recommends-against in production posture. |

### 6.3 Provider-neutral KMS adapter surface

The signer microservice (or `KmsHsmMpcSigner`) consults a provider-neutral
adapter trait:

```text
trait KmsHsmMpcBackend {
    sign_digest(key_ref, digest_32) -> RecoverableSig    // secp256k1, low-s, deterministic-or-RFC6979
    derive_address(key_ref)         -> 20-byte EVM addr  // idempotent, stable across calls
    health_check()                  -> HealthReport      // includes key alive/disabled state
    rotate_key_prepare(seed_label)  -> KeyHandle         // produces a NEW key; old still valid
    disable_key(key_ref)            -> ()                // IAM-gated; emits provider audit event
}
```

Required properties (independent of vendor; lifted from custody §7.3):

```
[ ] secp256k1 native (no transit-of-raw-key wrappers)
[ ] non-extractable: signing only; no `export-key` API exposed
[ ] deterministic address derivation across calls
[ ] per-sign provider-issued audit id retrievable as `kms_request_id`
[ ] IAM minimisation: signer service identity granted `sign_digest` on a single key handle, nothing else
[ ] rate limit per identity at the provider side
[ ] explicit `disable_key` distinct from delete (custody §4)
[ ] scheduled-deletion lock per Q-CD-15 (≥ 2 ops approvals; ≥ 7d wait)
[ ] regional failover (Q-CD-14): provider must support primary + secondary region
```

**No vendor / region / account name appears in this design or any tracked
doc.** Q-CD-5 (vendor) and Q-CD-14 (regions) remain operator-side
sub-decisions resolved in the offline binder.

---

## 7. Operational design

### 7.1 Deployment topology

```
[ backend app pod / VM ]  ──mTLS──▶  [ signer service pod / VM ]  ──IAM──▶  [ KMS / HSM / MPC ]
       │                                       │                                  │
       └─ structured logs ◀───────────────────┴─ structured logs ──────────────┘
                                                        │
                                                        ▼
                                          [ append-only audit sink (S3/GCS/CloudWatch + retention ≥ 1y) ]

                                          [ metrics / tracing / alerts → PagerDuty + ops Discord ]
```

- Backend and signer service deploy independently. Signer image is small,
  rebuilt and signed independently of backend.
- Signer service is reachable **only** by backend pods (VPC + service mesh).
  No public ingress.
- KMS / HSM / MPC is reachable **only** by the signer service identity
  (provider IAM). Backend identity has zero KMS permissions.

### 7.2 Network model

| Hop                       | Protocol              | Authentication                                                  |
| ------------------------- | --------------------- | --------------------------------------------------------------- |
| backend ↔ signer          | HTTPS/2 with mTLS     | client cert (SPIFFE / IAM-issued); both ends pin                |
| signer ↔ KMS              | provider HTTPS / native | provider IAM role; signer identity scoped to `sign_digest` only |
| backend ↔ chain RPC       | HTTPS                 | RPC provider key; **not used by signer**                        |
| signer service ingress    | mTLS only             | no plain HTTP listener; no `/healthz` without mTLS              |
| operator break-glass      | IAM-gated CLI         | per custody §4.2; produces audit event                          |

No public unauthenticated signing endpoint exists. The signer service has
no `/sign?msg=…` or `/sign_raw_tx` surface; only `/sign_option_execution_tx`
matching the §3 schema.

### 7.3 Rate limits

| Limit                                  | Default                  | Source                                |
| -------------------------------------- | ------------------------ | ------------------------------------- |
| per-signer-identity sign requests/min  | 60                       | custody §6.6 baseline                 |
| per-signer-identity sign requests/hour | 1800                     | derived                               |
| per-`intent_id` final-signature count  | 1                        | A12 idempotency                       |
| per-`request_id` retry count           | 1                        | A11 idempotency (in dedupe window)    |
| per-target / per-selector rate         | inherits identity bucket | (room for future per-pair carving)    |
| emergency zero-rate disable            | toggleable               | break-glass per §7.5                  |

The signer service exposes an `IAM-gated` toggle to set the rate limit to
zero (effectively disabling new signs) without revoking the KMS key. Used
during incident response (§7.5).

### 7.4 Rotation (warm-spare model)

Aligned with custody policy §9 + Cluster 2 §4.1 (key-deletion approval lock).

```
1. Provision NEW key inside KMS (out-of-band; operator + security).
   - new key handle: optionBackendExecutorNext
2. Derive NEW EOA address via signer adapter.derive_address(next).
3. Stage NEW signer configuration alongside OLD (signer service supports
   dual-active read-only verification).
4. Operator (OPS multisig) submits NEW_OME.setExecutor(BACKEND_EXECUTOR_NEXT)
   AND keeps BACKEND_EXECUTOR_OLD as authorised executor during overlap.
5. Backend config switched to point at NEW signer key handle (signer
   service config update, NOT backend .env edit; backend continues to
   point at the same signer endpoint).
6. Sepolia / staging smoke against NEW.
7. After observed clean window (≥ 24h), operator REVOKES OLD via
   NEW_OME.removeExecutor(BACKEND_EXECUTOR_OLD).
8. Drain OLD EOA's gas to Treasury (operator-issued tx; not signer).
9. Disable OLD key in KMS (`disable_key`; NOT delete; per Q-CD-15 scheduled
   deletion requires ≥ 2 ops approvals + ≥ 7d wait).
10. Archive OLD audit logs to long-term retention sink (≥ 1y).
```

All cross-references to `~/DEOPT/MAINNET_CUSTODY_POLICY.md §9.2 + §9.3`
(role-graph migration phases). No on-chain action by this design milestone.

### 7.5 Incident response

| Scenario                                    | Detection                                                                                       | Response                                                                                                                                                                                |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Suspected signer service compromise          | sign-rate spike, allowlist-violation rate spike, integrity-alert trip on audit sink            | (1) set signer rate to zero; (2) operator revokes BE executor on NEW_OME via OPS Safe; (3) initiate §7.4 rotation; (4) preserve full audit logs; (5) page security per §3.1 of alerts spec. |
| KMS / HSM / MPC outage                       | `kms_unavailable` / `kms_timeout` reject-rate alert                                              | Backend marks subsequent intents `broadcast_failed`; users see retry-later UX; operator investigates vendor status; **no fallback to local key** (fail-closed; custody BE-5).             |
| Signer service outage (KMS healthy)          | mTLS handshake / connection alert                                                                | Same: fail-closed; investigate; if outage > N minutes, operator may revoke executor + initiate rotation.                                                                                |
| Duplicate signature attempt for same intent  | A12 (`duplicate-intent-signed`) reject; alert if rate > 0                                        | Investigate backend dedupe state; possible bug in `should_broadcast` decision flow.                                                                                                       |
| Unknown-target signing request               | A2 (`target-not-allowed`) reject                                                                  | **Page**: should never happen; possible compromise upstream.                                                                                                                              |
| Rate-limit breach                            | A14 (`rate-limit`) reject                                                                         | Alert + investigate (DoS or runaway loop in backend).                                                                                                                                    |
| Calldata/policy fingerprint mismatch surge   | A9 (`policy-fingerprint`) reject-rate alert                                                       | Investigate: backend tampering, time skew, or stale-policy reuse bug.                                                                                                                    |
| `post-sign-from-mismatch` (A15)              | impossible-under-correct-KMS — implies wrong key in slot                                          | **Page** + immediately rate-zero + revoke executor on-chain.                                                                                                                              |

---

## 8. Test strategy (deferred to BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER)

This design enumerates the test surface; implementation lands in the next
milestone.

### 8.1 Unit tests (signer-side)

- Request schema validation: missing field, oversize body, malformed hex,
  unknown chain.
- Allowlist deny matrix: A1–A15 — every reject code has at least one test.
- Calldata hash binding: tampered calldata → `calldata-bind-mismatch`.
- Policy fingerprint: rotation of `policy_decision_id` / `nonce` /
  `simulation_block` / `deadline` / `calldata_hash` each yields fingerprint
  mismatch.
- Stale policy: clock advance past `STALE_POLICY_MAX_AGE_MS` → `policy-stale`.
- Duplicate `request_id` → `duplicate-request-id` (dedupe TTL respected).
- Duplicate `intent_id` after successful sign → `duplicate-intent-signed`.
- Post-sign address mismatch: synthesised KMS adapter returning a different
  key → `post-sign-from-mismatch` + critical-alert emission.
- Redaction: structured logger fixture asserts no 0x32/0x40/0x64-hex secret
  candidate leaks; no `DATABASE_URL` / `ADMIN_TOKEN` / `RPC_*` substring.

### 8.2 Backend-side unit tests (next milestone)

- `RemoteSigner` trait: blanket mock implementation that returns
  predetermined `SignerResponse` / `SignerError`.
- `LocalDevSigner` retained behind `EXECUTOR_ALLOW_LOCAL_SIGNER` flag;
  mainnet startup guard rejects when `EXECUTOR_PRIVATE_KEY.is_some() AND
  chain_id == 8453`.
- `policy_fingerprint` recompute identity between backend and signer (round-
  trip vector test).
- `broadcast_option_execution_intent_with_provider` swaps `ExecutorSigner`
  for `RemoteSigner`; existing `policy_approve_preserves_existing_broadcast_state_machine`
  + `policy_reject_transitions_cleanly_without_half_state` regression
  tests continue to pass (Phase D of `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`).

### 8.3 Integration tests

- **Mock remote signer**: in-process `tokio` server implements §3 schema;
  backend integration test issues a sign request, asserts signature
  round-trips and intent transitions to `broadcast_submitted` (using the
  existing `MockBroadcastProvider` from `service.rs::tests`).
- **Local dev signer**: end-to-end against `LocalDevSigner` on chain
  id 31337 (`anvil`) and 84532 (Sepolia fixtures); preserves existing
  Sepolia rehearsal assumption set.
- **Remote signer sandbox**: optional, gated by `SIGNER_SANDBOX_URL` env;
  exercises the real vendor adapter against a vendor sandbox key once
  Q-CD-5 vendor is selected and a sandbox key exists.

### 8.4 Regression tests

- `FIRST_LIVE_OPTION_EXECUTION_SMOKE_RESULT_V2_SEPOLIA.md` shape continues
  to match after the swap.
- `FIRST_LIVE_RFQ_OPTION_EXECUTION_SMOKE_RESULT_SEPOLIA.md` shape continues
  to match after the swap.
- `kms_request_id` correlation: backend log → signer log → (eventually)
  chain receipt forensic correlation.

### 8.5 No-live-broadcast posture

Every test runs against a mock provider, an in-process signer, or `anvil`.
No mainnet RPC. No mainnet broadcast. No vendor key creation. The
`MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION` 4-signature attestation gate
(custody policy §11) is the only path to a real mainnet sign.

---

## 9. Manifest implications (read-only; no edits)

| Manifest slot                                                              | Status after this design                                                |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `matchingExecutors.options[0].executor`                                    | BLOCKED on vendor selection (Q-CD-5) + KMS key generation               |
| `matchingExecutors.perps[0].executor`                                      | BLOCKED per Cluster 2 §2.3 (perp scaffold not in launch)                |
| `governanceRoles.kmsKeyHandles.optionBackendExecutor`                      | NEW schema slot recommended (per Cluster 2 §5.3); opaque label, not secret |
| `governanceRoles.kmsKeyHandles.optionBackendExecutorNext`                  | NEW schema slot for warm-spare rotation                                  |
| `backendSigner.endpointUrl`                                                | NEW schema slot (signer microservice URL; not secret)                    |
| `backendSigner.mtlsClientCertRef`                                          | NEW schema slot (opaque secret-store reference; not the cert bytes)      |
| `backendSigner.stalePolicyMaxAgeMs`                                        | NEW schema slot (recommended default 30_000)                             |

All schema additions are recommendations — actual schema-extension PR
sits under `MAINNET_MANIFEST_DEPENDENCY_SNAPSHOT_AFTER_CUSTODY_CLUSTERS.md`
follow-on track, not this milestone.

---

## 10. Open items / pending sub-decisions

| #  | Item                                                                                        | Owner                  |
| -- | ------------------------------------------------------------------------------------------- | ---------------------- |
| 1  | Q-CD-5 vendor selection (KMS / HSM / MPC provider name)                                     | Operator + Security    |
| 2  | Q-CD-14 region pair finalisation (primary + secondary; vendor-dependent)                    | Operator + Security + DevOps |
| 3  | Q-CD-15 vendor-specific deletion-lock procedure                                              | Operator + Security    |
| 4  | mTLS issuance topology (private CA vs SPIFFE/SPIRE vs cloud-managed)                         | DevOps + Security      |
| 5  | Audit-log sink choice (S3 + Object Lock vs CloudWatch with retention vs GCS Bucket Lock)     | DevOps + Compliance    |
| 6  | Per-pair / per-tier rate-limit carving (deferred; baseline is single bucket per identity)    | Risk + Backend         |
| 7  | KMS-key-generation runbook (operator-side; out-of-tree)                                      | Operator + Security    |

---

## 11. What this design is NOT

- It is NOT a vendor selection. Q-CD-5 remains operator-side.
- It is NOT an implementation. The next milestone
  `BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER` lands the `RemoteSigner`
  trait, the `LocalDevSigner` retention, the mainnet startup guard, and
  the in-tree mock-server integration test.
- It is NOT a mainnet cutover plan. That is V2G-Y phase Y-F plus
  `MAINNET_FIRST_LIVE_SMOKE_AUTHORIZATION` (custody §11).
- It does NOT specify, mention, or recommend any specific KMS / HSM / MPC
  vendor account, ARN, region pair, or IAM role string.

## 12. Cross-references

- `MAINNET_CUSTODY_POLICY.md §6 + §7 + §9 + §10 + §13.3`
- `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §1 + §5 + §6.1`
- `BACKEND_SHOULD_BROADCAST_ECONOMIC_GATE_RESULT.md §3 + §7`
- `SHOULD_BROADCAST_DESIGN_NOTE.md §1 + §3 + §6`
- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` (Sepolia precedent)
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §2.1 + §7.1`
- `BACKEND_MAINNET_IMPLEMENTATION_ROADMAP.md §1.2`
- `MAINNET_AUDIT_EXT_KICKOFF_BUNDLE.md` (auditor anchor for Q-26..Q-29)
- `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_NEXT_TASK.md` (next milestone)
