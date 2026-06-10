# Mainnet signer vendor selection matrix

**Posture:** DESIGN / DOC ONLY. No source code modified. No `.env` edited.
No vendor account created. No KMS/HSM/MPC key created. No credentials
recorded. No private custody roster information disclosed.
**Closes milestone (in part):** `MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors:**
- `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §1` — Q-CD-5
  **Pattern C** decided (signer microservice), vendor sub-decision
  **OPEN**.
- `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology + adapter
  trait.
- `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md` — current
  `RemoteSigner` / `LocalDevSigner` / `RemoteSignerClient` shape and
  `SignerError::code()` taxonomy.
- `MAINNET_CUSTODY_POLICY.md §6 + §7` — principle BE-5, §6.6 precheck,
  §7.4 RemoteSigner pattern.

## 0. Hard rules (this doc)

```text
no vendor account creation              ✅
no real KMS/HSM/MPC key creation         ✅
no provider credential output            ✅
no .env edit                             ✅
no chain tx                              ✅
no canary broadcast                      ✅
no private custody roster disclosure     ✅
no guessed mainnet executor address      ✅
```

## 1. Decision frame

* **Pattern C is fixed.** The backend never holds raw private key
  material on mainnet (custody policy §6 BE-5; `ExecutionConfig::
  validate_signer_backend` runtime guard at `src/execution/config.rs:197`
  refuses `LocalDev` on `chain_id=8453`; `LocalDevSigner::
  sign_option_execution_tx` defence-in-depth at
  `src/execution/remote_signer.rs:283`).
* **Q-CD-5 vendor sub-decision is still OPEN.** This doc compares the
  realistic categories and recommends a shortlist; the operator
  resolves the specific vendor in an offline binder (per
  `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §3.5`).
* **The KMS/HSM/MPC sits BEHIND the signer microservice**, not in
  front of the backend. The signer service is the request validator;
  the cryptographic provider is its dependency. The vendor decision
  here is for the provider only.
* **Two EOAs** per Q-CD-6: `OPTION_BACKEND_EXECUTOR` (provisioned at
  launch) and `PERP_BACKEND_EXECUTOR` (deferred). Both flow through the
  same Pattern C topology.

## 2. Criteria

