# Mainnet go / no-go criteria

**Posture:** DOC ONLY. **No chain mutation. No `.env` edit. No
Safe-tx. No broadcast. No mainnet activation by this doc — this doc
DEFINES the conditions under which mainnet activation may be
authorised in a separately-runbook'd operation.**

**Companion:** `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`.

## 0. Hard rules

```text
no mainnet tx                                 ✅
no Sepolia tx                                 ✅
no Safe tx                                    ✅
no broadcast                                  ✅
no AWS resource creation                      ✅
no .env edit                                  ✅
no secret printed                             ✅
no production address committed to log        ✅
no fallback to LocalDev signer on mainnet     ✅
```

## 1. Status colors

| Color | Meaning |
|---|---|
| **GREEN** | All criteria met. Authorised milestone may proceed (subject to per-milestone operator authorisation captured offline). |
| **YELLOW** | Some criteria are at-risk but not failed. Operator + Security review required before proceeding. Document the YELLOW criteria in the milestone's pre-flight notes. |
| **RED** | One or more criteria FAILED. NO-GO. Rollback per `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`. |

## 2. GREEN criteria (must ALL be true for mainnet canary preparation)

### 2.1 Backend layer

| # | Criterion |
|---|---|
| BG1 | `cargo fmt --check` clean. |
| BG2 | `cargo clippy --all-targets -- -D warnings` clean (default features). |
| BG3 | `cargo clippy --all-targets --all-features -- -D warnings` clean. |
| BG4 | `cargo test --all-targets --no-fail-fast` 1032+ tests green. |
| BG5 | `cargo test --all-targets --all-features --no-fail-fast` 1053+ tests green. |
| BG6 | `RemoteSignerClient::new` continues to use `UnimplementedTransport` (production-default fail-closed). |
| BG7 | `ExecutionConfig::validate_signer_backend` mainnet refusal of `LocalDev` mode + `Mock` provider intact. |
| BG8 | `LocalDevSigner::sign_option_execution_tx` runtime refusal on chain_id 8453 intact. |
| BG9 | `build_signer_for_state` runtime guard on chain_id 8453 intact. |
| BG10 | No `EXECUTOR_PRIVATE_KEY` set on mainnet env (`validate_startup` refuses). |
| BG11 | `BACKEND_SIGNER_MODE=remote` confirmed for mainnet env. |
| BG12 | `BACKEND_REMOTE_SIGNER_PROVIDER=aws_kms` confirmed. |
| BG13 | `BACKEND_SIGNER_ENDPOINT` set + non-empty + does not embed credentials. |
| BG14 | `BACKEND_SIGNER_TIMEOUT_MS` in 100..=30000. |
| BG15 | `EXECUTOR_FROM_ADDRESS` matches the KMS-derived production address. |
| BG16 | `EXECUTOR_REAL_BROADCAST_ENABLED` is FALSE at preflight (will be flipped true only at the authorised canary moment). |

### 2.2 Sol / contracts layer

| # | Criterion |
|---|---|
| SG1 | All in-scope contracts deployed on Base mainnet per `MAINNET-DEPLOYMENT`. |
| SG2 | OME `owner` == Timelock. |
| SG3 | PFV `owner` == Timelock. |
| SG4 | FM_V2 `owner` == Timelock. |
| SG5 | CV `owner` == Timelock. |
| SG6 | RG `owner` == Timelock. |
| SG7 | Timelock `PROPOSER_ROLE` held by OPS Safe; `EXECUTOR_ROLE` held by OPS Safe; `TIMELOCK_ADMIN_ROLE` held by Timelock itself; DEPLOYER stripped of all roles. |
| SG8 | OME `paused()` == false. |
| SG9 | PFV `paused()` == false. |
| SG10 | FM_V2 not paused (if it has the field). |
| SG11 | RG `paused()` == false. |
| SG12 | OME `isExecutor(BE)` == true (after `setExecutor` Safe-tx executed). |
| SG13 | Cluster 4 launch invariant: for every configured asset, `PFV.rebateReserve(asset)=0` AND `FM_V2.rebateBudget(asset)=0` AND `CV.balances(PFV,asset)=0`. |
| SG14 | R5 drift = 0: `CV.balances(PFV,asset) - (PFV.feeBalance(asset) + PFV.rebateReserve(asset)) = 0` for every asset. |

### 2.3 Custody layer

