# MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION — next-task prompt

**Posture:** DOC ONLY (this file is the copy-paste prompt for the
operator-side rehearsal phase). No source code modified by this
document. No real AWS resources created. No transactions sent.

**Anchors:**
- `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
- `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy.
- `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
- `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail.
- `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — preflight + GO/NO-GO.
- `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §2.2` — Phase 2 spec.

---

## How to use this doc

Copy the prompt block in §1 into the next operator-milestone
command. The default (Variant A) is **no-broadcast**: AWS KMS
operations only, ZERO chain transactions on any network. The Sepolia
variant (Variant B) is OPT-IN and requires an explicit additional
authorisation in the milestone brief.

Substitute placeholders as instructed in
`docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md §1`. NEVER paste real
AWS account IDs, KMS key IDs, ARNs, or signer addresses into tracked
docs.

---

## 1. Copy-paste prompt — Variant A (default; no-broadcast)

```text
Workspace root is ~/DEOPT.

Execute MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION only.

This is an operator-side runbook execution milestone.
This is the NO-BROADCAST variant.

Do not send any chain transaction.
Do not send any Sepolia transaction.
Do not send any mainnet transaction.
Do not create a Safe transaction.
Do not edit `.env` in this repo (operator may edit production env
in the operator secret store — never in tracked files).
Do not print secrets.
Do not print real AWS account IDs.
Do not print real KMS key IDs / ARNs.
Do not print the production EVM signer address into tracked logs.

Current state:

* AWS-KMS-OPERATOR-SETUP-PACK is closed.
* AWS account / IAM roles / KMS key are operator-provisioned per the
  setup pack against the operator's chosen architecture (Option A
  direct OR Option B microservice).
* CloudTrail trail covers KMS data events for the signer key.
* Backend `cargo build --features aws-kms-transport` succeeds.
* Backend `RemoteSignerClient::new` STILL uses UnimplementedTransport.
  Operator promotion to production happens at Phase 3, not Phase 2.
* No mainnet broadcast is authorised.
* No Sepolia broadcast is authorised.
* No Safe transaction is authorised.

Goal:
Execute the no-broadcast AWS KMS rehearsal per
`docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md §6 R1-R14`. Verify that
the AWS KMS signer can sign a controlled test prehash + the
recovered address matches the configured EXECUTOR_FROM_ADDRESS + the
backend's CloudTrail correlation + observability surfaces line up
end-to-end. Do NOT send any chain transaction. Do NOT promote the
production RemoteSignerClient::new wiring.

Required Phase A — preflight:

1. Confirm `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` sections
   §1-§5 are ALL GREEN (P1-P43). Capture results in the operator
   binder.
2. Confirm operator authorisation per
   `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md §3.5` is on file.
3. Confirm the IAM role attached to the rehearsal runtime is
   `<SIGNER_RUNTIME_ROLE_NAME>` per
   `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md`.
4. Confirm CloudTrail data events are flowing for
   `<KMS_KEY_ARN>` (operator generates a no-op `DescribeKey` and
   observes the event landing within 5 minutes).

Required Phase B — GetPublicKey + offline address derivation:

5. From the rehearsal runtime, call `aws kms get-public-key
   --key-id <KMS_KEY_ID_OR_ALIAS>` (or equivalent SDK call). Capture
   the SPKI Blob to a local file in an operator-controlled
   workspace (NOT into the repo).
6. Parse the SPKI offline to extract the SEC1 uncompressed
   secp256k1 public key (65 bytes; `0x04` prefix).
7. Derive the EVM address: `keccak256(pubkey[1..])[12..]`,
   formatted as `0x<40-hex-lowercase>`.
8. Confirm the derived address matches `EXECUTOR_FROM_ADDRESS` in
   the rehearsal env. DO NOT log the production address to tracked
   files.
9. Capture the CloudTrail `GetPublicKey` event id from the
   `userIdentity.arn` query in
   `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md §5.1`. Confirm
   the event is attributed to `<SIGNER_RUNTIME_PRINCIPAL_ARN>`.

Required Phase C — Sign a controlled test prehash:

10. Compute `test_prehash = keccak256("deopt-no-broadcast-rehearsal")`.
11. From the rehearsal runtime, call `aws kms sign
    --key-id <KMS_KEY_ID_OR_ALIAS> --message
    <base64-of-test_prehash> --message-type DIGEST
    --signing-algorithm ECDSA_SHA_256`. Capture the DER signature.
12. Parse the DER signature offline into `(r, s)`.
13. Try `y_parity ∈ {0, 1}`; for each, recover the verifying key via
    secp256k1 and derive the EVM address. Confirm ONE of the two
    candidates recovers to `EXECUTOR_FROM_ADDRESS`.
14. Capture the CloudTrail `Sign` event for this call. Confirm the
    event id maps to the AWS SDK RequestId path the backend uses
    (`output.request_id()` per
    `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`).
