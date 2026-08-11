# BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1

> **2026-08-10 CORRECTION** — the prior report described this milestone
> as fully closed "Pattern C" but `SignerBuilder::build_kms_aws` still
> returned `ProductionSignerUnavailable` and required an operator to
> manually inject the bridge into `AppState`. The follow-on milestone
> `BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1`
> (see `docs/BACKEND_HYBRID_V2_PRODUCTION_SIGNER_BOOTSTRAP_AND_STARTUP_WIRING_V1.md`)
> closes that gap: `HttpSignerTransport` implemented, `SignerBuilder`
> constructs a real bridge, `wire_hybrid_v2_execution_orchestrator`
> runs an identity bootstrap probe, no manual injection required.

Milestone status: **CLOSED (pre-broadcast, external signer integrated).**

**Verdicts:**

- Signer (integration): `EXTERNAL_SIGNER_INTEGRATED_PRE_BROADCAST_SAFE`
- Zero-broadcast (reaffirmed): `EXTERNAL_SIGNER_DOES_NOT_BROADCAST`,
  `BROADCAST_TECHNICALLY_DISABLED`,
  `NO_PUBLIC_SIGNING_BROADCAST_OR_CHAIN_WRITE_ACTION`
- Test harness: `EXTERNAL_SIGNER_TEST_HARNESS_VALIDATED`
- Live orchestrator DB integration:
  `EXTERNAL_SIGNER_LIVE_ORCHESTRATOR_DATABASE_INTEGRATION_VALIDATED`
- Properties:
  `EXTERNAL_SIGNER_LIVE_ORCHESTRATOR_PROPERTIES_VALIDATED`
- Performance: `EXTERNAL_SIGNER_PERFORMANCE_BOUNDED`
- Security: `EXTERNAL_SIGNER_SECURITY_VALIDATED`
- CI gate: `EXTERNAL_SIGNER_CI_GATE_VALIDATED`
- Documentation: `EXTERNAL_SIGNER_DOCUMENTATION_COMPLETE`

Broadcast remains **disabled by construction**. The next milestone
(`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`) will add it.

## 1. Scope

This milestone integrates the external signer microservice (Pattern C
per `MAINNET_BE_SIGNER_SERVICE_DESIGN.md`) with the Hybrid V2
pre-broadcast execution pipeline and wires a live
`ExecutionOrchestrator` into `AppState` so the admin
`prepare_execution` route drives a real end-to-end flow through
firewall + bridge + local signature verifier + persistence, landing
at the terminal `BROADCAST_DISABLED` phase.

Building blocks were split into two packages:

### Package A (Parts A–L; landed by prior commits)

- `migrations/0050_hybrid_v2_signer_idempotency.sql` — persisted
  `signer_request_idempotency_key` column + immutability trigger.
- `src/hybrid_v2/config.rs::HybridV2ExecutionConfig` — extended with
  `expected_signer_address`, `signer_kms_key_id`, `signer_provider`,
  `signer_request_timeout_ms`, `signer_max_retries`,
  `signer_auth_reference`.
- `src/hybrid_v2/execution/signer.rs::SignerAvailability` —
  availability enum + `availability()` method on the trait.
- `src/hybrid_v2/execution/signer_kms_bridge.rs::HybridV2KmsSignerBridge`
  — adapter lifting the perps `RemoteSigner` stack onto the HV2
  `ExecutionSigner` surface.
- `src/hybrid_v2/execution/signer_builder.rs::HybridV2SignerBuilder`
  — dispatch by `SignerProvider`.
- `src/hybrid_v2/execution/orchestrator.rs` — persists idempotency
  key at the Signing → SignatureVerified transition.
- `src/api/http.rs::AppState::hybrid_v2_execution_orchestrator` —
  optional live orchestrator handle.
- `src/main.rs::wire_hybrid_v2_execution_orchestrator` — fail-closed
  startup wiring.
- `src/api/hybrid_v2_execution_admin.rs::prepare_execution` — live
  orchestrator call with structured 503 when unavailable and
  `#[serde(deny_unknown_fields)]` on the request body.