| # | Criterion |
|---|---|
| CG1 | OPS Safe (`0xce0e46Db…0C932`) — threshold 2/3, owners 3, nonce known. |
| CG2 | GOV Safe (`0x7C6Ce20e…b166`) — threshold 3/5, owners 5, nonce known. |
| CG3 | OPS / GOV owner overlap = 0. |
| CG4 | DEPLOYER not an owner of OPS or GOV. |
| CG5 | Mainnet DEPLOYER attestation Q-CD-8 RESOLVED (binder confirmed). |
| CG6 | Treasury Safe address known + threshold + roster confirmed (or operator policy declares NOT_APPLICABLE at launch). |
| CG7 | Insurance Fund operator policy confirmed (or NOT_APPLICABLE at launch). |
| CG8 | Q-CD-5 vendor sub-decision technically closed (AWS KMS); operator commercial sign-off captured offline. |

### 2.4 Backend / signer runtime layer

| # | Criterion |
|---|---|
| RG1 | AWS KMS key created with `KeySpec=ECC_SECG_P256K1`, `KeyUsage=SIGN_VERIFY`, `Origin=AWS_KMS`. |
| RG2 | `kms:GetPublicKey` returns SPKI; derived EVM address matches `EXECUTOR_FROM_ADDRESS`. |
| RG3 | Signer runtime IAM role (`<SIGNER_RUNTIME_ROLE_NAME>`) attached to backend / signer microservice instance. |
| RG4 | IAM role policy verified via `iam:simulate-principal-policy` per `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md §3 P13-P24`. |
| RG5 | CloudTrail trail capturing KMS data events for `<KMS_KEY_ARN>`. |
| RG6 | Backend `health_check` returns Ok against the real KMS endpoint. |
| RG7 | Backend `health_check` does NOT issue `kms:Sign` (verified by CloudTrail event lookup). |
| RG8 | `/executor/health/v2.signer.local_signer_on_mainnet_refused_total == 0`. |
| RG9 | `/executor/health/v2.signer.signer_address` matches the KMS-derived address. |
| RG10 | No `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` in any tracked .env or .env.example. |

### 2.5 Health endpoint + metrics

| # | Criterion |
|---|---|
| HG1 | `/health` returns `{"ok":true}`. |
| HG2 | `/ready` returns `ready=true`. |
| HG3 | `/executor/health/v2.overall_status == "green"`. |
| HG4 | `/executor/health/v2.not_tracked_yet` is empty `[]`. |
| HG5 | `live_provider_config.protocol_fee_vault_configured == true`. |
| HG6 | `live_provider_config.fees_manager_v2_configured == true`. |
| HG7 | `live_provider_config.collateral_vault_configured == true`. |
| HG8 | `r5.drift_observed_total == 0` (cumulative). |
| HG9 | All Z1-Z11 counters from `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md §7` == 0 at preflight. |
| HG10 | `chain_state_last_seen.be_balance_floor_wei` matches the configured gas budget. |

### 2.6 Audit layer

| # | Criterion |
|---|---|
| AG1 | External audit engagement closed with auditor findings published. |
| AG2 | All Critical findings resolved. |
| AG3 | All High findings either resolved OR accepted with documented risk acceptance + auditor sign-off. |
| AG4 | Audit minimum-pass condition per `MAINNET_AUDIT_EXT_ENGAGEMENT_PACKAGE.md` met. |
| AG5 | Internal audit (V2G-AUDIT0) findings either resolved or accepted. |

### 2.7 Operator layer

| # | Criterion |
|---|---|
| OG1 | Operator on-call rotation staffed for the canary window. |
| OG2 | OPS Safe signers gathered with hardware wallets ready. |
| OG3 | Rollback plan rehearsed per `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`. |
| OG4 | `setExecutor` Safe-tx packet built (read-only) and reviewed by Security. |
| OG5 | Mainnet RPC URL configured in operator secret store. |
| OG6 | Mainnet `DATABASE_URL` configured in operator secret store. |
| OG7 | Monitoring alert routes (PagerDuty / Discord) live + tested. |
| OG8 | Final operator authorisation captured (signed handoff per `MAINNET_CUSTODY_CLUSTER_2_RESOLUTION_REDACTED.md`). |

### 2.8 Rehearsal phase signoff

