# Mainnet signer staging rehearsal plan

**Posture:** DESIGN / DOC ONLY. No source code modified. No `.env`
edited. No transaction execution. No canary broadcast.
**Closes milestone (in part):** `MAINNET-SIGNER-VENDOR-AND-REHEARSAL-PACK`.
**Anchors:**
- `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology.
- `MAINNET_SIGNER_VENDOR_SELECTION_MATRIX.md` — vendor input.
- `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract.
- `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — prior Sepolia
  rehearsal precedent.
- `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health surface used as
  acceptance signal.
- `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 + §4` — alert
  thresholds.

## 0. Hard rules (this doc)

```text
no transaction execution                 ✅
no canary broadcast                      ✅
no real KMS/HSM/MPC key creation         ✅
no provider credential in tracked docs   ✅
no .env edit                             ✅
no Safe tx                               ✅
no governance mutation                   ✅
no mainnet broadcast                     ✅
```

## 1. Goal

Sequenced rehearsal path from "no remote signer wired" to "mainnet
signer is provisioned, verified, and READY for the launch-day
broadcast" — without sending any transaction in this milestone.

## 2. Phase ladder

The rehearsal proceeds through 7 phases. Each phase has a stable
acceptance signal (the `/executor/health/v2` envelope + `/metrics`
counters); a phase is COMPLETE when every acceptance criterion below is
GREEN.

### Phase 1 — Mock remote signer

**Setup**
* Backend running with `BACKEND_SIGNER_MODE=remote` +
  `BACKEND_SIGNER_ENDPOINT=https://mock-signer.local`.
* `RemoteSignerClient::with_transport(mock)` injected (test-only,
  in-process); production builds STILL use `UnimplementedTransport`.

**Acceptance**

* [ ] `cargo test --lib remote_signer` passes for the mock-injection
  paths (already pinned by the existing `MockTransport` tests in
  `src/execution/remote_signer.rs::tests`).
* [ ] `signer.signer_mode == "remote"` on `/executor/health/v2`.
* [ ] `signer.remote_signer_configured == true`.
* [ ] `local_signer_on_mainnet_refused_total == 0`.

**Exit criterion**: mock-injection tests green; no flake.

### Phase 2 — Sandbox vendor signer (if available)

**Setup**
* Operator provisions a **sandbox-tier** vendor key (AWS KMS test
  region / GCP KMS test project / Turnkey sandbox org / etc.).
* Sandbox MUST be a separate account / project / org from mainnet —
  no cross-contamination risk.
* Vendor adapter (per
  `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md`) wired against the
  sandbox endpoint via the typed `BACKEND_SIGNER_ENDPOINT` config.
* Backend chain id: `31337` (anvil) — sandbox key is NOT mainnet.

**Acceptance**

* [ ] `health_check()` returns Ok against the sandbox endpoint.
* [ ] `derive_address()` matches the address recorded in the sandbox
  vendor's audit log.
* [ ] Adapter `derive_address` + `health_check` tests pass in CI
  against the sandbox endpoint (manual CI run with sandbox creds in
  CI secret store, NOT in tracked code).
* [ ] No vendor account or credential appears in tracked docs.

**Exit criterion**: sandbox round-trip green; the adapter responds
to every documented error mode (timeout, 5xx, 429, 403, malformed
signature recovery) per `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §6`.

### Phase 3 — Sepolia remote-signer rehearsal

**Setup**
* Operator provisions a **Sepolia-only** vendor key + records the
  derived address as `EXECUTOR_FROM_ADDRESS` for the Sepolia
  environment.
* Backend config:
  * `EXECUTOR_CHAIN_ID=84532`.
  * `BACKEND_SIGNER_MODE=remote`.
  * `BACKEND_SIGNER_ENDPOINT=<sepolia-vendor-url>`.
  * `EXECUTOR_ALLOW_LOCAL_SIGNER=false` (explicit refusal of the
    legacy local-key path on Sepolia too — the rehearsal validates
    the Remote path is the sole production-shaped surface).
  * Sepolia BE funded with Sepolia ETH.

**Acceptance**

* [ ] Backend starts cleanly; `ExecutionConfig::validate_startup`
  passes.
