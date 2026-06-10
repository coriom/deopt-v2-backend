# Mainnet signer rotation and incident runbook

**Posture:** DESIGN / DOC ONLY. No source code modified. No `.env`
edited. No transaction execution. No vendor account creation.
**Closes milestone (in part):** `MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors:**
- `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology.
- `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract.
- `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal phases.
- `MAINNET_CUSTODY_POLICY.md §6 + §7` — custody rules.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert routing.
- `MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md` — OPS Safe +
  GOV Safe references.

## 0. Hard rules (this doc)

```text
no transaction execution                 ✅
no vendor credential in tracked docs     ✅
no private custody roster disclosure     ✅
no real key creation                     ✅
no .env edit                             ✅
no fallback to mainnet local-key sign    ✅
no broadcast                             ✅
no Safe-tx execution                     ✅
```

## 1. Scope

Three runbooks under one cover:

* **§2** Normal rotation — planned cycle of OLD → NEXT signer.
* **§3** Emergency compromise — suspected or confirmed key
  compromise.
* **§4** Outage — vendor unavailable or degraded.

All three are **fail-closed**. None require a backend code change.

## 2. Normal rotation (planned cycle)

### 2.1 Triggers

* Quarterly rotation per custody policy §7.2.
* Vendor advisory (e.g. SDK security patch implying clients should
  rotate).
* Personnel change in the operator group with vendor IAM access.

### 2.2 Pre-flight (T-7 days)

* [ ] Schedule rotation window with on-call Backend + Security +
  Operator.
* [ ] Confirm OPS Safe (`0xce0e46Db1072B820CB5eCf30188ED76cb560C932`)
  signers are available with hardware wallets.
* [ ] Confirm GOV Safe
  (`0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166`) signers available
  for any cross-cutting governance actions.
* [ ] Confirm Sepolia rehearsal arc remains green
  (`/executor/health/v2` on Sepolia GREEN; `R5 drift = 0`).
* [ ] Review last successful rotation log archive.

### 2.3 Provision NEXT signer

* [ ] Operator creates NEXT key inside the vendor (sandbox-shaped
  procedure run against MAINNET vendor account; specific steps live
  in the vendor SDK runbook offline).
* [ ] NEXT key MUST have:
  * Non-exportable bit set (per
    `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §C4`).
  * Audit logging enabled.
  * IAM access limited to the signer microservice service account.
* [ ] No raw key bytes leave the vendor.
* [ ] No credential or key id appears in tracked docs; recorded in
  the offline binder.

### 2.4 Derive NEXT address

* [ ] Adapter `derive_address` call on the NEXT key id → produces a
  candidate EVM address.
* [ ] Operator records the address publicly (it is non-secret).

### 2.5 Verify NEXT address offline / read-only

* [ ] Run an independent read-only chain probe against mainnet
  confirming the NEXT address is empty (no prior history; nonce 0;
  no incoming transfers).
* [ ] Cross-check the address against the vendor's audit log
  (vendor-side public-key fingerprint matches the off-chain
  derivation).

### 2.6 Fund NEXT with gas