15. Confirm NO chain transaction was sent by this step. NO
    `eth_sendRawTransaction`. NO Safe-tx. NO mainnet, NO Sepolia.

Required Phase D — wire backend health_check (no broadcast):

16. In a non-production rehearsal copy of the backend (operator
    workstation OR a sandbox EKS pod with the rehearsal IAM role
    attached), build with `cargo build --features aws-kms-transport`.
17. Construct an `AwsKmsSdkTransport` against the operator's
    rehearsal `aws_sdk_kms::Client`. Construct an
    `AwsKmsSignerProvider` with the rehearsal `key_id` +
    `expected_address`. Wrap in `PluggableRemoteSignerTransport` +
    `RemoteSignerClient::with_transport` per
    `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_AWS_KMS_RESULT.md
    §3`.
18. Call `RemoteSigner::health_check()`. Confirm Ok(SignerHealth)
    with the expected address.
19. DO NOT call `RemoteSigner::sign_option_execution_tx` — that
    would set up a broadcast attempt. Phase 2 is read-only against
    the signer.
20. Confirm `/executor/health/v2.signer.signer_mode == "remote"`,
    `signer.remote_signer_configured == true`,
    `signer.signer_address` matches the derived address,
    `signer.local_signer_on_mainnet_refused_total == 0`,
    `overall_status == "green"`.

Required Phase E — observability verification:

21. Capture a Prometheus scrape against `/metrics`. Confirm:
    * `signer_attempted_total{signer_kind="remote"}` and
      `signer_success_total{signer_kind="remote"}` are EITHER zero
      (no sign attempts from the broadcast pipeline yet) OR
      reflect ONLY the operator-controlled health_check probes.
    * `signer_denied_total` is empty.
    * `local_signer_on_mainnet_refused_total == 0`.
    * `policy_data_failures_total` is empty (Phase 5 of the
      staging plan exercises this; Phase 2 does not).
22. Capture a `/executor/health/v2` snapshot. Confirm
    `overall_status == "green"`. Snapshot stored in operator binder.
23. Capture the CloudTrail event id from Phase C and verify the
    backend log line (if any was emitted by step 18) carries the
    matching `kms_request_id` per
    `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md §6`.

Required Phase F — non-broadcast acceptance:

24. Confirm sections §6 R1-R14 of
    `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` are all GREEN.
25. Operator captures the rehearsal artefacts (CloudTrail event
    list, `/metrics` snapshot, `/executor/health/v2` snapshot,
    derived-address attestation) in the operator binder.
26. Operator publishes a public-safe `MAINNET_SIGNER_REHEARSAL_PHASE_2_RESULT.md`
    closure note (no secrets, no AWS IDs, no production address).

Validation:

27. Capture `cargo fmt --check` ok against the rehearsal copy.
28. Capture `cargo clippy --all-targets --all-features -- -D warnings`
    ok against the rehearsal copy.
29. Capture `cargo test --all-targets --all-features --no-fail-fast`
    green against the rehearsal copy.
30. Confirm no `.env` edit in this repo.
31. Confirm no mainnet tx.
32. Confirm no Sepolia tx.
33. Confirm no Safe tx.
34. Confirm no secrets / no real AWS account ID / no real KMS key
    ID / no real ARN / no production address printed into tracked
    files.
35. Confirm no real AWS KMS key creation by this milestone (the key
    was created by the operator BEFORE this milestone fires, per the
    setup pack).

Forbidden:

* no mainnet tx.
* no Sepolia live broadcast.
* no Safe tx.
* no governance / Timelock / ownership / guardian mutation.
* no rebate reserve allocation.
* no PFV withdrawal.
* no fund movement.
* no `.env` edit in this repo.
* no real production credentials printed.
* no real AWS account IDs, KMS key IDs, or ARNs in tracked docs.
* no private custody roster disclosure.
* no `RemoteSignerClient::new` modification — production default
  stays `UnimplementedTransport`.
* no fallback path allowing mainnet local-key signing.

Hard stops:

* stop if any chain transaction would be sent.
* stop if `.env` would be modified in this repo.
* stop if real credentials would be required in tracked code.
* stop if real AWS account IDs / KMS IDs / ARNs would land in
  tracked docs.
* stop if production EVM signer address would be written to tracked
  files.
* stop if the CloudTrail trail is not capturing data events.
* stop if any preflight check (§1-§5) is RED.

Return final report grouped by:
workspace,
preflight results (P1-P43),
GetPublicKey + address-derivation results,
Sign test prehash + recovery results,
health_check result,
observability snapshot summary,
CloudTrail correlation result,
NO-BROADCAST confirmation,
artefacts captured (paths in operator binder; no secret content),
RUN_STATE update (public-safe one-line closure),
files changed (should be empty for tracked repo; rehearsal artefacts
  in operator binder),