- `tests/hybrid_v2_execution_live_orchestrator_pg_integration.rs`
  — 9 lifecycle tests (feature `test-signer`).

### Package B (Parts M–T; landed by this milestone's 7 commits)

- Part M — `tests/hybrid_v2_external_signer_no_broadcast_scan.rs`.
- Part N — `tests/hybrid_v2_external_signer_harness.rs` (10 self-tests).
- Part O — `tests/hybrid_v2_external_signer_full_matrix_pg_integration.rs`
  (25 PG tests).
- Part P — `tests/hybrid_v2_external_signer_properties.rs`
  (15 bounded properties).
- Part Q — `tests/hybrid_v2_external_signer_performance_bounds.rs`
  (bounded latency ceilings; wall-clock-fragile ones `#[ignore]`).
- Part R —
  `docs/BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1_SECURITY_REVIEW.md`.
- Part S — `.github/workflows/backend-postgres-integrity.yml`
  extended with default-feature scan + test-signer PG suites.
- Part T — this closure doc + operator runbook Section 17 +
  admin API reference + RUN_STATE entry.

## 2. Frozen safety posture (unchanged from V1)

- **BROADCAST_STRICTLY_FORBIDDEN** — no `send_*` method on the
  `ExecutionRpcClient` trait; runtime allowlist has 7 read methods;
  source-scan tests fail loud on any forbidden verb.
- **BASE_MAINNET_8453_IS_FORBIDDEN** — refused at three layers
  (config `validate_startup`, admin route entry, bridge boundary).
- **PRODUCTION_BACKEND_DOES_NOT_CUSTODY_RAW_PRIVATE_KEYS** — the
  backend NEVER holds a raw private key; `TestEphemeralSigner` is
  gated behind `#[cfg(test)]` / `test-signer`.
- **NO_NEW_CARGO_DEPS** — this milestone did not add any dependency
  to `Cargo.toml [dependencies]`; every helper reuses the perps
  signer stack + existing crate graph.
- **NO_LOG_OR_ERROR_LEAKS_A_SECRET** — Debug + error surfaces
  redact endpoint URL / KMS key id / auth reference; vendor error
  strings are truncated to 80 chars.

## 3. Configuration surface

