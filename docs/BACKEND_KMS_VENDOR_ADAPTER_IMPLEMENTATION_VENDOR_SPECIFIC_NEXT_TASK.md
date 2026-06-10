# BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION — vendor-specific next-task prompt

**Posture:** DOC ONLY (this file is the copy-paste prompt for the
next implementation milestone). No source code modified here. No
`.env` edited. No vendor credentials. No KMS/HSM/MPC key creation.

> **Addendum (2026-06-10):** the §2.1 default path SHIPPED as
> `BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-AWS-KMS`. The
> `AwsKmsSignerProvider` + mockable `AwsKmsTransport` + 27 unit + 6
> env tests live at `src/execution/signer_adapters/aws_kms.rs`. The
> §2.2 alternative-vendor override block remains usable if a future
> rotation cycle picks a different vendor — substitute the
> placeholders accordingly. See
> `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md`.
**Anchors:**
- `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — preferred path
  `AWS KMS asymmetric secp256k1`; fallback A `AWS CloudHSM`; fallback
  B `Turnkey`; operator commercial sign-off OPEN.
- `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter
  contract (16 named tests; SignerError mapping; redaction).
- `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`
  — pluggable shape shipped at `src/execution/signer_adapters.rs`.
- `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal phases
  this implementation feeds.

---

## How to use this doc

1. Confirm operator commercial / legal sign-off on the preferred
   vendor from
   `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md §6`. If the
   operator sticks with the technical recommendation, use the §2.1
   prompt below verbatim. If the operator selects an alternative
   vendor from the matrix, use the §2.2 override block to substitute
   the vendor name; the rest of the prompt stays identical because
   the `PluggableSignerProvider` shape is vendor-neutral.

2. Copy the chosen prompt block into the next milestone command.

3. The next milestone implements one `<Vendor>SignerProvider:
   PluggableSignerProvider`. No live broadcast. No `.env` edit. No
   real credentials. No real KMS key creation.

---

## 1. Operator override block (only fill in if needed)

If the operator selects an alternative vendor at commercial sign-off,
substitute the following placeholders in the prompt below:

| Placeholder | Default (preferred) | Operator override |
|---|---|---|
| `<VENDOR>` | `AWS_KMS` | (operator-selected: e.g. `AWS_CLOUD_HSM` / `TURNKEY` / `GCP_KMS` / `AZURE_KEY_VAULT` / `FIREBLOCKS`) |
| `<VendorNameInCamel>` | `AwsKms` | (e.g. `AwsCloudHsm` / `Turnkey` / `GcpKms` / `AzureKeyVault` / `Fireblocks`) |
| `<vendor_provider_kind>` | `AwsKms` | (must match a `SignerProviderKind` enum variant at `src/execution/signer_adapters.rs:62`. New vendors require an enum extension via a separate small PR.) |
| `<vendor_sdk_crate>` | `aws-sdk-kms` | (e.g. PKCS#11 driver crate / Turnkey SDK / etc. **Verify the crate ships without bundling secrets in tests.**) |
| `<vendor_env_key_prefix>` | `AWS_KMS_` | Vendor-specific env prefix. NEVER stored in backend `.env` — lives in the signer microservice's secret store. |

If no override: use the defaults verbatim — the prompt below targets
the preferred path (`AWS_KMS`).

## 2. Copy-paste prompts

### 2.1 Preferred path (AWS KMS) — default

```text
Workspace root is ~/DEOPT.

Execute BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-AWS-KMS only.

This is a backend implementation milestone.
Do not create AWS account.
Do not create real KMS keys.
Do not edit `.env`.
Do not broadcast.
Do not send transactions.
Do not touch mainnet with a transaction.
Do not create Safe transactions.
Do not expose secrets.

Current state:

* Sepolia rehearsal arc is complete.
* Orderbook live smoke is closed.
* RFQ live smoke is closed.
* R5 drift remains 0.
* Backend broadcast policy gate is live-fed.
* Remote signer abstraction exists.
* Mainnet refuses EXECUTOR_PRIVATE_KEY.
* Mainnet refuses LocalDevSigner.
* Mainnet refuses Mock pluggable provider.
* /metrics is wired.
* /executor/health/v2 is complete with not_tracked_yet=[].
* /executor/transactions/:intent_id returns both legacy PERP and option execution transactions.
* /executor/transactions list endpoint returns both legacy PERP and option execution transactions.
* MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK is closed.
* BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-PLUGGABLE is closed.
* MAINNET-KMS-VENDOR-SELECTION is closed:
  * Preferred path: AWS KMS asymmetric secp256k1.
  * Fallback A: AWS CloudHSM.
  * Fallback B: Turnkey.
  * Operator commercial sign-off captured offline.
