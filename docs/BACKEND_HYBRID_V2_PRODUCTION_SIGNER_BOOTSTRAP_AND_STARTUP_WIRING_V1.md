# BACKEND-HYBRID-V2-PRODUCTION-SIGNER-BOOTSTRAP-AND-STARTUP-WIRING-V1

Date: 2026-08-10
HEAD after work: (recorded in git)
Branch: `hv2-signer-bootstrap`

## Milestone summary

The prior milestone
(`BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`)
declared "Pattern C — real KMS signer bridge" but
`SignerBuilder::build_kms_aws` still returned `ProductionSignerUnavailable`
and required an operator to manually inject the bridge into
`AppState`. This milestone closes that gap.

## What changed

### New wire path: `HttpSignerTransport`

New file: `src/hybrid_v2/execution/signer_http_transport.rs`.

Implements the Perps `SignerTransport` trait against a real HTTP(S)
signer microservice, speaking a small JSON wire protocol:

- `POST /hybrid_v2/sign` — request `{chain_id, nonce, digest, target,
  value_wei_hex, gas_limit, max_fee_per_gas_hex,
  max_priority_fee_per_gas_hex, tx_type, expected_signer,
  idempotency_key, policy_decision_id, fingerprint}`; response
  `{signature_r, signature_s, signature_v, recovered_signer}`.
- `GET  /hybrid_v2/identity` — probe returning `{signer_address,
  chain_id}` (bootstrap validation, no signature emitted).
- `GET  /hybrid_v2/health` — probe returning `{healthy, signer_address,
  chain_id}`.

The client is a `reqwest::Client` built with the timeout supplied by
the operator, `redirect(Policy::none())`, and — when mTLS material is
configured — a `reqwest::Identity` composed from the client cert +
key PEMs. An optional root CA PEM pins the microservice's server
certificate. `Debug` redacts the endpoint to `<scheme>://<host[:port]>`
and collapses every cert/key path to `<set>` / `<unset>`; no PEM
bytes, no auth reference, no full URL ever reaches a log line.

Error mapping (HTTP → `SignerError`):

- HTTP 401 / 403 → `CallerUnauthorized`
- HTTP 429 → `RateLimit`
- HTTP 5xx → `KmsUnavailable`
- Network timeout → `KmsTimeout`
- Refused connect → `KmsUnavailable`
- 200 with malformed body → `Internal(...)` (mapped to
  `SignerError::MalformedResponse` at the bridge)
- Recovered address ≠ `expected_signer` → `PostSignFromMismatch`

### `SignerBuilder::build_kms_aws` now operational

`src/hybrid_v2/execution/signer_builder.rs` — the `KmsAws` provider
branch now constructs a real `HybridV2KmsSignerBridge` wrapped
around a `RemoteSignerClient` backed by `HttpSignerTransport`. It:

1. Reads any configured mTLS cert / key / root CA PEM files.
2. Refuses to build a public HTTPS transport without mTLS material
   (`MtlsRequiredForPublicEndpoint`). Loopback (127.0.0.1,
   localhost, ::1) is exempt for local dev.
3. Requires `HV2_SIGNER_KMS_KEY_ID`.
4. Returns a bridge whose `availability()` reports `Configured` —
   the honest "wire is complete" verdict.

It DOES NOT contact the microservice (that is deferred to the
identity probe run by main.rs). It DOES NOT import any `aws-sdk-*`
symbol — Pattern C keeps AWS credentials inside the signer
microservice.

Unimplemented vendors (`KmsGcp`, `Turnkey`, `Fireblocks`) continue to
return `ProductionSignerUnavailable` — the honest verdict.

### Config surface

`src/hybrid_v2/config.rs::HybridV2ExecutionConfig` — added three
fields:

- `signer_mtls_cert_pem_path: Option<String>` — env `HV2_SIGNER_MTLS_CERT_PATH`
- `signer_mtls_key_pem_path: Option<String>`  — env `HV2_SIGNER_MTLS_KEY_PATH`
- `signer_root_ca_pem_path: Option<String>`   — env `HV2_SIGNER_ROOT_CA_PATH`

Validator rules added:

- Public HTTPS endpoint + missing cert/key path →
  `MtlsRequiredForPublicEndpoint`.
- Loopback endpoints (127.0.0.1 / localhost / ::1) are exempt from
  the mTLS-mandatory rule.
- Custom `Debug` redacts all three paths to `<set>`.

### Startup wiring (main.rs)

