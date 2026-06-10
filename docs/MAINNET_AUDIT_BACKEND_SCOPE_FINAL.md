# MAINNET-AUDIT-EXT — Backend Scope (FINAL)

**Posture:** read-only. **No chain mutation. No `.env` edit. No mainnet. No
secrets. No private signer identity.** Companion to
`~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_FINAL.md`.

**Date:** 2026-06-10
**Anchor commit:** `d133e2c` (`deopt-v2-backend/`)
**Test posture:** `cargo test --all-targets --all-features` → 1053 tests green
at last AWS / KMS milestone closure.

## 1. Broadcast policy gate

The backend's broadcast policy decides whether the executor signs and submits
a transaction to chain. Canonical spec: `BACKEND_GAS_FEES_REBATES_POLICY_V1.md`.
Implementation milestone: `BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE`
(not yet landed — auditor confirms absence by independent
`grep -rn 'fn should_broadcast\|should_broadcast(' src/` at the engagement-kickoff
commit, expected 0 hits).

### 1.1 should_broadcast data sources (planned)

1. **Live provider config flags** (`/executor/health/v2.live_provider_config.*` ALL true at preflight):
   - `econ.fees_manager_v2_address_set`
   - `econ.protocol_fee_vault_address_set`
   - `econ.collateral_vault_address_set`
2. **Effective ppm cache** (`BACKEND-LIVE-PROVIDER-EFFECTIVE-PPM-CACHE` milestone result).
3. **PFV.rebateReserve / FM_V2.rebateBudget** read-only RPC fetch (every executor cycle).
4. **CV.balances(PFV, asset)** read-only RPC fetch.
5. **R5 drift** = `CV.balances(PFV, asset) - (PFV.feeBalance + PFV.rebateReserve)` — MUST be 0.
6. **BE balance floor** (`BACKEND-OBSERVABILITY-BE-BALANCE-FLOOR-EXPOSE` result).
7. **BE EOA gas price / EIP-1559 fee components**.
8. **Last singleton policy data failure** (`last_policy_data_failure` singleton).

Policy decomposition (Spec §8) — the auditor should opine on whether 8 steps
are sufficient or whether a 9th is needed.

## 2. Signer model

Implementation: `src/execution/signer.rs` + `src/execution/aws_kms_transport.rs`.

### 2.1 RemoteSigner trait

```rust
#[async_trait]
pub trait RemoteSigner: Send + Sync {
    async fn sign_prehash(&self, prehash: [u8; 32]) -> Result<RemoteSignature>;
    async fn health_check(&self) -> Result<()>;
    fn address(&self) -> Address;
}
```

### 2.2 PluggableSignerProvider trait

```rust
#[async_trait]
pub trait PluggableSignerProvider: Send + Sync {
    async fn build(&self) -> Result<Arc<dyn RemoteSigner>>;
    fn kind(&self) -> SignerProviderKind;
}

pub enum SignerProviderKind {
    Mock,
    VendorAgnostic,
    AwsKms,
    Turnkey,
    Fireblocks,
    GcpKms,
    AzureHsm,
}
```

### 2.3 RemoteSignerClient

```rust
pub struct RemoteSignerClient<T: AwsKmsTransport> {
    transport: T,
    expected_address: Address,
}
```

**Production `RemoteSignerClient::new` continues to use `UnimplementedTransport`
which fail-closed (`unimplemented!()` on any call). Real AWS calls only
land when the `aws-kms-transport` Cargo feature is on and the operator wires
a real `AwsKmsTransport` impl.**

## 3. AWS KMS model

Implementation: `src/execution/aws_kms_transport.rs` (Cargo feature
`aws-kms-transport`); operator spec: `AWS_KMS_OPERATOR_SETUP_PACK.md` +
4 siblings; backend integration: `BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md`;
CloudTrail correlation: `BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`.

### 3.1 AwsKmsTransport trait

```rust
#[async_trait]
pub trait AwsKmsTransport: Send + Sync {
    async fn get_public_key(&self, key_id: &str) -> Result<(Vec<u8>, Option<String>)>;
    async fn sign(&self, key_id: &str, message: &[u8]) -> Result<(Vec<u8>, Option<String>)>;
}
```