* PluggableSignerProvider trait exists at src/execution/signer_adapters.rs.
* SignerProviderKind::AwsKms enum variant exists.
* No real AWS account exists in tracked source.
* No real KMS key exists in tracked source.
* No production startup path uses the new adapter yet (RemoteSignerClient::new still uses UnimplementedTransport).
* 999 backend tests green.
* No mainnet action is authorised.
* No Sepolia live broadcast is authorised.
* No chain transaction is authorised.
* No Safe transaction is authorised.
* No .env edit is authorised.
* No real KMS/HSM/MPC key creation is authorised.

Goal:
Implement AwsKmsSignerProvider: PluggableSignerProvider per docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md.
The adapter MUST work against a mock AWS KMS surface in tests.
It MUST NOT add real credentials, real keys, or real AWS accounts.
The production RemoteSignerClient::new MUST continue to use UnimplementedTransport as its default.

Required Phase A — inspect:

1. Read:
   * src/execution/signer_adapters.rs (PluggableSignerProvider trait, VendorError, MockVendorSignerProvider, PluggableRemoteSignerTransport, validate_signature).
   * src/execution/remote_signer.rs (RemoteSignerClient, SignerError taxonomy, SignerTransport).
   * src/execution/config.rs (ExecutionConfig fields, validate_signer_backend mainnet refusal).
   * src/config/env.rs (BACKEND_REMOTE_SIGNER_PROVIDER env loader).
   * docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md §4 + §5 (config keys + error mapping table).
   * docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md (16 named tests + redaction).

Required Phase B — adapter module:

2. Create src/execution/signer_adapters/aws_kms.rs (split the existing
   single-file signer_adapters.rs into a small module tree: signer_adapters/mod.rs
   re-exports the existing types; signer_adapters/aws_kms.rs holds the new struct).
3. Define struct AwsKmsSignerProvider with fields:
   * expected_address: AccountId (configured EOA).
   * vendor_endpoint: String (signer microservice mTLS endpoint URL —
     NOT a direct AWS endpoint; the microservice fronts AWS).
   * timeout: Duration (from BACKEND_SIGNER_TIMEOUT_MS).
   * transport: Arc<dyn AwsKmsTransport> (for mock injection).
4. Define a thin inner trait AwsKmsTransport for transport injection:
   * send_get_public_key(&self, endpoint: &str) -> Future<(Sec1PublicKey, RequestId)>.
   * send_sign(&self, endpoint: &str, prehash: [u8;32], metadata: SignRequestMetadata) -> Future<(DerSignature, RequestId)>.
5. Define a MockAwsKmsTransport for tests.
6. Implement PluggableSignerProvider for AwsKmsSignerProvider:
   * provider_kind → SignerProviderKind::AwsKms.
   * derive_address → calls get_public_key + computes EVM address per
     keccak256(uncompressed_pub_key[1..])[12..].
   * sign_prehash → calls sign, decodes DER signature, recovers
     y_parity via secp256k1_ecdsa_recover_compact, builds
     RecoverableSignature, validates structurally via validate_signature,
     returns PluggableSignResult.

Required Phase C — error mapping:

7. Map AWS error responses onto VendorError per docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md §5.
8. Preserve the 80-char reason truncation in map_vendor_error.

Required Phase D — typed config:

9. Add typed-config field ExecutionConfig.backend_signer_timeout_ms: u32
   (default 2500; range 100..=30000) via src/config/env.rs.
10. Read env key BACKEND_SIGNER_TIMEOUT_MS.
11. NO new env keys for AWS credentials or key id — those live in the
    signer microservice's secret store. The backend never reads AWS_KMS_*
    env vars.

Required Phase E — integration:

12. Add RemoteSignerClient::with_aws_kms_provider(endpoint, expected_address, transport, timeout) constructor that:
    * Constructs AwsKmsSignerProvider.
    * Wraps in PluggableRemoteSignerTransport.
    * Calls RemoteSignerClient::with_transport.
13. Production RemoteSignerClient::new continues to use UnimplementedTransport.

Required Phase F — observability:

14. The existing broadcast call site at src/options/service.rs:1755 already
    records signer_attempt / success / denied counters. The new adapter
    MUST NOT bypass them; the mapping at map_vendor_error preserves the
    existing SignerError::code() taxonomy.

Required Phase G — tests:

15. Implement the 16 named tests from
    docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §6, plus:
    * adapter_derives_address_from_sec1_uncompressed_public_key.
    * adapter_recovers_y_parity_correctly.
    * adapter_handles_der_signature_canonical_form.
    * adapter_rejects_non_canonical_der_signature.
16. All tests use MockAwsKmsTransport; no live AWS calls.
17. Add config tests:
    * mainnet with backend_signer_provider=AwsKms + missing endpoint refuses startup.
    * mainnet with backend_signer_provider=AwsKms + valid endpoint passes startup.
    * BACKEND_SIGNER_TIMEOUT_MS=0 rejects at env load.
    * BACKEND_SIGNER_TIMEOUT_MS above ceiling rejects.