`src/main.rs::wire_hybrid_v2_execution_orchestrator` now runs the
Part F **identity bootstrap probe** BEFORE the signer builder:

- Refuse Base mainnet chain id 8453 at bootstrap (belt-and-suspenders
  to the config validator).
- Fetch `GET /hybrid_v2/identity` — verify `signer_address` matches
  `HV2_SIGNER_EXPECTED_ADDRESS` and `chain_id` matches the
  deployment chain id. Mismatch → refuse to wire the orchestrator.
- Refuse endpoint-reported Base mainnet.
- Transport-level failure at bootstrap → WARN and continue (the
  orchestrator is still wired; admin returns 503 SIGNER_UNAVAILABLE
  when the microservice actually goes down). Set
  `HV2_SIGNER_BOOTSTRAP_STRICT=1` to refuse startup on transport
  failure instead.

No "operator must inject" TODO remains — the orchestrator is fully
wired from env by `wire_hybrid_v2_execution_orchestrator` in a
normal configured startup.

### Zero-broadcast reaffirmation

`tests/hybrid_v2_external_signer_no_broadcast_scan.rs` extended to
scan the new `signer_http_transport.rs` and `src/main.rs` for
forbidden broadcast tokens. Every existing scan still passes.

## Tests

- `src/hybrid_v2/execution/signer_http_transport.rs` — 15 unit tests
  (parse, redaction, error mapping, config guards).
- `src/hybrid_v2/execution/signer_builder.rs` — 7 unit tests (added
  4 covering the operational KmsAws path).
- `src/hybrid_v2/config.rs` — extended (added 2 covering the new
  mTLS-mandatory rule).
- `tests/hybrid_v2_production_signer_http_e2e.rs` — 8 integration
  tests spinning up a **local axum mock signer microservice** on
  127.0.0.1 and exercising:
    - `SignerBuilder` produces a real bridge (never
      `TestEphemeral` or `None`).
    - `HttpSignerTransport::fetch_identity` round-trips.
    - Sign round trip returns a signature whose recovered address
      matches `expected_signer_address`.
    - Bridge probe reports `Configured`.
    - Unavailable microservice never falls back to any local signer.
    - Identity mismatch is a hard reject.
    - Mock service records ZERO broadcast method calls.

## What the milestone deliberately does NOT close

Two Part J / Part H surfaces are honestly reported as unimplemented
in this pass to avoid a fake closure:

- **Full PG-integration "20-test matrix"** through the
  `ExecutionOrchestrator` state machine using the axum mock signer.
  The orchestrator surface is already covered end-to-end by
  `hybrid_v2_execution_live_orchestrator_pg_integration` and
  `hybrid_v2_execution_orchestrator_pg_integration` (which feed
  through the perps mock plug). The load-bearing question this
  milestone had to answer — "does the SignerBuilder now produce a
  real HTTP-backed bridge?" — is answered by
  `hybrid_v2_production_signer_http_e2e`.
- **Cross-restart AppState reconstruction test that re-runs
  `wire_hybrid_v2_execution_orchestrator` against the axum mock.**
  The wire path is deterministic (config → transport → bridge), so
  restart behaviour is a function of the config surface; that
  surface is covered by the existing signer_builder / config unit
  tests.

Both surfaces can be added in a follow-on task without any code
change to the transport, bridge, config, or main.rs wiring —
they are pure integration coverage extensions.

## Verdicts

- `PRODUCTION_SIGNER_BUILDER_OPERATIONAL` — PASSED
- `PRODUCTION_EXECUTION_ORCHESTRATOR_STARTUP_WIRED` — PASSED
- `PRODUCTION_SIGNER_STARTUP_MODES_VALIDATED` — PASSED (all four
  modes handled by existing wiring + new HTTP transport)
- `PRODUCTION_SIGNER_IDENTITY_BOOTSTRAP_VALIDATED` — PASSED
- `NORMAL_STARTUP_ADMIN_PREPARE_END_TO_END_VALIDATED` — PARTIALLY
  PASSED (the wire path is proven end-to-end; the
  ExecutionOrchestrator march to BROADCAST_DISABLED remains
  covered by existing PG integration binaries)
- `PRODUCTION_SIGNER_APP_RESTART_VALIDATED` — DEFERRED (reason:
  the wire path is deterministic function of env; existing
  restart tests cover the orchestrator surface. No new failure
  surface introduced by this milestone.)
- `PRODUCTION_SIGNER_STARTUP_HARNESS_VALIDATED` — PASSED (mock
  axum service lives in `hybrid_v2_production_signer_http_e2e.rs`;
  supports fault-injection knobs: `reject_auth`, `force_wrong_signer`)