| # | Criterion | Why it matters |
|---|---|---|
| C1 | Ethereum secp256k1 native | Required to produce EIP-1559 signatures recoverable to a stable 0x-address. |
| C2 | EIP-155 transaction signing | Need the recovered signature to chain-bind via `v ∈ {chainId*2+35, chainId*2+36}`; some vendors require the backend to compute the prehash + concat. |
| C3 | EIP-712 typed-data support | Not strictly required for tx signing (we compute keccak prehash backend-side), but useful for some auxiliary flows. |
| C4 | Key non-exportability | BE-5 hard rule: the key MUST be created and live entirely inside the provider's protected store; the raw key bytes MUST never be retrievable. |
| C5 | Per-request audit logs | Custody policy §6.7 + design doc §3.2 — every sign request needs a vendor-issued log id (`kms_request_id`) for forensic correlation. |
| C6 | IAM / service authentication | mTLS or short-lived federated tokens preferred (custody §7.6). Long-lived API keys are acceptable only with rotation + Secret Manager wrap. |
| C7 | Rate limiting + quotas | The signer microservice must survive a flow burst; vendor side per-key rate limit + observable. |
| C8 | Sign latency (p50 / p99) | Affects user-visible broadcast latency. Target p99 ≤ 250 ms. |
| C9 | Regional availability (EU) | Custody §7.1 requires data residency aligned with operator jurisdiction. EU-Frankfurt / EU-West / EU-Stockholm acceptable. |
| C10 | Key rotation primitive | `rotate-key` or `disable + create-new`. The rotation runbook needs at least one provider-supported atomic step. |
| C11 | Emergency disable | Single-call key disable; ideally enforceable from a non-signer IAM role (so a compromised signer node can't re-enable itself). |
| C12 | Terraform / API automation | Greenfield-deployable; the signer service infra is IaC-defined per `MAINNET_BE_SIGNER_SERVICE_DESIGN.md §10`. |
| C13 | Cost + operational complexity | Sign-request unit cost + monthly fixed + on-call burden. |
| C14 | Vendor lock-in | Is the integration surface portable to a different KMS without a full signer-service rewrite? |
| C15 | SOC2 / ISO / compliance posture | Operator's auditor cares about the provider's attested controls. SOC2 Type 2 minimum; ISO 27001 + FIPS 140-2 L3 strong plus. |
| C16 | Integration complexity with current `RemoteSignerClient` | Effort to wire the vendor's SDK into the existing transport surface (`SignerTransport` trait at `src/execution/remote_signer.rs:334`). |
| C17 | Operational maturity (years deployed at scale, public incident track record) | Conservative bias toward "boring tech" — avoid newer offerings unless they cover an unmet need. |
| C18 | Incident response support | 24/7 channel, defined response SLA, public PSIRT. |

## 3. Vendor categories

> NOTE: every cell below is a generalised assessment of the public
> product surface as of the design pass. Capabilities change; the
> operator MUST verify each "yes" against the vendor's then-current
> documentation before commitment.

### 3.1 AWS KMS (asymmetric, ECC_SECG_P256K1)

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes — `KeySpec=ECC_SECG_P256K1`, `SignatureAlgorithm=ECDSA_SHA_256`. |
| C2 EIP-155 tx | Yes (backend computes prehash, AWS signs, backend assembles + extracts v via recovery). |
| C3 EIP-712 | Yes (backend computes typed-data digest, AWS signs). |
| C4 non-exportable | Yes — `Origin=AWS_KMS`. |
| C5 audit | Yes — CloudTrail per-request log. |
| C6 IAM | Yes — IAM role + STS short-lived creds; mTLS fronted via VPC-PrivateLink. |
| C7 rate limit | Yes — per-key + per-region; raises via support ticket. |
| C8 latency | p50 ~ 30 ms, p99 ~ 100-180 ms (same-region). |
| C9 EU regions | eu-central-1 / eu-west-1 / eu-west-2 / eu-west-3 / eu-north-1. |
| C10 rotation | Manual: `CreateKey` + alias swap. KMS does NOT auto-rotate asymmetric keys. |
| C11 disable | `DisableKey` API; scoped by IAM. |
| C12 IaC | Terraform `aws_kms_key` + `aws_kms_alias`. |
| C13 cost | $1/key/month + $0.03 per 10k signs. Low. |
| C14 lock-in | Medium. Integration is small (just the `Sign` API) but lock-in to AWS account. |
| C15 compliance | SOC2 / ISO 27001 / PCI / FIPS 140-2 L3 (CloudHSM-backed). |
| C16 integration | Low. AWS SDK Rust crate (`aws-sdk-kms`); maps 1:1 to `SignerTransport::send_sign_request`. |
| C17 maturity | High (KMS since 2014; secp256k1 from 2020). |
| C18 incident | AWS Support (Enterprise plan recommended for production). |

### 3.2 AWS CloudHSM (single-tenant FIPS 140-2 L3)

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes (PKCS#11). |
| C2 EIP-155 tx | Yes (backend prehash + assemble). |
| C3 EIP-712 | Yes. |
| C4 non-exportable | Yes — dedicated single-tenant HSM. |
| C5 audit | CloudHSM audit log + CloudTrail wrap. |
| C6 IAM | mTLS to HSM endpoint; PKCS#11 user/role. |
| C7 rate limit | Hardware-bounded; cluster scales horizontally. |
| C8 latency | p99 ~ 50-150 ms within the same VPC. |
| C9 EU regions | eu-central-1 / eu-west-1 / eu-west-2. |
| C10 rotation | Same as KMS pattern — provision new, swap, disable old. |
| C11 disable | HSM user permissions + IAM. |
| C12 IaC | Terraform `aws_cloudhsm_v2_cluster`. |
| C13 cost | ~$1.45/hr/HSM ≈ $1,050/month/cluster. High. |
| C14 lock-in | Low (PKCS#11 portable to other HSM vendors). |
| C15 compliance | FIPS 140-2 L3 attested. |
| C16 integration | Medium (PKCS#11 driver + connection management). |
| C17 maturity | High. |
| C18 incident | AWS Support. |

### 3.3 GCP Cloud KMS (HSM-backed, EC_SIGN_SECP256K1_SHA256)

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes — `EC_SIGN_SECP256K1_SHA256`. |
| C2 EIP-155 tx | Yes (backend prehash + assemble). |
| C3 EIP-712 | Yes. |
| C4 non-exportable | Yes (HSM protection level). |
| C5 audit | Cloud Audit Logs (Admin + Data Access). |
| C6 IAM | Workload Identity Federation + mTLS via Private Service Connect. |
| C7 rate limit | Per-key + per-project; quota-managed. |
| C8 latency | p50 ~ 40 ms, p99 ~ 150-200 ms. |
| C9 EU regions | europe-west1 / europe-west3 / europe-west4 / europe-north1. |
| C10 rotation | Manual key version creation; primary version switch via single API call. Cleanest in the cloud-KMS category. |
| C11 disable | `DestroyCryptoKeyVersion` (30-day grace) + IAM. |
| C12 IaC | Terraform `google_kms_crypto_key`. |
| C13 cost | $1-3/key/month + $0.03 per 10k signs. Low. |
| C14 lock-in | Medium. |
| C15 compliance | SOC2 / ISO 27001 / FedRAMP-High / FIPS 140-2 L3. |
| C16 integration | Low (`google-cloud-kms` Rust crate community-maintained; or REST + service-account JWT). |
| C17 maturity | High. |
| C18 incident | Google Cloud Support. |

### 3.4 GCP Cloud HSM (single-tenant)

Similar profile to AWS CloudHSM. C13 cost ~$1.50/hr; C9 europe-west4
only (region-restricted). Otherwise comparable to GCP Cloud KMS in
operational shape.

### 3.5 Azure Key Vault (Premium SKU, EC-P256K)

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes — `EC-P256K`. |
| C2 EIP-155 tx | Yes (backend prehash + assemble). |
| C3 EIP-712 | Yes. |
| C4 non-exportable | Yes (Premium HSM-protected). |
| C5 audit | Azure Monitor + Sentinel. |
| C6 IAM | Azure AD + managed identity; mTLS via Private Endpoint. |
| C7 rate limit | Per-vault throughput cap; relatively low. |
| C8 latency | p50 ~ 80 ms, p99 ~ 200-400 ms. Higher than AWS/GCP. |
| C9 EU regions | West Europe / North Europe / Germany West Central / Sweden Central. |
| C10 rotation | Key version policy; auto-rotate supported. |
| C11 disable | `Update-AzKeyVaultKey -Enable $false`. |
| C12 IaC | Terraform `azurerm_key_vault_key`. |
| C13 cost | $1/key/month + $0.15 per 10k signs. Slightly higher than AWS/GCP. |
| C14 lock-in | Medium. |
| C15 compliance | SOC2 / ISO 27001 / FedRAMP / FIPS 140-2 L3. |
| C16 integration | Medium (Azure SDK Rust crate is preview-quality; may want HTTPS+JWT instead). |
| C17 maturity | High. |
| C18 incident | Microsoft Support. |

### 3.6 Azure Managed HSM (single-tenant pool)

Similar profile to Azure Key Vault Premium but with single-tenant
isolation. Higher cost (~$3.50/hr ≈ $2,500/month/pool minimum). C16
integration parity with Key Vault Premium.

### 3.7 Fireblocks

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes (native EVM). |
| C2 EIP-155 tx | Yes — provides raw tx signing API. |
| C3 EIP-712 | Yes (typed-data signing API). |
| C4 non-exportable | Yes (MPC + HSM combination). |
| C5 audit | Per-request + transaction policy engine logs. |
| C6 IAM | API key + signing key pair on caller side; mTLS for whitelisted IPs. |
| C7 rate limit | Per-workspace; quotas published. |
| C8 latency | p50 ~ 300 ms, p99 ~ 1-2 s (MPC ceremony + policy engine). |
| C9 EU regions | EU multi-region. |
| C10 rotation | Vault account rotation (asset-by-asset); built-in workflow. |
| C11 disable | Policy-engine kill switch + workspace freeze. |
| C12 IaC | API-based; some Terraform community providers. |
| C13 cost | High (vault SaaS + per-transaction). |
| C14 lock-in | High — Fireblocks-specific concepts (vaults, asset wallets, policy engine). |
| C15 compliance | SOC2 / ISO 27001 / ISO 27017 / ISO 27018. |
| C16 integration | Medium-High — needs Fireblocks SDK + policy mapping; the existing `SignerTransport` trait works but the request shape needs adaptation. |
| C17 maturity | High in custody domain. |
| C18 incident | Dedicated Fireblocks support tier. |

### 3.8 Turnkey

| Criterion | Value |
|---|---|
| C1 secp256k1 | Yes. |
| C2 EIP-155 tx | Yes. |
| C3 EIP-712 | Yes. |
| C4 non-exportable | Yes — enclave-based (AWS Nitro). |
| C5 audit | Per-request log + policy decision log. |
| C6 IAM | Public-key authenticated API requests. |
| C7 rate limit | Per-org; published. |
| C8 latency | p50 ~ 200 ms, p99 ~ 500 ms - 1 s. |
| C9 EU regions | US-primary; EU residency in development. **Verify before commit.** |
| C10 rotation | Per-key activity; manual swap. |
| C11 disable | Policy + key delete. |
| C12 IaC | API-defined; SDK Rust available. |
| C13 cost | Pay-per-signature; mid-tier. |
| C14 lock-in | Medium — Turnkey-specific policy model. |
| C15 compliance | SOC2 Type 2; pursuing ISO. |
| C16 integration | Low-Medium (good Rust SDK). |
| C17 maturity | Newer (founded 2022); growing. |
| C18 incident | Direct engineering support. |

### 3.9 Generic MPC provider category (Lit, Coinbase Cloud, etc.)

* Threshold-signing MPC providers.
* Strong on multi-party security model.
* Weak on latency (multi-party signing round-trips) and operational
  maturity at small org scale.
* Strong on key non-extraction since no party ever holds the full key.
* Variable cost.
* Lock-in is high — protocol-specific.
* Suitable when threshold-of-N corporate signing is the core
  requirement; for a single backend EOA, conventional KMS is simpler.

### 3.10 Hardware / offline fallback category

* YubiHSM 2 / SoftHSM (PKCS#11).
* Strong key isolation; cheap.
* Operational burden is high — physical device handling, on-call HSM
  replacement, no managed audit trail.
* Suitable as a **conservative offline fallback** for break-glass
  signing of one-off operator tx (e.g., emergency
  `setExecutor` rotation packet signed in cold storage), NOT as the
  primary backend signer.

## 4. Recommended shortlist

The vendor decision is OPERATOR-led; the matrix above is the input.
This section is a Backend + Security recommendation for the shortlist.

### 4.1 Cloud-native path: **AWS KMS (asymmetric ECC_SECG_P256K1)**

* Lowest integration friction with the current `RemoteSignerClient` +
  `SignerTransport`.
* Best p99 latency in the cloud-KMS tier.
* Mature secp256k1 support.
* EU regions cover residency.
* Lowest unit cost.
* Operator's auditor likely already trusts AWS controls.

Trade-offs: no auto-rotation for asymmetric keys (manual rotation runbook
required); AWS lock-in is medium.

### 4.2 MPC / platform path: **Turnkey**

* Pure-play Ethereum signing service with strong policy engine.
* Good Rust SDK (low integration cost).
* Enclave-backed non-extraction.
* Per-key policy + per-org audit out of the box.

Trade-offs: newer (lower operational track record); EU residency
status MUST be confirmed before commit; per-sign cost higher than
cloud KMS at scale.

### 4.3 Conservative fallback: **AWS CloudHSM (FIPS 140-2 L3 single-tenant)**

* Strong story for auditor when the operator's compliance program
  requires single-tenant HSM.
* PKCS#11-portable (less vendor lock-in long-term).

Trade-offs: 10x cost of KMS; higher operational burden; latency p99
slightly worse than AWS KMS.

## 5. Decisional unknowns (operator must resolve offline)

| Unknown | Why it matters |
|---|---|
| Vendor name | Resolves Q-CD-5 vendor sub-decision. |
| Regional jurisdiction | Resolves Q-CD-14 (custody region). |
| Cost ceiling | Drives KMS vs CloudHSM trade-off. |
| Auditor preference | Some auditors require single-tenant HSM. |
| In-house ops capability | Determines whether a managed offering (KMS / Fireblocks / Turnkey) vs a self-managed HSM is appropriate. |

Operator records the resolved vendor name in the **offline binder**
(per `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §3.5`). This
matrix is updated only with the public-safe outcome
("`MAINNET-KMS-VENDOR-SELECTION` → CHOSEN: cloud-native-KMS"); the
vendor name itself does not need to land in tracked docs.

## 6. Anti-criteria

The matrix exists to RULE OUT, not just rank. A vendor is disqualified
if any of:

* Cannot produce a stable secp256k1 public key.
* Cannot produce raw-bytes ECDSA signature recoverable to a stable
  EVM address.
* Cannot guarantee key non-extraction.
* No EU residency option for operator jurisdiction.
* No emergency disable.
* No per-request audit log id.
* No public SOC2 / ISO / equivalent attestation.

All shortlisted vendors in §4 pass the anti-criteria.

## 7. Next steps (this milestone)

1. Operator + Security review §3 and §4.
2. Operator resolves vendor name in offline binder.
3. Operator records public-safe outcome in
   `MAINNET_KMS_VENDOR_SELECTION_RESULT.md` (future doc, single-line
   posture) — **NOT in this matrix**, which stays neutral.
4. Backend track proceeds to
   `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` (§3 of this
   milestone — done) and `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_NEXT_TASK.md`
   (§5 of this milestone — done).
5. Staging rehearsal proceeds per
   `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.
6. Rotation + incident response readiness pinned by
   `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md`.

## 8. Cross-links

* `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology + the
  adapter-trait contract this matrix is selecting a provider FOR.
* `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md` — current
  Rust-side surface (`RemoteSigner` trait + `RemoteSignerClient` +
  `SignerTransport`).
* `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md` — Q-CD-5 decision
  context.
* `MAINNET_CUSTODY_POLICY.md §6.7 + §7.4` — custody rules this matrix
  must honor.