* [ ] `/executor/health/v2` reports `signer.remote_signer_configured ==
  true`, `signer.signer_address == <sepolia BE address>`.
* [ ] One simulated option execution intent → `should_broadcast`
  approves → backend issues a SignerRequest → vendor signs → backend
  reassembles raw tx → backend broadcasts to Sepolia → confirmation
  worker observes mined_success.
* [ ] **The Sepolia rehearsal is the ONLY phase that broadcasts.** It
  is permitted by the existing Sepolia rehearsal arc (`Orderbook live
  smoke is closed`; `RFQ live smoke is closed`) and is supervised
  per `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`. This milestone's
  documentation phase does NOT itself broadcast; the rehearsal phase
  is gated on operator authorisation in a separate runbook execution.

**Exit criterion**: Sepolia broadcast confirmed; `/executor/health/v2`
green; `R5 drift = 0`.

### Phase 4 — No-broadcast mainnet dry run

**Setup**
* Mainnet vendor key provisioned (separate from Sepolia) per the
  rotation runbook §2.
* Mainnet BE address **derived but NOT yet granted executor role**.
* Backend config:
  * `EXECUTOR_CHAIN_ID=8453`.
  * `BACKEND_SIGNER_MODE=remote`.
  * `BACKEND_SIGNER_ENDPOINT=<mainnet-vendor-url>`.
  * `EXECUTOR_REAL_BROADCAST_ENABLED=false`.
  * `EXECUTOR_PRIVATE_KEY` NOT SET (refused by
    `validate_startup`).

**Acceptance**

* [ ] Backend starts; `ExecutionConfig::validate_startup` passes.
* [ ] `/executor/health/v2.signer.signer_mode == "remote"`.
* [ ] `/executor/health/v2.signer.signer_address` matches the derived
  mainnet address.
* [ ] `/executor/health/v2.overall_status == "green"` (no hard stops).
* [ ] `signer.local_signer_on_mainnet_refused_total == 0` —
  defence-in-depth NOT TRIGGERED.
* [ ] Operator validates `health_check()` returns Ok by hitting a
  dedicated admin endpoint (read-only; no sign call).
* [ ] No broadcast attempted; `signer_attempted_total == 0`.

**Exit criterion**: every config-level signal is consistent with a
production-ready Remote signer; the only missing piece is the
chain-side grant of the executor role.

### Phase 5 — Read-only mainnet preflight

**Setup**
* Mainnet contract addresses + RPC URL configured.
* Live-provider config gates wired
  (`PROTOCOL_FEE_VAULT_ADDRESS`, `FEES_MANAGER_V2`, collateral
  vault).

**Acceptance**

* [ ] `/executor/health/v2.live_provider_config.protocol_fee_vault_configured == true`.
* [ ] `/executor/health/v2.live_provider_config.fees_manager_v2_configured == true`.
* [ ] `/executor/health/v2.live_provider_config.collateral_vault_configured == true`.
* [ ] No `policy_data_failures_total{*}` increments observed in a 1-hour
  observation window.
* [ ] `R5 drift` observed: `last_r5_drift_zero == Some(true)`.
* [ ] Sweep against the launch-invariant verifier (rebateReserve = 0
  for every configured asset) → GREEN.

**Exit criterion**: every live-provider read succeeds; no chain-state
hard stop in `/executor/health/v2`.

### Phase 6 — Final Sepolia canary with remote signer

**Setup**
* Re-run Phase 3 a final time AGAINST THE PRODUCTION COMMIT (the
  exact deploy artifact slated for mainnet launch). No code drift
  between this canary and the mainnet broadcast.

**Acceptance**

* [ ] Phase 3 criteria all GREEN against the production commit.
* [ ] Confirmation worker auto-transition observed end-to-end.
* [ ] `/executor/transactions/:intent_id` returns the row with
  `source == "option"` + correct `source_type`.
* [ ] `/executor/transactions` list view includes the row.
* [ ] `BadNonce` remediation pattern dry-run (deliberate
  `(0,0) → (1,1)` chain misalignment + recovery) → confirmation
  worker recovers without operator intervention.

**Exit criterion**: production-commit Sepolia rehearsal green; commit
hash recorded for the mainnet launch operation.

### Phase 7 — Mainnet canary PREPARATION (no broadcast yet)