`Option<String>` is the CloudTrail `RequestId` (5-step sanitiser:
empty / whitespace / control chars / URL-shape / length-cap rejection → synthetic UUID fallback).

### 3.2 Key spec + IAM expectations

- Key spec: `ECC_SECG_P256K1` (secp256k1).
- Key usage: `SIGN_VERIFY`.
- Origin: `AWS_KMS` (not `EXTERNAL`).
- IAM role policy MUST allow only `kms:GetPublicKey` + `kms:Sign` + `kms:DescribeKey`. **NEVER `kms:*` wildcards.**
- KMS key policy MUST deny `kms:Delete*` and `kms:ScheduleKeyDeletion` to principals other than the operator break-glass identity.
- CloudTrail MUST capture every `kms:Sign` event with the correlation `RequestId`.
- See `AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` for the parameterised template.

## 4. RemoteSigner fail-closed behaviour

Production `RemoteSignerClient::new` uses `UnimplementedTransport`:

```rust
pub struct UnimplementedTransport;

#[async_trait]
impl AwsKmsTransport for UnimplementedTransport {
    async fn get_public_key(&self, _: &str) -> Result<(Vec<u8>, Option<String>)> {
        unimplemented!("UnimplementedTransport: production code path; wire AwsKmsTransport via aws-kms-transport feature")
    }
    async fn sign(&self, _: &str, _: &[u8]) -> Result<(Vec<u8>, Option<String>)> {
        unimplemented!("UnimplementedTransport: production code path; wire AwsKmsTransport via aws-kms-transport feature")
    }
}
```

Until the operator wires a real transport (feature-gated) the production build
fails closed at every call site — there is no fallback to in-process key signing.

### 4.1 3-defence-in-depth on mainnet

| Defence | Implementation | What it catches |
|---|---|---|
| Startup `validate_signer_backend` | `src/execution/config.rs` `validate_signer_backend` | Mainnet refuses `EXECUTOR_PRIVATE_KEY`; refuses `LocalDevSigner` mode; refuses Mock provider at boot. |
| Runtime `build_signer_for_state` | `src/execution/signer.rs` | Mainnet refuses to construct a signer that resolves to a `LocalDevSigner`. |
| `LocalDevSigner` runtime guard | `src/execution/signer.rs::LocalDevSigner::sign_prehash` | Even if both startup checks are bypassed, the local signer's sign call refuses on mainnet at runtime. |

Metric: `local_signer_on_mainnet_refused_total`. **MUST be 0 at preflight.**
If non-zero: defence-in-depth fire = potential compromise. Investigate immediately.

## 5. Executor health endpoint (v2)

Implementation: `src/api/executor_health_v2.rs`. Spec:
`EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md`.

### 5.1 Surface

`GET /executor/health/v2` returns JSON with:

- `overall_status` ∈ {`ok`, `degraded`, `unhealthy`}.
- `signer.signer_mode` ∈ {`local`, `remote`, `mock`} — **MUST be `"remote"` on mainnet.**
- `signer.remote_signer_configured` — **MUST be true on mainnet.**
- `signer.signer_address` — MUST match expected production EOA (derived offline from `kms:GetPublicKey`).
- `signer.signer_last_health_check_at_ms`.
- `signer.signer_health_check_status`.
- `signer.last_signer_error_code`.
- `signer.local_signer_on_mainnet_refused_total` — **MUST be 0.**
- `live_provider_config.econ.*` — **ALL MUST be true on mainnet.**
- `live_provider_config.last_policy_data_failure` (singleton).
- `chain_state.last_seen.be_balance_floor_wei` — must NOT be below operator floor.
- `chain_state.last_seen.gas_price_wei`.
- `chain_state.last_seen.block_number`.
- `r5.drift_observed_total` — **MUST be 0.**
- `r5.drift_last_seen_at_ms`.
- `intent_tracking.not_tracked_yet[]` — **MUST be empty.**

### 5.2 Operator's preflight oracle

`/executor/health/v2` is the single endpoint the operator queries to gate
mainnet activation. Every field listed in §5.1 has a corresponding GO / RED
condition in `MAINNET_GO_NO_GO_CRITERIA.md`.

## 6. Metrics / observability