validations,
blockers,
next milestone recommendation.
```

---

## 2. Copy-paste prompt — Variant B (Sepolia canary; OPT-IN)

> **WARNING:** This variant performs ONE Sepolia broadcast through
> the AWS KMS adapter. It is OFF by default and MUST be explicitly
> authorised by the operator in the milestone brief.
>
> Substitute the line `THIS-AUTHORISATION-IS-ON-FILE: yes` only
> after capturing an offline authorisation from
> Security + Operator + Backend leads per
> `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`.
>
> Variant B does NOT broadcast on mainnet. Mainnet broadcast lives
> in Phase 7 of the staging plan and requires a SEPARATE authorised
> milestone.

```text
Workspace root is ~/DEOPT.

Execute MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION only.

THIS IS THE SEPOLIA CANARY VARIANT.
THIS-AUTHORISATION-IS-ON-FILE: <yes|no>

If `no` → ABORT. Use Variant A (no-broadcast) instead.
If `yes` → continue with the steps below.

Do not send any mainnet transaction.
Do not create a Safe transaction.
Do not edit `.env` in this repo.
Do not print secrets.
Do not print real AWS account IDs / KMS key IDs / ARNs / production
  signer address.

Current state:
[Same as Variant A current state, plus:]
* Sepolia rehearsal arc is closed (orderbook + RFQ smokes both
  shipped per RUN_STATE).
* Operator has provisioned a SEPOLIA-ONLY KMS key + IAM role
  distinct from the mainnet key. Sepolia funding is in place on the
  derived address.
* Operator + Security + Backend authorisation captured offline.

Goal:
Execute Variant A (steps Phase A-F) END-TO-END plus a SINGLE Sepolia
broadcast through the AWS KMS adapter. Validate that:
* should_broadcast approves before signer call.
* AWS KMS Sign succeeds with the real CloudTrail RequestId.
* Backend assembles + broadcasts the EIP-1559 tx on Sepolia.
* Confirmation worker observes mined_success within 30 minutes.
* /executor/health/v2 / /metrics / /executor/transactions surfaces
  the round-trip.
* No fallback to LocalDev.
* No mainnet contact.

Required Phases A-F:
[Same as Variant A Phase A-F; verify NO-BROADCAST against the
configured Sepolia adapter — i.e. Phase D `health_check` only.]

Required Phase G — Sepolia canary broadcast (ONE broadcast):

36. In the operator-controlled rehearsal backend (NOT production),
    flip `EXECUTOR_CHAIN_ID=84532` and
    `EXECUTOR_REAL_BROADCAST_ENABLED=true` in the rehearsal env (NOT
    tracked).
37. Build with `cargo build --features aws-kms-transport`.
38. Construct an option execution intent against a Sepolia test
    series; run through the broadcast pipeline.
39. Confirm:
    * `should_broadcast` approves.
    * `AwsKmsSdkTransport.sign_digest` returns a real DER signature.
    * `validate_signature` passes (low-s + curve-order).
    * `RemoteSignerClient` post-sign cross-check passes.
    * Sepolia RPC `eth_sendRawTransaction` returns a tx hash.
    * Confirmation worker observes mined_success within 30 minutes.
40. Confirm `/executor/transactions/<intent_id>` returns the row
    with `source: "option"` + `confirmation_status: "mined_success"`
    + the real CloudTrail-correlatable `kms_request_id`.
41. Confirm `/executor/health/v2.overall_status == "green"` after
    the broadcast.
42. Capture all artefacts in the operator binder.

Validation:
[Same as Variant A, plus:]
43. Confirm exactly ONE Sepolia tx was broadcast.
44. Confirm tx hash is publicly recorded.
45. Confirm NO mainnet contact.
46. Confirm NO Safe tx.
47. Confirm NO fallback to LocalDev.
48. Confirm `local_signer_on_mainnet_refused_total == 0` (no
    defence-in-depth fire — we're on Sepolia, not mainnet, but the
    counter must remain pinned at zero).

Forbidden:
[Same as Variant A, plus:]
* no mainnet broadcast.
* no Safe-tx on Sepolia.
* no more than ONE Sepolia broadcast — the canary is single-shot.

Hard stops:
[Same as Variant A, plus:]
* stop if the AWS KMS key used would be the mainnet key (must be
  Sepolia-only key).
* stop if more than one Sepolia broadcast is attempted.
* stop if `should_broadcast` is bypassed.
* stop if the backend would attempt to read CloudTrail directly (it
  should not — CloudTrail correlation is operator-side).

Return final report grouped by:
[Same as Variant A, plus:]
Sepolia broadcast tx hash (public),
confirmation outcome,
end-to-end CloudTrail RequestId correlation outcome.
```

---

## 3. Cross-links

* `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision +
  readiness summary.
* `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy.
* `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
* `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail + alerts.
* `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — preflight + GO/NO-GO.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase ladder.
* `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — feature
  flag + SDK integration.
* `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — RequestId
  extraction.
* `docs/BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia
  precedent for authorised cutover broadcasts.