| # | Criterion |
|---|---|
| PG1 | Phase 1 mock remote signer — covered by 22 unit tests. |
| PG2 | Phase 2 sandbox AWS KMS — operator-executed; result doc published. |
| PG3 | Phase 3 Sepolia remote-signer rehearsal — operator-executed; mined_success observed; result doc published. |
| PG4 | Phase 4 no-broadcast mainnet dry run — operator-executed; `/executor/health/v2.overall_status == "green"`. |
| PG5 | Phase 5 read-only mainnet preflight — operator-executed per `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md`. |
| PG6 | Phase 6 final Sepolia canary against production commit — operator-executed; result doc published. |

## 3. YELLOW criteria (operator + Security review required)

| Field | Yellow trigger |
|---|---|
| `signer.last_signer_error_code` | Non-null at preflight — means a prior denial occurred; investigate context. |
| `signer_denied_total{*}` cumulative | Non-zero but small (`<5`) — operator confirms each denial source. |
| `fm_v2_*_failures_total` | Non-zero — investigate RPC / ABI drift; do NOT proceed until counter returns to 0 for `>1h`. |
| `policy_data_failures_total{*}` | Non-zero — same; operator investigates which `read_type` failed. |
| Phase 6 Sepolia canary commit drift | Production commit hash differs from Phase 6 commit hash — must re-run Phase 6. |
| Vendor sandbox region differs from mainnet region | Operator confirms; document the diff. |
| Phase 2 result doc more than 90 days old | Re-run Phase 2 against the current production commit. |
| `chain_state_last_seen.be_balance_wei` < `be_balance_floor_wei` | BE gas funding too low; OPS Safe tops up BEFORE canary. |

## 4. RED criteria — IMMEDIATE NO-GO