**Setup**
* Treasury Safe + InsuranceFund + governance Timelock configured
  per their respective milestones.
* OPS Safe (`0xce0e46Db1072B820CB5eCf30188ED76cb560C932`) ready to
  execute `setExecutor(OPTION_BACKEND_EXECUTOR)` on the OME.
* Mainnet BE has received gas funding from the OPS Safe.
* All §3 alert routes (PagerDuty + Discord) live and tested
  per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`.

**Acceptance** (preparation, NOT broadcast)

* [ ] `setExecutor` packet built (read-only — Safe-tx data) and
  reviewed by Security.
* [ ] Operator on-call rotation staffed.
* [ ] OPS Safe signers gathered with hardware wallets ready.
* [ ] Rollback path rehearsed (see §3 below).
* [ ] `/executor/health/v2` green on the production binary against
  mainnet config.
* [ ] One simulated intent → backend computes prehash → adapter
  performs `derive_address` round-trip (no signing) → confirms
  vendor key healthy.
* [ ] Operator authorisation captured (signed handoff per
  `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md` operator policy)
  for the broadcast operation.

**Exit criterion**: every signal needed to execute the launch-day
broadcast in a separate authorised operation is GREEN. The
broadcast itself is OUT OF SCOPE for this milestone.

## 3. Rollback plan

Each phase has a defined rollback. The rollback path is
**fail-closed**: if any phase produces a red signal, the backend
broadcast capability is disabled and the next phase is NOT entered.

### 3.1 Disable remote signer

* Set `BACKEND_SIGNER_MODE=local_dev` on the **Sepolia / dev**
  environment ONLY (mainnet refuses LocalDev).
* On mainnet: restart the backend with
  `EXECUTOR_REAL_BROADCAST_ENABLED=false` — the broadcast path is
  inert; signer is not called.

### 3.2 Disable execution

* Set `EXECUTION_ENABLED=false` and restart.
* `/executor/status` reports `executionEnabled: false`.
* No new broadcasts attempted.

### 3.3 Pause if needed

* If chain-state evidence suggests the BE address is misbehaving
  (unlikely in this milestone since no broadcast), the OPS Safe
  executes `OME.pause()` per the existing pause runbook.
* This is a HARD chain-side fail-closed.

### 3.4 Keep backend fail-closed

* `EXECUTOR_PRIVATE_KEY` MUST remain unset on mainnet (refused by
  `validate_startup`).
* `BACKEND_SIGNER_MODE=remote` MUST remain the only path on mainnet.
* `LocalDevSigner` runtime guard MUST remain intact.
* No bypass flag or env shim is acceptable.

### 3.5 Observability during rollback

* `/executor/health/v2` MUST continue to report GREEN
  `overall_status` once rollback completes — the system is
  intentionally fail-closed; no hard stops should remain set.
* `signer.local_signer_on_mainnet_refused_total` MUST remain 0
  (any non-zero value means the runtime guard fired, which is a
  red incident regardless of rollback state).

## 4. Acceptance signal summary

Single source of truth: `/executor/health/v2`. The phase ladder
relies on the following fields being GREEN at each transition:

| Field | Phase 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|
| `signer.signer_mode == "remote"` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `signer.remote_signer_configured` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `signer.signer_address` set | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `local_signer_on_mainnet_refused_total == 0` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `live_provider_config.*_configured` | — | — | — | — | ✓ | ✓ | ✓ |
| `last_r5_drift_zero == Some(true)` | — | — | — | — | ✓ | ✓ | ✓ |
| `overall_status == "green"` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

## 5. Non-goals

* This rehearsal plan does NOT define mainnet launch-day broadcast
  execution. That is a separate authorised runbook operation.
* This plan does NOT specify Frontend cutover; see
  `FRONTEND-V2G-W3-SSR-PROXY` for that track.
* This plan does NOT cover Treasury Safe / InsuranceFund operations
  (separate operator milestones).

## 6. Cross-links

* `MAINNET_BE_SIGNER_SERVICE_DESIGN.md` — Pattern C topology.
* `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md` — adapter contract
  that the rehearsal exercises.
* `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md` — what to do
  when a phase fails.
* `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — alert routing under
  rehearsal load.
* `EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health envelope schema.
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` — Sepolia precedent.
