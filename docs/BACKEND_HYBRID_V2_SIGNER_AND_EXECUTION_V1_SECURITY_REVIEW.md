# BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1 — Security Review

Milestone status: **Pre-broadcast surface only. Broadcast is disabled by construction.**

Signer verdict: **`BACKEND_HYBRID_V2_SIGNER_INTERFACE_READY_EXTERNAL_SIGNER_REQUIRED`**.

Final verdict: **`BACKEND_HYBRID_V2_SIGNER_EXECUTION_SECURITY_VALIDATED`** (see closing section).

## 1. Scope and threat model

The Hybrid V2 pre-broadcast execution pipeline is a backend-only surface that:

- accepts SIGNED (owner-side EIP-712) buyer/seller order envelopes;
- derives a canonical execution id, deterministic ABI plan, and gas/nonce/signature;
- lands at the terminal `BROADCAST_DISABLED` phase after passing every gate.

Broadcast is **disabled** in this milestone. The `ExecutionRpcClient` trait has no
`send_*` method (compile-time firewall); the runtime allowlist in
`src/hybrid_v2/execution/rpc.rs` and the runtime source-scan in
`tests/hybrid_v2_execution_zero_broadcast_scan.rs` are belt-and-braces
defenses on top of that.

The threat model covers:

- an operator/API caller with valid admin credentials trying to influence
  target/calldata/gas/nonce/chain_id from the request body;
- a compromised RPC provider returning malformed frames, wrong chain id,
  or manipulated fee history;
- a compromised signer returning tampered `(r, s, v)` or claiming a
  different `recovered_signer`;
- a stale simulation reused after a chain-state change;
- an attacker with write access to the projection PostgreSQL trying to
  mutate immutable execution row fields;
- concurrent operator requests racing on the same canonical execution id.

## 2. Execution authority

**On-chain authority is permissionless.** `OptionMatchingEngineV2::executeMatch`
accepts pre-signed EIP-712 envelopes from any msg.sender. The backend acts
purely as the **gas payer / batch relayer** — the recovered signer on-chain
is the OWNER, not the backend.

The backend consequently:

- **Cannot** authorize a match the owner didn't sign — the on-chain call
  reverts.
- **Can** choose to pay gas for a valid signed intent — this is the
  narrow authority the pre-broadcast pipeline exercises.
- **Cannot** exfiltrate collateral or premium — those follow the on-chain
  logic tied to the recovered owner subkey.

## 3. Threat / mitigation matrix

### 3.1 Signature replay (cross-contract)

- Owner-side EIP-712 domain separator (`OptionMatchingEngineV2`) prevents
  replaying a signature bound to a different verifier contract.
- The engine's per-owner `nonce` counter (`SignedActionEnvelope.nonce`)
  prevents same-contract replay.
- The manifest binds `deployment_version` — a wrong deployment_version
  causes the on-chain call to revert.

### 3.2 Signature replay (cross-chain)

- The `SigningRequest.chain_id` field is included in
  `signing_payload_hash` (see `orchestrator::derive_signing_payload_hash`,
  layout begins with `HV2_SIG_V1 || chain_id_be8 || …`). A signature
  produced for chain X cannot be reused on chain Y — the payload hash
  differs.
- On-chain, the engine's EIP-712 domain also binds `chain_id`, adding a
  second independent check.

### 3.3 Signature replay (cross-deployment on same chain)

- `derive_canonical_execution_id` (see `src/hybrid_v2/execution/identity.rs`)
  incorporates `deployment_id` in the SHA-256 preimage. Two deployments
  on the same chain produce distinct canonical ids for the same intent.
- The DB `hybrid_v2_execution_requests.canonical_execution_id` PRIMARY
  KEY blocks two rows from claiming the same id.

### 3.4 Nonce collision (backend-scoped)

- `hybrid_v2_executor_nonces` carries `UNIQUE(chain_id, signer_identity,
  reserved_nonce)`. Two concurrent workers race on the atomic INSERT;
  one wins, the other advances (Part V.9 tests exercise this on real PG).
- The `NonceReserver` computes candidates from both the on-chain
  `pending` nonce and the persisted `get_reserved_nonces_for(chain,
  signer)` set, so a restart never re-issues a slot.

### 3.5 Calldata / target / value / gas substitution

- The plan builder in `src/hybrid_v2/execution/plan.rs` derives calldata
  bytes from the allowlisted `OptionMatchingEngineV2::executeMatch` ABI.
  No caller-influenced field can alter the selector or the argument
  layout.
- `TargetPolicy::is_allowed` refuses unknown targets and wrong selectors
  (`src/hybrid_v2/execution/target_policy.rs`; see also Part V.2 tests).