* [ ] OPS Safe sends a small operator-gas transfer to the NEXT
  address (sufficient for the launch-day gas budget; per the
  funding ladder in
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` Cluster 3 §4.3).
* [ ] Operator records the transfer tx hash.
* [ ] `/executor/health/v2.chain_state_last_seen.be_balance_floor_wei`
  observed against the NEW BE address in a dry-run config; balance
  above floor → GREEN.

### 2.7 Grant executor role to NEXT

* [ ] OPS Safe executes `OptionMatchingEngine.setExecutor(NEXT)` per
  the existing executor-grant procedure.
* [ ] OPS Safe also executes `setExecutor(OLD, false)` AFTER the
  cutover (§2.10 — NOT in the same tx; sequencing matters for
  R-6 accounting bright line).
* [ ] Operator captures the Safe-tx hashes.

### 2.8 Switch backend signer config

* [ ] Operator edits the production secrets store (NOT a tracked
  `.env`) to point `EXECUTOR_FROM_ADDRESS` at the NEXT address and
  the vendor key id at the NEXT key.
* [ ] Backend restart triggers `ExecutionConfig::validate_startup`;
  on success the runtime guard at `build_signer_for_state` confirms
  Remote mode + endpoint set.
* [ ] No `EXECUTOR_PRIVATE_KEY` is set (defence-in-depth — refused
  by `validate_startup` on chain_id 8453).

### 2.9 Verify `/executor/health/v2` + `/metrics`

* [ ] `signer.signer_address == NEXT address`.
* [ ] `signer.remote_signer_configured == true`.
* [ ] `local_signer_on_mainnet_refused_total == 0`.
* [ ] `overall_status == "green"` with empty `hard_stops`.
* [ ] Prometheus scrape confirms no anomalous alert firing.

### 2.10 Revoke OLD executor

* [ ] OPS Safe executes `setExecutor(OLD, false)` on the OME.
* [ ] Read-only verification: `OME.isExecutor(OLD) == false`.
* [ ] `/executor/health/v2` continues green (because the backend
  already moved to NEXT; OLD is no longer in `signer.signer_address`).

### 2.11 Drain OLD gas (if appropriate)

* [ ] Operator decides per custody policy §7.3:
  * If OLD held substantial residual gas, sweep to OPS Safe via a
    standard EOA transfer (signed by the vendor's OLD key — this is
    the OLD key's FINAL operation).
  * If residual gas is dust, leave; the key will be disabled in
    §2.12.

### 2.12 Disable OLD key

* [ ] Operator disables (NOT deletes) the OLD vendor key. Disable
  preserves audit history.
* [ ] Operator records disable timestamp + vendor's confirmation id
  in the offline binder.

### 2.13 Archive logs

* [ ] Backend ships rotation-window logs to the audit archive (per
  §5 retention policy).
* [ ] Vendor audit log export captured for the rotation window.
* [ ] OPS Safe + GOV Safe tx hashes recorded.
* [ ] Rotation result written to a NEW
  `MAINNET_SIGNER_ROTATION_<YYYY-MM-DD>_RESULT.md` (public-safe;
  no key ids, no credentials, no human roster details).

### 2.14 Acceptance

A rotation is COMPLETE when every item above is checked AND a 24-hour
soak period shows:

* [ ] `/executor/health/v2.overall_status == "green"`.
* [ ] `signer_attempted_total` and `signer_success_total` continue to
  grow at expected rates.
* [ ] `last_signer_error_code` does not become a denial code.
* [ ] No alert fires from
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3.1` (signer integrity).

## 3. Emergency compromise

### 3.1 Triggers

Any of:

* Vendor reports suspicious activity against the key.
* Backend logs show `signer_denied_total{code="caller-unauthorized"}`
  spike (an unauthorised caller hit the signer service).
* `/executor/health/v2.signer.local_signer_on_mainnet_refused_total >
  0` (defence-in-depth fired — should be impossible under normal
  operation).
* Public disclosure of vendor SDK / dependency vulnerability with
  immediate exploit risk.
* Personnel security event implicating someone with vendor IAM
  access.

### 3.2 First 5 minutes — contain

* [ ] **Disable signer key** at the vendor (single API call; do NOT
  delete — preserve forensic evidence).
* [ ] **Disable backend execution** by restarting with
  `EXECUTION_ENABLED=false` (no broadcast attempts).
* [ ] **Pause OME** via OPS Safe `OME.pause()` IF the chain-state
  suggests in-flight risk (e.g., a confirmed unauthorised tx from
  the BE address). Pause is high-blast-radius — invoke ONLY with
  Security approval.
* [ ] **Revoke executor** via OPS Safe `setExecutor(OLD, false)` —
  removes the chain-side authority even if the vendor disable is
  later rolled back.

### 3.3 Within 30 minutes — rotate credentials

* [ ] Rotate any RPC provider tokens (custody §7.5).
* [ ] Rotate vendor API key / mTLS cert pair.
* [ ] Rotate admin tokens for the backend admin endpoints.
* [ ] Rotate webhook secrets used by the monitoring alert routes.
* [ ] Force re-authentication on all backend operator sessions.

### 3.4 Within 24 hours — investigate + report

* [ ] Pull vendor audit log for the compromise window.
* [ ] Pull backend `tracing` audit log
  (`target: "broadcast_signer"` and `target: "deopt.admin.audit"`).
* [ ] Pull `/executor/health/v2` snapshot for the window.
* [ ] Pull `/metrics` Prometheus history.
* [ ] Reconstruct the timeline.
* [ ] Operator publishes a public-safe incident report (no signer
  identity, no vendor credentials, no roster).
* [ ] Provision a NEW signer per §2 — the NORMAL rotation runbook
  becomes the recovery path.

### 3.5 Preserve audit logs

* The compromised vendor key remains DISABLED, not deleted, for at
  least the retention window in §5.
* Backend log archive for the incident window is sealed (operator
  copies to a separate immutable bucket with retention lock).

### 3.6 Acceptance

The incident is RESOLVED when:

* [ ] Compromised key fully disabled + audit log archived.
* [ ] NEW signer provisioned + executor role granted per §2.
* [ ] Public incident report published.
* [ ] Internal post-mortem completed with corrective actions.
* [ ] `/executor/health/v2.overall_status` returns to green for a
  72-hour soak.

## 4. Outage (remote signer unavailable or degraded)

### 4.1 Triggers

* `signer_attempted_total` growing but `signer_success_total` flat
  — sign requests are not completing.
* `signer_denied_total{code="kms-timeout"}` or
  `signer_denied_total{code="transport"}` spike.
* Vendor status page shows incident affecting the relevant region.
* Backend `tracing` shows `SignerError::Transport(...)` increasing.

### 4.2 First 5 minutes — contain

* [ ] Page on-call per
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3.1`.
* [ ] Confirm vendor incident scope via vendor status page.
* [ ] **Backend is already fail-closed by design** —
  `should_broadcast` may still APPROVE intents, but the signer call
  fails → broadcast HALTS without any local-key attempt
  (`LocalDevSigner` runtime guard at
  `src/execution/remote_signer.rs:283`; `build_signer_for_state`
  guard at `src/options/service.rs:1465`). No operator action is
  needed for safety.
* [ ] Operator notification to users: broadcast paused; matching
  engine + indexer remain healthy; settlement of in-flight intents
  resumes on signer recovery.

### 4.3 During outage

* [ ] Monitor `/executor/health/v2` — `signer.last_signer_error_code`
  reports the current category (`kms-timeout` / `transport` /
  `caller-unauthorized` / etc.).
* [ ] Monitor vendor status page for ETA.
* [ ] Operator MAY temporarily set `EXECUTION_ENABLED=false` if the
  failure rate produces excessive `broadcast_failed` rows in the DB
  (avoid log noise). This is a soft-disable — restoring is a
  single env var change + restart.

### 4.4 What MUST NOT happen during outage

* No mainnet local-key signing. The custody policy §6 BE-5
  prohibition is absolute. There is no "emergency exception" for
  outage scenarios. The system is fail-closed by design and stays
  that way.
* No bypass flag is acceptable. The `validate_signer_backend`
  guard MUST stay intact.
* No swap to a different vendor mid-outage. Vendor rotation goes
  through §2's planned rotation procedure with full validation.

### 4.5 Recovery

* [ ] Vendor recovers; vendor status page declares resolved.
* [ ] Backend `tracing` shows `signer_success_total` resumes growth.
* [ ] `/executor/health/v2.signer.last_signer_success_at_ms` updates.
* [ ] Confirmation worker processes any in-flight intents whose
  signer call had been retried since the recovery.
* [ ] Operator publishes a brief post-incident note.

### 4.6 Acceptance

The outage is RECOVERED when:

* [ ] `/executor/health/v2.overall_status == "green"`.
* [ ] `signer_success_total` growth rate returns to baseline.
* [ ] No follow-on alerts firing.
* [ ] Operator post-incident note published.

## 5. Audit-log retention

| Source | Retention | Storage |
|---|---|---|
| Vendor per-request audit log (`kms_request_id`) | 7 years minimum | Vendor-side; export monthly to operator's immutable archive bucket. |
| Backend `tracing` log (`target: "broadcast_signer"`) | 7 years minimum | Operator's immutable archive bucket. |
| Backend admin audit log (`target: "deopt.admin.audit"`) | 7 years minimum | Operator's immutable archive bucket. |
| `/metrics` Prometheus snapshot | 18 months on hot store; 7 years on archive. | Operator's Prometheus / archive bucket. |
| `/executor/health/v2` snapshot | 18 months on hot store; 7 years on archive (selectively for the incident window only). | Operator's archive bucket. |
| Safe-tx history (OPS + GOV) | Indefinite (chain-native; immutable). | Chain. |
| Rotation result docs (`MAINNET_SIGNER_ROTATION_<DATE>_RESULT.md`) | Indefinite. | Tracked repo, public-safe content. |
| Vendor IAM access logs | 7 years minimum. | Vendor-side; quarterly export. |
| Compromised-window sealed archive | Indefinite. | Operator's immutable archive bucket with retention lock. |

Retention windows are minimums per
`MAINNET_CUSTODY_POLICY.md §8`. Operators MAY exceed; never reduce
without legal review.

## 6. Cross-links

* `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology.
* `MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — vendor decision input.
* `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — rehearsal context.
* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert routing.
* `MAINNET_CUSTODY_POLICY.md §6.7 + §7 + §8` — custody rules.
* `MAINNET_CUSTODY_CLUSTER_1_RESOLUTION_REDACTED.md` — OPS / GOV Safe
  references.
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia precedent
  for cutover sequencing.