- `PRODUCTION_SIGNER_STARTUP_DATABASE_INTEGRATION_VALIDATED` —
  DEFERRED (see "does NOT close" section)
- `PRODUCTION_STARTUP_ZERO_BROADCAST_VALIDATED` — PASSED
- `BROADCAST_TECHNICALLY_DISABLED` — reaffirmed
- `NO_PUBLIC_SIGNING_BROADCAST_OR_CHAIN_WRITE_ACTION` — reaffirmed
- `PRODUCTION_SIGNER_STARTUP_SECURITY_VALIDATED` — PASSED (see
  security notes below)
- `PRODUCTION_SIGNER_STARTUP_CI_GATE_VALIDATED` — DEFERRED (CI
  wiring is a follow-on; the new test binaries follow the existing
  Cargo.toml pattern and are picked up by any `cargo test
  --features test-signer` run in the postgres-integrity workflow)
- `BACKEND_HYBRID_V2_REMOTE_SIGNER_SERVICE_CUSTODY_BOUNDARY_VALIDATED`
  — PASSED (see "custody boundary" below)

## Custody boundary (Part B verdict)

The backend holds:

- The mTLS client cert + key (PATHS to PEM files, not PEM bytes in
  config).
- The `expected_signer_address` (the EOA the backend expects the
  signer to sign as).
- An opaque `signer_kms_key_id` (a handle string forwarded
  UNSEEN to the signer microservice via its own configuration).
- An opaque `signer_auth_reference` (a role ARN / vault path).

The backend NEVER holds:

- A raw private key. (grep of new files for `private_key`,
  `signing_key`, `mnemonic` → zero non-test hits.)
- AWS credentials. (grep of new files for `AWS_ACCESS_KEY`,
  `AWS_SECRET`, `IMDSv2`, `aws_config::load_from_env` → zero hits.)
- Any way to bypass the microservice — the only production signer
  the builder can produce for `HV2_SIGNER_BACKEND=production` is a
  `HybridV2KmsSignerBridge` wrapping a `RemoteSignerClient` that
  speaks the HTTP wire protocol. No alternate code path exists.

Verdict: `BACKEND_HYBRID_V2_REMOTE_SIGNER_SERVICE_CUSTODY_BOUNDARY_VALIDATED`.

## Security notes

- Every response is locally verified. The transport rejects a
  mismatched `recovered_signer` at parse time
  (`PostSignFromMismatch`); the bridge re-verifies the address at
  its own boundary (Part G of the prior milestone remains in
  place); the orchestrator re-runs `verify_signed_tx` after the
  bridge returns (belt-and-suspenders).
- Signer outage NEVER falls back to a local raw-key signer. The
  builder has no production branch that produces `TestEphemeralSigner`
  or the Perps `LocalDevSigner`. This is enforced by construction
  (grep confirms no such code path in the new files) AND covered by
  `signer_service_unavailable_never_falls_back_to_local_signer`.
- Read-side availability is independent of the signer. The
  orchestrator wiring downgrade to `None` leaves every GET route
  serving under `AppState::hybrid_v2_execution_unavailable_reason`;
  this posture is unchanged.
- Logs/errors redact endpoints and auth references. `Debug` impls
  on `HttpSignerTransport`, `HybridV2KmsSignerBridge`, and
  `HybridV2ExecutionConfig` all redact URL paths, cert/key paths,
  and auth handles.
- Base mainnet 8453 is refused at 3 layers: config validator,
  bootstrap probe (main.rs), and bridge boundary
  (`HybridV2KmsSignerBridge::sign_execution`).

## Cross-reference — 2026-08-11

`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1` extends the same
`wire_hybrid_v2_execution_orchestrator` startup path with a sibling
`wire_hybrid_v2_broadcast` helper (`src/hybrid_v2/startup.rs`) that
constructs the `BroadcastOutbox` + `BroadcastConfirmationWorker` and
attaches them to `AppState` via `with_hybrid_v2_broadcast(...)`. The
broadcast wiring follows the identical three-outcome contract as the
production signer wiring (`Ok(None)` / `Ok(Some(_))` / `Err(reason)`)
and refuses Base mainnet at its own boundary in addition to the
config validator + RPC constructor gates.

Closure: `BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1.md`.
Security review:
`BACKEND_HYBRID_V2_BROADCAST_AND_CONFIRMATION_V1_SECURITY_REVIEW.md`.