- The row's `calldata_hash` and `plan_hash` are persisted at first
  successful build; a SQL trigger (migration 0049) refuses any UPDATE
  that would mutate them. Part V and Part W both prove this.
- The `SignerPolicyFirewall` (`src/hybrid_v2/execution/signer_firewall.rs`)
  re-runs an independent verification of target/selector/calldata/value/
  gas/fee/nonce/plan_hash immediately before invoking the signer, using
  the plan REBUILT from the row + intent — a mutated row (or a tampered
  intent) is rejected.

### 3.6 Signer impersonation / response tampering

- The signer trait returns `(r, s, v, recovered_signer)` — never a raw
  transaction blob (structural broadcast kill switch).
- `verify_signed_tx` (`src/hybrid_v2/execution/signature_verify.rs`)
  independently RECOVERS the signer from `(r, s, v)` over the
  `signing_payload_hash` and compares to both the claimed
  `recovered_signer` and the expected signer address; mismatch fails
  closed.
- High-S signatures (EIP-2 malleability) are refused.
- Tampering `r` or `s` after signing produces a recovered signer that
  fails the comparison — Part V and Part W tests cover this on the
  Test signer.

### 3.7 Stale simulation

- The simulator binds `eth_call` to the head block's number **and hash**
  (both persisted as `simulation_block_number` / `simulation_block_hash`).
- The `simulation_max_age_ms` field, honoured by the firewall's
  `revalidate` step, refuses a stale simulation before signing.
- TOCTOU between simulation and (future) broadcast is not applicable
  this milestone because **broadcast is disabled**. A follow-up
  milestone MUST re-verify simulation and readiness at the very moment
  before broadcast is attempted.

### 3.8 Database tampering

- The orchestrator relies on Postgres for its authoritative row state.
  An attacker with direct DB write access can force the row into an
  arbitrary phase — the mitigation for this is **DB access control**
  (per-role write privileges, network isolation), not application code.
- Application-level tamper defense:
  - Immutability triggers on `plan_hash`, `calldata_hash`, and the
    `execution_kind` value (migration 0049).
  - The firewall's independent revalidation catches an on-disk mutation
    that survives the trigger.
- Documented as **out-of-scope for this milestone**: full trust
  boundary between backend process and its database.

### 3.9 Operator authorization

- All 5 admin routes (`src/api/hybrid_v2_execution_admin.rs`) call
  `ensure_admin`, which enforces the standard admin token via
  `AppState::admin_config.token_matches` (constant-time compare in the
  admin config module).
- Missing or invalid token → 403 (`ADMIN_TOKEN_REQUIRED` /
  `INVALID_ADMIN_TOKEN`).
- Base mainnet chain id (8453) is refused at the handler entry point
  (`refuse_mainnet`), so an operator with valid credentials still
  cannot prepare an execution on mainnet.

### 3.10 RPC compromise

- The RPC client (`src/hybrid_v2/execution/rpc.rs`) hits a curated
  allowlist (`ALLOWED_METHODS`). Any other method is refused at the
  wire layer.
- Per-call timeouts + bounded transport retries
  (`MAX_TRANSPORT_RETRIES = 3` — Part X asserts this bound).
- `eth_call` is read-only; the client has no `send_*` method. A
  compromised RPC can lie about state, but cannot cause a broadcast
  from the backend.
- The `hybrid_v2_execution_zero_broadcast_scan` test file-walks the
  entire `src/hybrid_v2/execution/` tree and refuses any token
  matching `send_*` / `eth_sendRawTransaction` / etc.

### 3.11 Signer outage

- The default `SignerBackend::Production` produces
  `ProductionSignerUnavailable` — every sign call returns
  `SignerError::SignerUnavailable`, orchestrator lands the row in
  `Failed(SIGNER_UNAVAILABLE)`. This is a first-class terminal, not a
  silent no-op.

### 3.12 Log / metric secret leakage

- **Signature bytes are never logged.** The signature-verify module has
  no `println!`/`tracing::*` calls (see `signature_verify.rs`).
- The sanitized admin row (`SanitizedExecutionRow`) OMITS
  `signature_r`, `signature_s`, `signature_v`. Only `recovered_signer`
  (a public address value derivable from the eventual on-chain tx) is
  exposed. Part V.12 asserts the JSON serialization does not contain
  the omitted fields.
- The TestEphemeralSigner's `Debug` impl deliberately omits the private
  key (`<redacted>` placeholder).

## 4. Evidence checklist (with file:line references)