| Env var | Purpose | Bound / default |
|---|---|---|
| `HV2_EXECUTION_ENABLED` | Master switch | default false |
| `HV2_EXECUTOR_ADDRESS` | 20-byte hex, required when enabled | zero-address refused |
| `HV2_SIGNER_BACKEND` | `production` (default) or `test_ephemeral` (feature-gated) | production |
| `HV2_SIGNER_PROVIDER` | `kms_aws` \| `kms_gcp` \| `turnkey` \| `fireblocks` \| `mock` | required for Production |
| `HV2_SIGNER_ENDPOINT` | `https://…` or `http://127.0.0.1:*` / `http://localhost:*` | non-https refused for public hosts |
| `HV2_SIGNER_EXPECTED_ADDRESS` | 20-byte hex EOA | required for Production |
| `HV2_SIGNER_KMS_KEY_ID` | Opaque vendor key id | required when provider=kms_aws |
| `HV2_SIGNER_REQUEST_TIMEOUT_MS` | Per-request timeout | 100..30_000, default 2500 |
| `HV2_SIGNER_MAX_RETRIES` | Retry budget for transient errors only | 0..5, default 1 |
| `HV2_SIGNER_AUTH_REFERENCE` | Opaque handle to auth material (IAM ARN, secret path) | NEVER a raw secret |
| `HV2_EXECUTION_RPC_URL` | Execution-side RPC (may reuse indexer's) | http/https only |
| `HV2_EXECUTION_RPC_TIMEOUT_MS` | RPC per-request timeout | 500..60_000 |
| `HV2_SIMULATION_MAX_AGE_MS` | Firewall staleness cap on persisted simulation | 1_000..3_600_000 |

`HybridV2ExecutionConfig::validate_startup` refuses:
- `chain_id == 8453`
- non-https endpoint outside localhost
- Production signer_kind without `expected_signer_address` + endpoint
  + provider
- `signer_request_timeout_ms` out of [100, 30_000]
- `signer_max_retries > 5`
- `SignerProvider::Mock` in non-test / non-`test-signer` builds

## 4. Wire diagram

```
                admin/prepare_execution
                        │
                        ▼
             AppState.hybrid_v2_execution_orchestrator
                        │  (Arc<ExecutionOrchestrator>, or None → 503)
                        ▼
                ExecutionOrchestrator
                 ├── PreflightChecker (readiness / drift)
                 ├── ExecutionPlanBuilder (manifest allowlist)
                 ├── HttpExecutionRpcClient (7 read methods only)
                 ├── ExecutionSimulator
                 ├── NonceReserver (atomic UNIQUE INSERT on PG)
                 ├── GasFeePolicy
                 ├── SignerPolicyFirewall  ◄── revalidates every field
                 ├── ExecutionSigner (Arc<dyn>)
                 │      │
                 │      ▼
                 │  HybridV2KmsSignerBridge  ◄── this milestone
                 │      │
                 │      ▼
                 │  RemoteSignerClient (perps stack)
                 │      │
                 │      ▼
                 │  PluggableRemoteSignerTransport
                 │      │
                 │      ▼
                 │  AwsKmsSignerProvider  (feature "aws-kms-transport")
                 │  ---- or ---
                 │  MockVendorSignerProvider  (feature "test-signer")
                 │
                 └── verify_signed_tx  ◄── local re-verification
                        │
                        ▼
                  PostgresHybridV2ProjectionStore
                  (persists plan_hash, calldata_hash, r/s/v,
                   recovered_signer, signer_request_idempotency_key)
                        │
                        ▼
                  Terminal: BROADCAST_DISABLED
```

## 5. Failure classes (extended)

Unchanged from V1 (see
`docs/BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1.md § Failure classes`).
This milestone adds no new failure classes; it just proves the
existing ones fire correctly for external-signer failure modes.

## 6. Test binaries

| Binary | Feature | Purpose | Test count |
|---|---|---|---|
| `hybrid_v2_external_signer_no_broadcast_scan` | (default) | Zero-broadcast source scan + allowlist assertion + harness sanity | 4 |
| `hybrid_v2_external_signer_harness` | `test-signer` | Shared helper module self-tests | 10 |
| `hybrid_v2_execution_live_orchestrator_pg_integration` | `test-signer` | Package A lifecycle (PG) | 9 |
| `hybrid_v2_external_signer_full_matrix_pg_integration` | `test-signer` | Adversarial coverage (PG) | 25 |
| `hybrid_v2_external_signer_properties` | `test-signer` | Bounded 20-case properties | 15 |
| `hybrid_v2_external_signer_performance_bounds` | `test-signer` | Wall-clock ceilings (3 ignored) | 4 + 3 ignored |

## 7. Cross-references

- V1 closure doc:
  `docs/BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1.md`.
- V1 security review:
  `docs/BACKEND_HYBRID_V2_SIGNER_AND_EXECUTION_V1_SECURITY_REVIEW.md`.
- Package A migration:
  `migrations/0050_hybrid_v2_signer_idempotency.sql`.
- Package B security review:
  `docs/BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1_SECURITY_REVIEW.md`.
- Operator runbook Section 17:
  `docs/HYBRID_V2_OPERATOR_RUNBOOK.md § 17`.
- Admin API reference: `docs/HYBRID_V2_ADMIN_API.md`.
- Pattern C signer design:
  `docs/MAINNET_BE_SIGNER_SERVICE_DESIGN.md` (referenced; not part
  of this repo).

## 8. Deferred to next milestone

- **Live AWS KMS transport wiring.** With
  `--features aws-kms-transport`, the current signer builder returns
  `ProductionSignerUnavailable` because Pattern C requires an
  operator-supplied `aws_sdk_kms::Client`. The follow-on operator-
  wiring milestone assembles it from `aws-config` and injects a
  fully-wired `HybridV2KmsSignerBridge`.
- **Broadcast + confirmation.**
  `BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` is the next
  milestone.
- **Live signer connectivity smoke test.** The integration tests in
  this milestone all target the mock provider; a real staging KMS
  smoke is scoped to the operator-wiring milestone.