Implementation: `src/options/broadcast_observability.rs` + Prometheus
exposition via `src/api/routes.rs::metrics_handler`.

### 6.1 Mandatory zero counters at preflight

```
policy_rejected_total{*} == 0
signer_attempted_total{*} == 0          (no broadcast at preflight)
signer_success_total{*} == 0
signer_denied_total{*} == 0
local_signer_on_mainnet_refused_total{*} == 0
policy_data_failures_total{*} == 0
fm_v2_decode_failures_total{*} == 0
fm_v2_rpc_failures_total{*} == 0
r5_drift_observed_total{*} == 0
econ_data_available_*_total{*} == 0    (no broadcast at preflight)
```

### 6.2 Cardinality policy

- No high-cardinality labels (no per-tx-hash, no per-user-address label).
- Provider name labels (`provider="aws_kms"`, etc.) acceptable.
- Block number labels NEVER.
- See `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` for the full label catalogue.

### 6.3 RPC failure metrics

Implementation: `fm_v2_rpc_failures_total{rpc_method, error_kind}`.
Per-method buckets: `eth_call`, `eth_getBalance`, `eth_chainId`, etc.

### 6.4 CloudTrail RequestId

Backend captures `RequestId` from every AWS KMS response (header
`x-amzn-requestid` or `x-amzn-RequestId`). Sanitiser: 5-step
(empty / whitespace / control chars / URL-shape / length-cap rejection →
synthetic UUID fallback). Metric: `aws_kms_request_id_synthetic_fallback_total`.

## 7. Transaction visibility endpoints

Implementation: `src/api/routes.rs` (executor scope).

| Endpoint | Returns | Notes |
|---|---|---|
| `GET /executor/transactions/by-intent/{intent_id}` | latest transactions per intent | fixed in `OPTION-EXECUTION-TX-VISIBILITY-FIX` |
| `GET /executor/transactions/list` | recent transactions with filters | extended in `BACKEND-EXECUTOR-TRANSACTIONS-LIST-EXTEND` |
| `GET /executor/health/v2` | broadcast posture (§5) | |
| `GET /metrics` | Prometheus exposition (§6) | |
| `GET /health` | liveness | |
| `GET /ready` | readiness | |

## 8. Event indexer

Implementation: `src/options/event_indexer.rs`. Indexes:

- `Filled` (orderbook fill);
- `RfqFilled` (RFQ fill);
- `BadNonce` (`0x4bd574ec`);
- `InvalidSignature` (`0x8baa579f`);
- `RebateFunded` / `RebatePaid` (rebates path, INACTIVE at launch).

Mainnet posture: starts at deployment block of OME; backfills forward only;
reconciles against execution intents via `execution_intent_id` correlation key.

## 9. Nonce sync

Implementation: `src/nonce_sync/*`. Default `OPTION_NONCE_SYNC_ENABLED=true`
for mainnet (per Sepolia rehearsal evidence in
`RFQ_SMOKE_NONCE_SYNC_REMEDIATION.md`).

Sync triggers:

- On `BadNonce()` revert: drain pending intents until reorganised, re-broadcast.
- On startup: fetch `eth_getTransactionCount(BE, "pending")`; gate broadcast until in sync.
- On periodic heartbeat (default 15 s).

## 10. Confirmation worker

Implementation: `src/options/confirmation_worker.rs`. Tracks every broadcast
transaction:

- Pending: tx submitted, no receipt yet.
- Confirmed: tx receipt with status 1.
- Reverted: tx receipt with status 0; ingest revert reason.
- Stuck: timeout exceeded; eligible for replacement.

Default `OPTION_CONFIRMATION_WORKER_ENABLED=true` for mainnet.

## 11. R5 drift

`R5 = CV.balances(PFV, asset) - (PFV.feeBalance + PFV.rebateReserve)`.

Implementation: `src/fees/onchain_summary.rs` + `vault_observability.rs`.
The backend observes R5 every executor cycle. Any non-zero observation
increments `r5_drift_observed_total{asset}`. Any non-zero counter → preflight
RED → operator must investigate.

Sepolia rehearsal: R5 drift = 0 across 2 trades + 7 GOV-G tx + nonce-sync
remediation (cumulative).

