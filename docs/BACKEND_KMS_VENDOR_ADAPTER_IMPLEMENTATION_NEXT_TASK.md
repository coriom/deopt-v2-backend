# BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION — next-task prompt

**Posture:** DOC ONLY (this file is the copy-paste prompt for the
next milestone). No source code modified here. No `.env` edited. No
vendor credentials. No KMS/HSM/MPC key creation. No private custody
detail.

> **Addendum (2026-06-10):** the §3.2 vendor-agnostic path SHIPPED as
> `BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-PLUGGABLE`. The
> pluggable `PluggableSignerProvider` trait + `MockVendorSignerProvider`
> + 22-test pin live at `src/execution/signer_adapters.rs`. The §3.1
> vendor-specific path REMAINS the recommended next step once
> `MAINNET-KMS-VENDOR-SELECTION` resolves — use it to ship a concrete
> `AwsKmsSignerProvider` / `TurnkeySignerProvider` / etc. that
> implements the new trait. See
> `docs/BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`.
**Closes milestone (in part):** `MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors (for the implementation milestone):**
- `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract.
- `MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — vendor input.
- `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md` — current
  Rust surface (`RemoteSigner` / `LocalDevSigner` /
  `RemoteSignerClient` / `SignerTransport` /
  `UnimplementedTransport` / `SignerError`).
- `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health surface to wire.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert taxonomy.

---

## How to use this doc

Copy the prompt block in §3 into the next milestone command.

If the vendor selection from
`MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md §4` has resolved at the
time of execution (operator recorded the choice in the offline
binder), use the vendor-specific section (§3.1). If the choice is
still pending, use the vendor-agnostic section (§3.2) — it ships a
pluggable adapter shape that any of the shortlisted vendors can be
wired into with a small follow-on PR.

Both options ship the same `SignerTransport` integration shape, the
same test classes, and the same observability + redaction guarantees.

---

## 1. Common scope (both options)

The implementation milestone must:

* Add a new module under `src/execution/signer_adapters/` (new
  directory). The vendor-specific variant lives at
  `src/execution/signer_adapters/<vendor>.rs`; the vendor-agnostic
  variant lives at
  `src/execution/signer_adapters/pluggable.rs`.
* Implement `SignerTransport` from
  `src/execution/remote_signer.rs:334` for the adapter.
* Wire the adapter into `RemoteSignerClient::with_transport` at
  `RemoteSignerClient::new` site OR via a new constructor variant
  `RemoteSignerClient::with_vendor_transport(endpoint, expected_address)`.
* Preserve the existing mainnet startup guard
  (`ExecutionConfig::validate_startup` at `src/execution/config.rs:97`)
  + the runtime defence-in-depth at
  `build_signer_for_state` (`src/options/service.rs:1465`) +
  `LocalDevSigner::sign_option_execution_tx` mainnet refusal
  (`src/execution/remote_signer.rs:283`).
* Land the test classes enumerated in
  `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §6`.

## 2. Common non-goals (both options)

* No real KMS/HSM/MPC key creation.
* No vendor account creation.
* No real credentials in tracked source or test fixtures.
* No `.env` edit.
* No live broadcast.
* No mainnet transaction.
* No Safe transaction.
* No removal of `UnimplementedTransport` — it remains the default in
  `RemoteSignerClient::new` until the adapter is operator-verified
  in Phase 3 of `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md`.

## 3. Copy-paste prompts

### 3.1 Vendor-specific path

Use this prompt if `MAINNET-KMS-VENDOR-SELECTION` has resolved.
Substitute `<VENDOR>` with the operator-selected vendor name (e.g.
`AWS_KMS`, `GCP_KMS`, `TURNKEY`).