ANY of the following → NO-GO. Rollback per
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`.

| # | Field | Red trigger |
|---|---|---|
| R1 | `EXECUTOR_PRIVATE_KEY` set on mainnet | `validate_startup` refuses; deployment fails. |
| R2 | `BACKEND_SIGNER_MODE != remote` on mainnet | `validate_signer_backend` refuses. |
| R3 | `BACKEND_REMOTE_SIGNER_PROVIDER == mock` on mainnet | `validate_signer_backend` refuses. |
| R4 | `signer.local_signer_on_mainnet_refused_total > 0` | Defence-in-depth fire: `LocalDev` was somehow seated on mainnet and rejected at runtime — compromise OR misconfiguration. |
| R5 | `not_tracked_yet` non-empty | Health endpoint schema gap. |
| R6 | `r5.drift_observed_total > 0` | R5 invariant breach. |
| R7 | `last_r5_drift_zero == Some(false)` | R5 invariant breach observed in latest read. |
| R8 | `live_provider_config.protocol_fee_vault_configured == false` on mainnet | PFV address not configured. |
| R9 | `live_provider_config.fees_manager_v2_configured == false` on mainnet | FM_V2 address not configured. |
| R10 | `live_provider_config.collateral_vault_configured == false` on mainnet | CV address not configured. |
| R11 | `chain_state_last_seen.ome_paused == Some(true)` | OME paused. |
| R12 | `chain_state_last_seen.ome_is_executor == Some(false)` | BE not authorised as OME executor. |
| R13 | Cluster 4 launch invariant breached: ANY of `PFV.rebateReserve > 0` / `FM_V2.rebateBudget > 0` / `CV.balances(PFV, asset) != 0` at preflight | Operator-side seeding error or compromise. |
| R14 | Mainnet OPS Safe owner overlap with GOV Safe `!= 0` | Custody policy violation. |
| R15 | DEPLOYER is an owner of OPS or GOV Safe | Custody policy violation. |
| R16 | DEPLOYER holds ANY Timelock role | Custody policy violation. |
| R17 | OME `owner != Timelock` (or PFV/FM_V2/CV/RG owner != Timelock) | Governance migration not complete. |
| R18 | Critical audit finding unresolved | Audit gate failure. |
| R19 | High audit finding unresolved AND not formally accepted | Audit gate failure. |
| R20 | Real AWS account ID / KMS key id / ARN / signer EVM address committed to tracked docs | Information hygiene failure. |
| R21 | `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` in tracked .env / .env.example | Credential hygiene failure. |
| R22 | `cargo test` red | Backend regression. |
| R23 | `cargo clippy --all-features -- -D warnings` red | Backend regression. |
| R24 | Mainnet Timelock instance not deployed | Governance gap. |
| R25 | OPS Safe signer not staffed for the canary window | Operational gap. |
| R26 | Mainnet RPC URL unreachable or unauthorised | Infrastructure gap. |
| R27 | CloudTrail trail not capturing KMS data events | Audit gap. |
| R28 | `iam:simulate-principal-policy` shows runtime role can perform a denied action | IAM drift. |
| R29 | Operator commercial sign-off on Q-CD-5 NOT captured | Decision gap. |

## 5. Rollback triggers (post-canary)

If the mainnet canary broadcast fires (Phase 7 future milestone) and
any of the following are observed, rollback per
`MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3`:

| # | Trigger |
|---|---|
| RB1 | `signer.local_signer_on_mainnet_refused_total > 0` after the canary. |
| RB2 | Cluster 4 launch invariant breached post-canary (rebateReserve / rebateBudget / CV(PFV) drift from zero). |
| RB3 | R5 drift observed post-canary. |
| RB4 | OME `paused()` becomes true unexpectedly. |
| RB5 | OME `isExecutor(BE)` becomes false unexpectedly. |
| RB6 | Confirmation worker fails to observe mined_success within 30 minutes. |
| RB7 | `signer_denied_total{*}` increments during the canary window. |
| RB8 | `policy_data_failures_total{*}` increments during the canary window. |
| RB9 | Real CloudTrail RequestId does NOT appear in backend logs (synthetic-only) → audit correlation gap; halt + investigate. |

## 6. Canary abort conditions (mid-flight; before second tx)

Abort the canary path AND disable execution if any of the following
appears within the FIRST canary's broadcast window:

| Condition | Action |
|---|---|
| `should_broadcast` reject code = `policy:rebate-reserve` (rebate-positive intent + reserve=0) | Already-blocked-by-policy; investigate intent source. |
| `should_broadcast` reject code = `negative-effective-ppm` on mainnet | Already-blocked-by-policy; investigate fee config. |
| Sign operation returns `SignerError::PostSignFromMismatch` | KMS key id / address drift; abort + investigate. |
| Sign operation returns `SignerError::ChainNotAllowed` | Signer-microservice policy gate firing; investigate. |
| `chain_state_last_seen.be_balance_wei < be_balance_floor_wei` | BE drained; refill before continuing. |
| RPC unreachable for `>5min` | Operator notification; defer canary. |
| OPS Safe signer unreachable | Defer canary. |

## 7. Signer abort conditions

Disable signer + execution if any of the following at any time:

| Condition | Action |
|---|---|
| `signer.local_signer_on_mainnet_refused_total > 0` | Disable execution; investigate config drift. |
| CloudTrail shows `Sign` from a principal OTHER than `<SIGNER_RUNTIME_PRINCIPAL_ARN>` | INCIDENT — execute `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §3 emergency compromise`. |
| `signer_denied_total{code="kms-timeout"}` spike | Vendor outage; backend already fail-closes; alert on-call. |
| `signer_denied_total{code="transport"}` spike | Same. |
| KMS `DisableKey` event in CloudTrail (not operator-initiated) | INCIDENT. |

## 8. Monitoring abort conditions

| Condition | Action |
|---|---|
| Prometheus scrape failure for `>15min` | Investigate; defer canary if mid-preflight. |
| CloudWatch / SIEM forwarding lag `>1h` | Investigate audit pipeline. |
| Backend `/metrics` endpoint returns 5xx | Investigate. |
| Backend `/executor/health/v2` returns 5xx | Investigate; defer canary. |

## 9. Custody abort conditions

| Condition | Action |
|---|---|
| OPS Safe nonce drifts unexpectedly | Investigate. |
| GOV Safe nonce drifts unexpectedly | Investigate. |
| Timelock role assignment changes outside an authorised governance window | INCIDENT. |
| OME `owner()` returns a different address than Timelock | INCIDENT. |
| Any of PFV/FM_V2/CV/RG `owner()` returns a different address than Timelock | INCIDENT. |

## 10. Frontend / admin abort conditions

| Condition | Action |
|---|---|
| Admin auth proxy compromised | Rotate admin tokens; defer canary. |
| `/admin/status` returns unexpected booleans | Investigate. |
| Public-facing frontend reachable from mainnet without operator-controlled SSR / proxy | Defer canary; complete `FRONTEND-V2G-W3-SSR-PROXY` first. |

## 11. Cross-links

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — missing values.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — verification commands.
* `MAINNET_NEXT_SAFE_MILESTONES.md` — milestone DAG.
* `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` — next-task prompt.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md §3` — rollback plan.
* `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §3 + §4` —
  emergency compromise + outage.