## 12. Known security boundaries

| Boundary | Description |
|---|---|
| Backend NEVER holds raw private key in production | `EXECUTOR_PRIVATE_KEY` refused on mainnet; signer is remote (KMS) only. |
| Backend NEVER mints / withdraws / transfers funds via owner authority | BE's only chain authority is `NEW_OME.isExecutor(BE) = true`. No ownership / guardian / proposer / executor (Timelock) / fee-recipient / rebate-funding-account / treasury role. |
| Backend NEVER bypasses broadcast policy | `should_broadcast` is the single gate; no override flag. |
| Mainnet-specific config flags refused | `EXECUTOR_PRIVATE_KEY`, `LocalDevSigner`, Mock provider all refused at startup + runtime. |
| Admin token surface | `src/admin.rs` uses Bearer token; **F-H1 mainnet blocker** (token currently in browser sessionStorage; closure via V2G-W3 SSR proxy). |

## 13. Open operator dependencies

| Item | Owner | Source |
|---|---|---|
| Production RPC URL | Operator | `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` |
| AWS account ID / KMS key ID / IAM role | Operator (AWS provisioning) | `AWS_KMS_OPERATOR_SETUP_PACK.md` |
| Production EXECUTOR_FROM_ADDRESS | Operator (derived offline from `kms:GetPublicKey`) | `MAINNET_KMS_VENDOR_SELECTION_DECISION.md` |
| CloudTrail trail ARN | Operator | `AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` |
| Prometheus scrape config | Operator | `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` |
| Alert thresholds | Operator | `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` |
| Production EVM addresses (OME / PFV / FM_V2 / CV / RG / OracleRouter / InsuranceFund) | sol team (MAINNET-DEPLOYMENT) | `MAINNET_MANIFEST_TODO_INVENTORY.md` |

## 14. Open auditor questions on backend scope

- **BQ-1** Is the `UnimplementedTransport` fail-closed posture (production `RemoteSignerClient::new`) the auditor's recommended default, or does the auditor suggest a hard panic instead?
- **BQ-2** Is the 3-defence-in-depth on mainnet (`validate_signer_backend` + `build_signer_for_state` + `LocalDevSigner` runtime guard) sufficient, or does the auditor recommend a 4th?
- **BQ-3** Is the cardinality policy (no per-tx-hash, no per-user-address) sufficient to prevent metric explosion under adversarial load?
- **BQ-4** Does the operator's broadcast policy gate need additional read-only RPC checks beyond the 8 planned `should_broadcast` data sources?
- **BQ-5** Is the 5-step `RequestId` sanitiser sufficient, or does the auditor recommend additional defences against CloudTrail-correlation evasion?
- **BQ-6** Is the `aws-kms-transport` Cargo feature gating model the auditor's recommended posture for vendor-agnostic signer plug-ability, or does the auditor recommend always-compiled with runtime feature flag?
- **BQ-7** Is the event indexer's reconciliation key (`execution_intent_id`) robust against re-orgs, or does the auditor recommend a block-hash-derived key?
- **BQ-8** Is the admin token Bearer posture (`src/admin.rs`) acceptable behind the V2G-W3 SSR proxy, or does the auditor recommend an OIDC-token-exchange flow at the proxy boundary?

## 15. Cross-links

- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_EXT_KICKOFF_FINAL.md` — kickoff finalisation
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_CONTRACT_SCOPE_FINAL.md` — contract scope
- `~/DEOPT/deopt-v2-frontend/docs/MAINNET_AUDIT_FRONTEND_ADMIN_SCOPE_FINAL.md` — frontend scope
- `~/DEOPT/deopt-v2-sol/docs/MAINNET_AUDIT_RISK_REGISTER_FINAL.md` — risk register
- `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md`
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`
- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`
- `BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md`
- `BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`
- `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`
- `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md`
- `MAINNET_GO_NO_GO_CRITERIA.md`
- `MAINNET_NEXT_SAFE_MILESTONES.md`
- `INTERNAL_AUDIT_FINDINGS_V2G_AUDIT0.md`
- `AWS_KMS_OPERATOR_SETUP_PACK.md` + 4 siblings

**End of mainnet audit backend scope (FINAL).**