| Check | Evidence |
|---|---|
| No raw private key committed | `git grep -n 'private_key\\|MNEMONIC\\|SECRET_KEY' src/` returns nothing under `hybrid_v2/execution/` |
| No mnemonic | Same as above |
| No broadcast capability | `tests/hybrid_v2_execution_zero_broadcast_scan.rs` (source-scan across every `src/hybrid_v2/execution/*.rs`) |
| No Base mainnet acceptance | `src/hybrid_v2/execution/target_policy.rs:75-77` (`BaseMainnetForbidden`); `src/hybrid_v2/execution/orchestrator.rs:236` (early exit); `src/hybrid_v2/execution/preflight.rs:133-138`; `src/api/hybrid_v2_execution_admin.rs:113-122` |
| No public arbitrary tx | `src/api/hybrid_v2_read/router.rs` audit test + `src/api/hybrid_v2_read/router.rs:91` (public boundary contract) |
| Preflight rejects on drift class | `src/hybrid_v2/execution/preflight.rs:176-190` handles `ReconciliationDrift`; `UnsupportedView` case: preflight relies on the readiness aggregator, and the orchestrator additionally requires `readiness.is_ready()` via the readiness contract (see docstring at `preflight.rs:9-15`) |
| No signature accepted without local verification | `src/hybrid_v2/execution/orchestrator.rs:984-994` — `verify_signed_tx` is called BEFORE the row is advanced to `SignatureVerified` and BEFORE the signature is persisted |
| plan_hash immutability | `migrations/0049_hybrid_v2_execution.sql` (immutability trigger). Part V.13 (`plan_hash_immutability_trigger_refuses_mutation`) and Part W (`prop_plan_immutability_after_signing_starts_sql_trigger`) prove this on real PG |
| Bounded RPC retries | `src/hybrid_v2/execution/orchestrator.rs:MAX_TRANSPORT_RETRIES = 3` — Part X asserts this |
| Sanitized admin row omits (r,s,v) | Part V.12 (`sanitized_row_omits_r_s_v_but_carries_recovered_signer`) |

## 5. Reconciliation authority

The audit invariant `UNSUPPORTED_RECONCILIATION_VIEWS_ARE_NOT_EXECUTION_AUTHORITY`
holds:

- Preflight blocks execution when the reconciliation axis reports
  `ReconciliationDrift` (`ProjectionDrift`, `ManifestMismatch`,
  `MalformedChainResponse`).
- When the reconciliation axis reports `UnsupportedView` (indexer cannot
  reach the chain source to compare), preflight relies on
  `ReadinessReport::is_ready`; the orchestrator additionally refuses to
  advance if readiness is not `Ready`. So `UnsupportedView` alone does
  NOT enable execution unless every other axis is Ready.
- The orchestrator never treats a reconciliation "success" as authority
  to bypass simulation — simulation is always the authoritative
  last-word check.

## 6. Known limitations & follow-ups (documented for closure)

1. **Production signer NOT integrated.** The default is
   `ProductionSignerUnavailable`. Production deployments require
   wiring a `SignerBackend::RemoteKMS` or equivalent. This milestone
   ships the interface and the pipeline — not the KMS integration.
2. **Orchestrator not wired into live AppState.** The admin `prepare`
   route returns `503 EXECUTION_ORCHESTRATOR_NOT_WIRED`. Live wiring
   comes with the production signer milestone.
3. **Broadcast is deliberately absent.** The next milestone
   (`BACKEND-HYBRID-V2-BROADCAST-AND-CONFIRMATION-V1`) will introduce
   the broadcast surface, and MUST re-verify simulation freshness and
   readiness immediately before broadcast (TOCTOU class).
4. **DB access control** is out-of-scope. Application-layer defenses
   catch tampering that survives the SQL triggers, but full DB tamper
   resistance requires OS/network-level access-control (Postgres role
   hardening, network isolation).

## 7. Verdict

Every audit-required security concern for the pre-broadcast surface has
been analysed and either mitigated in code (with test coverage) or
documented as out-of-scope with a scheduled follow-up milestone.

**Return value: `BACKEND_HYBRID_V2_SIGNER_EXECUTION_SECURITY_VALIDATED`.**

## 8. Follow-on review

The follow-on milestone
`BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
extends this security surface with external-signer integration
(Pattern C KMS bridge, live orchestrator wiring, persisted signer
idempotency key). See its paired review:
`BACKEND_HYBRID_V2_EXTERNAL_SIGNER_INTEGRATION_AND_LIVE_ORCHESTRATOR_V1_SECURITY_REVIEW.md`.

No frozen invariant from V1 has been retracted; every one is
reaffirmed by the new source-scans and PG matrix in the follow-on
milestone.