Required Phase H — docs:

18. Create BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md
    capturing the adapter design, error mapping table, timeout/retry
    policy, signature recovery flow, tests added, validation results.
19. Update docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md with a
    one-line "Adapter for AWS KMS: SHIPPED" addendum.
20. Update docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md with a
    one-line "AWS KMS adapter SHIPPED at <date>" addendum.
21. Update RUN_STATE.md with a closure paragraph.

Validation:

22. cargo fmt.
23. cargo clippy --all-targets --all-features -- -D warnings.
24. cargo test --all-targets --all-features --no-fail-fast.
25. git diff --check.
26. Confirm no .env edit.
27. Confirm no chain tx.
28. Confirm no backend broadcast.
29. Confirm no Safe tx.
30. Confirm no real AWS account creation.
31. Confirm no real KMS key creation.
32. Confirm no AWS credentials in tracked source.
33. Confirm UnimplementedTransport remains the default of RemoteSignerClient::new.
34. Confirm mainnet still refuses LocalDev + Mock providers.
35. Confirm no secrets printed.

Forbidden:

* no mainnet tx.
* no Sepolia live broadcast.
* no Safe tx.
* no governance mutation.
* no .env edit.
* no real KMS key creation.
* no real AWS account creation.
* no AWS credentials in source.
* no private key / admin token / RPC secret / DATABASE_URL / API key in output.
* no fallback path allowing mainnet local private key signing.
* no removal of UnimplementedTransport as RemoteSignerClient::new default.

Hard stops:

* stop if a real AWS account or real KMS key would be required.
* stop if implementation would require credentials.
* stop if implementation would require live broadcast.
* stop if implementation would require editing .env.
* stop if any secret would be printed.
* stop if RemoteSignerClient::new is altered to make the adapter the default before the rehearsal Phase 3 operator approval.
* stop if mainnet could select Mock provider.

Return final report grouped by:
workspace,
source/docs inspected,
adapter module path,
AwsKmsTransport trait surface,
signature recovery flow,
DER decode behavior,
SignerError mapping confirmed against decision doc §5,
timeout policy,
typed-config fields added,
observability hooks pinned,
tests added,
tests run,
docs touched,
RUN_STATE update,
files changed,
validations,
blockers,
next milestone recommendation.
```

### 2.2 Alternative vendor (override)

If operator commercial sign-off selects an alternative vendor, take
the §2.1 prompt and:

1. Substitute `AWS_KMS` → operator vendor (e.g. `TURNKEY`).
2. Substitute `AwsKms` → operator vendor in camel case (e.g.
   `Turnkey`).
3. Substitute `aws_kms` → matching `SignerProviderKind` enum variant
   string.
4. Substitute the vendor SDK crate name in Phase B (e.g. Turnkey Rust
   SDK).
5. Substitute the error-mapping table in Phase C with the vendor's
   error categories projected onto the same 8-variant `VendorError`
   taxonomy.
6. Substitute the Phase G adapter-specific tests
   (`adapter_derives_address_from_*`) with the vendor's signature
   serialisation specifics.
7. Everything else (config keys, mainnet guard, observability,
   redaction, hard stops) stays IDENTICAL — the pluggable shape was
   designed to be vendor-neutral at every layer except the SDK glue.

The result doc named in Phase H §18 becomes
`BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_<VENDOR>_RESULT.md`.

## 3. Preserved invariants (any vendor)

Regardless of which vendor the operator selects, the implementation
MUST preserve:

* `UnimplementedTransport` as `RemoteSignerClient::new`'s default —
  the new adapter only ships behind `with_transport` /
  `with_<vendor>_provider`. Production startup remains fail-closed
  until the operator-approved rehearsal Phase 3 cutover.
* `ExecutionConfig::validate_signer_backend` mainnet refusal of
  `LocalDev` mode + `Mock` provider — none of these can become a
  fallback under any failure mode.
* `LocalDevSigner` runtime guard at
  `src/execution/remote_signer.rs:283` — mainnet chain_id 8453
  refuses to sign defensively.
* `build_signer_for_state` guard at `src/options/service.rs:1465` —
  refuses LocalDev on mainnet at the broadcast call site.
* No fallback to local-key signing on remote signer failure on
  mainnet under ANY circumstance.

## 4. Cross-links

* `docs/MAINNET_KMS_VENDOR_SELECTION_DECISION.md` — preferred + fallback
  paths + operator questions + implementation consequences.
* `docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter
  contract.
* `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`
  — pluggable shape this prompt extends.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal flow.
* `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation +
  incident readiness.