```text
Workspace root is ~/DEOPT.

Execute BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION only.

Current state:

* MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK is closed.
* MAINNET-KMS-VENDOR-SELECTION resolved: vendor = <VENDOR>.
* Vendor credentials live in operator secret store; not in repo.
* No vendor account or real key is created by this milestone.
* RemoteSigner trait exists.
* LocalDevSigner exists.
* RemoteSignerClient::with_transport exists (mock injection point).
* UnimplementedTransport is the default for RemoteSignerClient::new.
* Mainnet refuses LocalDev at startup + at runtime.
* /executor/health/v2 surfaces the full signer block.
* 970 backend tests green.
* No mainnet action is authorised.
* No Sepolia live broadcast is authorised.
* No chain transaction is authorised.
* No Safe transaction is authorised.
* No .env edit is authorised.
* No real KMS/HSM/MPC key creation is authorised.

Goal:
Implement the <VENDOR> SignerTransport adapter per
deopt-v2-backend/docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md.
Do NOT broadcast. Do NOT create vendor accounts. Do NOT add real
credentials. Use only mock providers in tests.

Required Phase A — inspect:

1. Read:
   * src/execution/remote_signer.rs
   * src/execution/config.rs
   * src/options/service.rs (build_signer_for_state, broadcast call
     site, observability hooks)
   * src/options/broadcast_observability.rs
   * src/api/executor_health_v2.rs
   * docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md
   * docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md
2. Confirm the SignerError taxonomy is unchanged; the adapter MUST
   map vendor errors onto the existing 20 variants.

Required Phase B — adapter module:

3. Create src/execution/signer_adapters/<vendor>.rs.
4. Define an adapter struct holding the vendor SDK client +
   endpoint config + expected_address.
5. Implement SignerTransport for the struct:
   * send_sign_request → vendor sign call → map response.
   * send_health_check → vendor derive_address + endpoint reach
     check.
6. Implement vendor → SignerError mapping per §3 of the requirements
   doc.
7. Implement signature recovery + address mismatch detection per
   §2.14 of the requirements doc.
8. Implement timeout per §2.10 (default 2500 ms; configurable via
   typed config).
9. Do NOT retry sign requests; retries on read-only health are OK.

Required Phase C — wire into RemoteSignerClient:

10. Add a constructor RemoteSignerClient::with_vendor_transport or a
    factory function that builds the adapter and injects it.
11. Production code does NOT touch this constructor until the
    rehearsal Phase 3 cutover (operator action; out of scope here).
12. UnimplementedTransport remains the default in
    RemoteSignerClient::new.

Required Phase D — typed config:

13. Add typed-config fields for any new env keys (e.g.
    BACKEND_SIGNER_TIMEOUT_MS, vendor-specific key id) via
    src/config/env.rs. Loader MUST validate at startup.
14. Do NOT edit .env.

Required Phase E — observability + health:

15. The existing broadcast call site at src/options/service.rs:1755
    already records signer_attempt / success / denied counters. The
    adapter must NOT bypass them. Pin via existing
    observability_signer_* integration tests + add one for
    address-mismatch routing through PostSignFromMismatch.

Required Phase F — tests:

16. Add the test classes from
    docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §6 (16 named
    tests). All use mock vendor transports; no live calls.

Required Phase G — docs:

17. Create BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_<VENDOR>_RESULT.md
    capturing the adapter design, error mapping, timeout choice,
    tests added, validation results.
18. Update MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md with a
    one-line "Adapter for <VENDOR>: SHIPPED" addendum.
19. Update RUN_STATE.md with a closure paragraph.

Validation:

20. cargo fmt.
21. cargo clippy --all-targets --all-features -- -D warnings.
22. cargo test --all-targets --all-features --no-fail-fast.
23. git diff --check.
24. Confirm no .env edit.
25. Confirm no chain tx.
26. Confirm no backend broadcast.
27. Confirm no Safe tx.
28. Confirm no secrets printed.
29. Confirm no vendor credentials in source.
30. Confirm no real KMS key creation.

Forbidden:

* no mainnet tx.
* no Sepolia live broadcast.
* no Safe tx.
* no governance mutation.
* no .env edit.
* no real KMS/HSM/MPC key creation.
* no vendor credentials in source.
* no private key / admin token / RPC secret / DATABASE_URL / API
  key in output.
* no fallback path allowing mainnet local private key signing.

Hard stops:

* stop if a real vendor account or real key would be required.
* stop if implementation would require credentials.
* stop if implementation would require live broadcast.
* stop if implementation would require editing .env.
* stop if any secret would be printed.
* stop if RemoteSignerClient::new is altered to make the adapter
  the default before the rehearsal Phase 3 operator approval.

Return final report grouped by:
workspace,
source/docs inspected,
adapter module path,
SignerTransport implementation summary,
SignerError mapping table,
timeout + retry policy,
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

### 3.2 Vendor-agnostic path

Use this prompt if `MAINNET-KMS-VENDOR-SELECTION` has NOT yet
resolved.

```text
Workspace root is ~/DEOPT.

Execute BACKEND-KMS-VENDOR-ADAPTER-IMPLEMENTATION-PLUGGABLE only.

Current state:

* MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK is closed.
* MAINNET-KMS-VENDOR-SELECTION is NOT yet resolved.
* Vendor selection from the matrix is pending operator decision.
* RemoteSigner trait exists.
* LocalDevSigner exists.
* RemoteSignerClient::with_transport exists (mock injection point).
* UnimplementedTransport is the default for RemoteSignerClient::new.
* Mainnet refuses LocalDev at startup + at runtime.
* /executor/health/v2 surfaces the full signer block.
* 970 backend tests green.
* No mainnet action is authorised.
* No Sepolia live broadcast is authorised.
* No chain transaction is authorised.
* No Safe transaction is authorised.
* No .env edit is authorised.
* No real KMS/HSM/MPC key creation is authorised.

Goal:
Implement a pluggable SignerTransport adapter shape that can be
specialised to ANY vendor in the matrix's shortlist (AWS KMS / GCP
KMS / Azure / Turnkey / Fireblocks / etc.) by swapping a small
inner Provider trait implementation. Do NOT broadcast. Do NOT
create vendor accounts. Do NOT add real credentials. Use only mock
providers in tests.

Required Phase A — inspect:

1. Read:
   * src/execution/remote_signer.rs
   * src/execution/config.rs
   * src/options/service.rs
   * src/options/broadcast_observability.rs
   * src/api/executor_health_v2.rs
   * docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md
   * docs/MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md

Required Phase B — pluggable adapter module:

2. Create src/execution/signer_adapters/pluggable.rs.
3. Define a small inner Provider trait with two methods:
   * sign_prehash(&self, prehash: [u8;32]) -> Future<…>
   * recover_public_key(&self) -> Future<…>
4. Define an outer struct PluggableSignerTransport<P: Provider> that
   implements SignerTransport from
   src/execution/remote_signer.rs:334.
5. The outer struct routes vendor errors onto the existing
   SignerError taxonomy per §3 of the requirements doc.
6. Define a MockProvider used by the test harness.

Required Phase C — wire into RemoteSignerClient:

7. Add RemoteSignerClient::with_pluggable_provider<P>(provider,
   endpoint, expected_address) constructor.
8. UnimplementedTransport remains the default in
   RemoteSignerClient::new.

Required Phase D — typed config:

9. Add typed-config field BACKEND_SIGNER_TIMEOUT_MS (default 2500;
   range 100..=30000) via src/config/env.rs.
10. Do NOT add vendor-specific env keys yet — they land in the
    vendor-specific follow-on once Q-CD-5 resolves.

Required Phase E — observability + health:

11. The existing broadcast call site at src/options/service.rs:1755
    already records signer_attempt / success / denied counters. The
    pluggable adapter must NOT bypass them.

Required Phase F — tests:

12. Implement the 16 test classes from
    docs/MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §6 against
    the MockProvider.
13. All tests are unit-level; no live vendor calls.

Required Phase G — docs:

14. Create BACKEND_KMS_VENDOR_ADAPTER_PLUGGABLE_RESULT.md capturing
    the Provider trait surface + Mock pattern + SignerError mapping
    + test classes + how to ship a vendor-specific Provider in a
    follow-on milestone.
15. Update MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md with a
    one-line "Pluggable adapter shape: SHIPPED" addendum.
16. Update RUN_STATE.md with a closure paragraph.

Validation:

17. cargo fmt.
18. cargo clippy --all-targets --all-features -- -D warnings.
19. cargo test --all-targets --all-features --no-fail-fast.
20. git diff --check.
21. Confirm no .env edit.
22. Confirm no chain tx.
23. Confirm no backend broadcast.
24. Confirm no Safe tx.
25. Confirm no secrets printed.
26. Confirm no vendor credentials in source.

Forbidden:

* no mainnet tx.
* no Sepolia live broadcast.
* no Safe tx.
* no governance mutation.
* no .env edit.
* no real KMS/HSM/MPC key creation.
* no vendor account creation.
* no vendor credentials in source.
* no private key / admin token / RPC secret / DATABASE_URL / API
  key in output.
* no fallback path allowing mainnet local private key signing.
* no removal of UnimplementedTransport from RemoteSignerClient::new.

Hard stops:

* stop if a real vendor account or real key would be required.
* stop if implementation would require credentials.
* stop if implementation would require live broadcast.
* stop if implementation would require editing .env.
* stop if any secret would be printed.

Return final report grouped by:
workspace,
source/docs inspected,
Provider trait surface,
PluggableSignerTransport shape,
MockProvider design,
SignerError mapping table,
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

## 4. Cross-links

* `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract.
* `MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — vendor input.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal flow the
  shipped adapter feeds into.
* `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — rotation +
  incident readiness the adapter MUST not block.
* `BACKEND_SIGNER_INTERFACE_KMS_HSM_ADAPTER_RESULT.md` — current
  Rust-side surface.
