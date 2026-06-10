# Mainnet read-only preflight checklist

**Posture:** DOC ONLY. READ-ONLY verification commands. **No
transaction. No broadcast. No Safe-tx. No state mutation. No AWS
resource creation. No `.env` edit. No secret printed.**

**Companion:** `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`.

## 0. Hard rules (this checklist)

```text
no transaction on any chain                   ✅
no Safe transaction                           ✅
no AWS resource creation                      ✅
no AWS CLI apply/create/update/delete         ✅
no Terraform apply                            ✅
no `.env` edit                                ✅
no signer call that emits a Sign request      ✅
   (GetPublicKey is allowed; Sign is NOT)
no contract state mutation                    ✅
no fund movement                              ✅
no secret printed                             ✅
no production address committed to logs       ✅
```

Every command below is a `view` / `read` operation. None mutates
state on any chain or in any cloud account.

## 1. Custody — Safe + Timelock read-only checks

### 1.1 OPS Safe

| # | Check | Command (placeholder) | Expected |
|---|---|---|---|
| C1 | OPS Safe exists on Base mainnet | `cast call --rpc-url <MAINNET_RPC> 0xce0e46Db1072B820CB5eCf30188ED76cb560C932 "VERSION()(string)"` | Returns Safe contract version string (e.g. `1.4.1`). |
| C2 | OPS Safe threshold | `cast call --rpc-url <MAINNET_RPC> 0xce0e46Db1072B820CB5eCf30188ED76cb560C932 "getThreshold()(uint256)"` | `2` |
| C3 | OPS Safe owner count | `cast call --rpc-url <MAINNET_RPC> 0xce0e46Db1072B820CB5eCf30188ED76cb560C932 "getOwners()(address[])"` then count | `3` |
| C4 | OPS Safe nonce | `cast call --rpc-url <MAINNET_RPC> 0xce0e46Db1072B820CB5eCf30188ED76cb560C932 "nonce()(uint256)"` | Operator-observed; recorded in preflight notes. |

### 1.2 GOV Safe

| # | Check | Command | Expected |
|---|---|---|---|
| C5 | GOV Safe exists | `cast call --rpc-url <MAINNET_RPC> 0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166 "VERSION()(string)"` | Returns version. |
| C6 | GOV Safe threshold | `cast call ... "getThreshold()(uint256)"` | `3` |
| C7 | GOV Safe owner count | `cast call ... "getOwners()(address[])"` then count | `5` |
| C8 | GOV Safe nonce | `cast call ... "nonce()(uint256)"` | Operator-observed. |

### 1.3 Owner overlap + DEPLOYER non-membership

| # | Check | How | Expected |
|---|---|---|---|
| C9 | OPS / GOV owner overlap | Compute set intersection of OPS owners and GOV owners. | `0` (no overlap, per Cluster 1 result). |
| C10 | DEPLOYER not OPS owner | Check DEPLOYER address is NOT in OPS getOwners(). | `false` for membership. |
| C11 | DEPLOYER not GOV owner | Same for GOV. | `false`. |

### 1.4 Treasury Safe (if created by `MAINNET-TREASURY-SAFE-CREATION-PACKET`)

| # | Check | Expected |
|---|---|---|
| C12 | Treasury Safe address recorded in operator binder | Operator confirms. |
| C13 | Treasury Safe threshold | Per operator policy. |
| C14 | Treasury Safe owner roster disjoint from OPS / GOV | Operator confirms 0 overlap. |

### 1.5 Timelock (if deployed by `MAINNET-V2G-Y-OWNERSHIP-MIGRATION`)

| # | Check | Command | Expected |
|---|---|---|---|
| T1 | Timelock instance exists | `cast call --rpc-url <MAINNET_RPC> <TIMELOCK_ADDRESS> "getMinDelay()(uint256)"` | Per Q-CD-G6 decision. |
| T2 | PROPOSER_ROLE held by OPS Safe | `cast call ... "hasRole(bytes32,address)(bool)" <PROPOSER_ROLE_HASH> 0xce0e46Db…0C932` | `true`. |
| T3 | EXECUTOR_ROLE held by OPS Safe | `cast call ... "hasRole(bytes32,address)(bool)" <EXECUTOR_ROLE_HASH> 0xce0e46Db…0C932` | `true`. |
| T4 | TIMELOCK_ADMIN_ROLE held by Timelock itself | `cast call ... "hasRole(bytes32,address)(bool)" <TIMELOCK_ADMIN_ROLE_HASH> <TIMELOCK_ADDRESS>` | `true`. |
| T5 | DEPLOYER NOT holding any Timelock role | All 3 hasRole calls against DEPLOYER → `false`. | Confirmed. |

## 2. Contracts — read-only verification

### 2.1 OptionMatchingEngine (OME)

| # | Check | Command | Expected |
|---|---|---|---|
| O1 | OME owner | `cast call --rpc-url <MAINNET_RPC> <OME_ADDRESS> "owner()(address)"` | Should be Timelock per `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`. |
| O2 | OME guardian (per design) | `cast call ... "guardian()(address)"` | Per operator policy. |
| O3 | OME paused | `cast call ... "paused()(bool)"` | `false` (operational). |
| O4 | OME isExecutor(BE) | `cast call ... "isExecutor(address)(bool)" <BE_address>` | `true` after `setExecutor` Safe-tx; `false` before. |
| O5 | OME setExecutor caller is Timelock-only | Read source — operator confirms `onlyOwner`. | Confirmed. |

### 2.2 ProtocolFeeVault (PFV)

| # | Check | Command | Expected |
|---|---|---|---|
| P1 | PFV owner | `cast call ... "owner()(address)" <PFV_ADDRESS>` | Timelock. |
| P2 | PFV guardian | `cast call ... "guardian()(address)"` | Per operator policy. |
| P3 | PFV paused | `cast call ... "paused()(bool)"` | `false`. |
| P4 | PFV feeBalance(asset) | `cast call ... "feeBalance(address)(uint256)" <USDC_ADDRESS>` | `0` at launch (no fees collected yet). |
| P5 | PFV rebateReserve(asset) | `cast call ... "rebateReserve(address)(uint256)" <USDC_ADDRESS>` | `0` per Cluster 4 launch invariant. |
| P6 | PFV rebatesPaused | `cast call ... "rebatesPaused()(bool)"` | Per operator policy at launch. |

### 2.3 FeesManagerV2

| # | Check | Command | Expected |
|---|---|---|---|
| F1 | FM_V2 owner | `cast call ... "owner()(address)" <FM_V2_ADDRESS>` | Timelock. |
| F2 | FM_V2 paused (if implemented) | `cast call ... "paused()(bool)"` | `false`. |
| F3 | FM_V2 rebateBudget(asset) | `cast call ... "rebateBudget(address)(uint256)" <USDC_ADDRESS>` | `0` per Cluster 4 launch invariant. |
| F4 | FM_V2 quoteFees(...) | Read-only ABI call with maker/taker test inputs. | Returns FeeQuote struct without revert. |

### 2.4 CollateralVault (CV)

| # | Check | Command | Expected |
|---|---|---|---|
| V1 | CV owner | `cast call ... "owner()(address)" <CV_ADDRESS>` | Timelock. |
| V2 | CV balances(PFV, asset) | `cast call ... "balances(address,address)(uint256)" <PFV_ADDRESS> <USDC_ADDRESS>` | `0` at launch (must equal `PFV.feeBalance + PFV.rebateReserve` per R5). |

### 2.5 RiskGuardian (RG)

| # | Check | Expected |
|---|---|---|
| R1 | RG owner | Timelock. |
| R2 | RG guardian | Per operator policy. |
| R3 | RG paused | `false`. |

### 2.6 Cluster 4 launch invariant verification

| # | Check | How |
|---|---|---|
| LI1 | `PFV.rebateReserve(asset) == 0` for every configured asset | Per asset in `PROTOCOL_FEE_VAULT_RECONCILIATION_ASSETS`. |
| LI2 | `FM_V2.rebateBudget(asset) == 0` for every configured asset | Same. |
| LI3 | `CV.balances(PFV, asset) == 0` for every configured asset | Same. |
| LI4 | R5 drift = 0 | Computed: `CV.balances(PFV, asset) - (PFV.feeBalance(asset) + PFV.rebateReserve(asset))` = `0` for every asset. |

## 3. Backend health / observability

| # | Check | Command | Expected |
|---|---|---|---|
| H1 | Backend process up | `curl -sf https://<BACKEND_HOST>/health` | `{"ok":true,"service":"deopt-v2-backend"}` |
| H2 | Backend ready | `curl -sf https://<BACKEND_HOST>/ready` | HTTP 200; readiness JSON returns `"ready":true`. |
| H3 | `/executor/health/v2.overall_status` | `curl ... /executor/health/v2 \| jq -r .overall_status` | `"green"`. |
| H4 | `signer.signer_mode` | `... \| jq -r .signer.signer_mode` | `"remote"`. |
| H5 | `signer.remote_signer_configured` | `... \| jq -r .signer.remote_signer_configured` | `true`. |
| H6 | `signer.signer_address` matches `EXECUTOR_FROM_ADDRESS` | Cross-check with operator binder. | Match. |
| H7 | `signer.local_signer_on_mainnet_refused_total` | `... \| jq -r .signer.local_signer_on_mainnet_refused_total` | `0`. |
| H8 | `policy_gate.last_policy_data_failure_type` | `... \| jq -r .policy_gate.last_policy_data_failure_type` | `null` at launch (no broadcast attempts yet). |
| H9 | `live_provider_config.protocol_fee_vault_configured` | `... \| jq -r .live_provider_config.protocol_fee_vault_configured` | `true`. |
| H10 | `live_provider_config.fees_manager_v2_configured` | Same field. | `true`. |
| H11 | `live_provider_config.collateral_vault_configured` | Same. | `true`. |
| H12 | `not_tracked_yet` array is empty | `... \| jq '.not_tracked_yet \| length'` | `0`. |
| H13 | `chain_state_last_seen.be_balance_floor_wei` | `... \| jq -r .chain_state_last_seen.be_balance_floor_wei` | Bounded; matches the configured gas budget. |
| H14 | `r5.drift_observed_total` | `... \| jq -r .r5.drift_observed_total` | `0`. |
| H15 | `/metrics` Prometheus scrape | `curl -sf -H "x-admin-token: <…>" https://<BACKEND_HOST>/metrics` | Returns Prometheus text output. |
| H16 | No `signer_denied_total` increments | `grep deopt_option_broadcast_signer_denied_total /tmp/metrics` | Empty or zero. |
| H17 | No `fm_v2_*_failures_total` increments | Same grep pattern. | Zero. |
| H18 | No `policy_data_failures_total` increments | Same. | Zero. |
| H19 | No `r5_drift_observed_total` increments | Same. | Zero. |

## 4. Signer health (no-sign mode)

`health_check` calls `kms:GetPublicKey` ONLY (per design — never a
`kms:Sign`). Backend exposes this via the `RemoteSigner::health_check`
trait method.

| # | Check | How | Expected |
|---|---|---|---|
| S1 | Signer `health_check` call returns Ok(SignerHealth) | Operator drives via dedicated admin endpoint OR via backend startup log. | `health=true; mode=remote; signer_address=<expected>`. |
| S2 | Signer `health_check` does NOT issue a Sign | CloudTrail event lookup for the window shows ONLY `GetPublicKey` events; ZERO `Sign` events. | Confirmed. |
| S3 | Recovered EVM address matches `EXECUTOR_FROM_ADDRESS` | Cross-check. | Match. |

## 5. AWS KMS read-only checks (only if operator AWS ready)

These commands are READ-ONLY. They DO NOT create or modify any AWS
resource. They are run from an operator-controlled console session
with the `<KMS_ADMIN_ROLE_NAME>` or read-only auditor role per
`AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md §2`.

| # | Check | Command | Expected |
|---|---|---|---|
| K1 | KMS key state | `aws kms describe-key --key-id <KMS_KEY_ID_OR_ALIAS>` | `KeyState=Enabled`, `KeySpec=ECC_SECG_P256K1`, `KeyUsage=SIGN_VERIFY`, `Origin=AWS_KMS`. |
| K2 | KMS key region matches | Inspect returned `Arn` region. | Matches operator's selected region per `AWS_KMS_OPERATOR_SETUP_PACK.md §3`. |
| K3 | Public key extractable | `aws kms get-public-key --key-id <KMS_KEY_ID_OR_ALIAS>` | Returns SPKI bytes; can decode offline. |
| K4 | Derived EVM address matches `EXECUTOR_FROM_ADDRESS` | Operator offline derives + cross-checks. | Match. |
| K5 | CloudTrail captures K1 + K3 | Operator queries CloudTrail event lookup. | Both events recorded with attributable IAM principal. |
| K6 | NO `Sign` call made by this checklist | Operator confirms intent. | Confirmed. |
| K7 | `iam:simulate-principal-policy` simulation of runtime role | Operator simulates allow + deny per `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md §3 P13-P24`. | All checks PASS. |

## 6. Frontend / admin

| # | Check | Expected |
|---|---|---|
| A1 | Admin dashboard URL reachable | HTTP 200; admin token required (returns 403 without token). |
| A2 | `/admin/status` endpoint returns expected booleans | Per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`. |
| A3 | `/admin/options/executions/:intent_id/lifecycle` returns 404 for unknown intent | Confirmed. |
| A4 | `/executor/transactions` list endpoint returns `[]` before any broadcast | Confirmed. |
| A5 | Frontend admin auth proxy in place | Per `ADMIN_FRONTEND_AUTH_PROXY_V2G_W2.md`. |

## 7. Cross-cutting "must remain ZERO at preflight"

| # | Field | Expected |
|---|---|---|
| Z1 | `policy_rejected_total{*}` cumulative | `0` (no broadcast attempts yet). |
| Z2 | `signer_attempted_total{*}` cumulative | `0`. |
| Z3 | `signer_success_total{*}` cumulative | `0`. |
| Z4 | `signer_denied_total{*}` cumulative | `0`. |
| Z5 | `local_signer_on_mainnet_refused_total` | `0`. |
| Z6 | `policy_data_failures_total{*}` cumulative | `0`. |
| Z7 | `fm_v2_decode_failures_total` | `0`. |
| Z8 | `fm_v2_rpc_failures_total` | `0`. |
| Z9 | `r5_drift_observed_total` | `0`. |
| Z10 | `econ_data_available_false_total` | `0`. |
| Z11 | `econ_data_available_true_total` | `0`. |

ANY non-zero in Z1-Z11 at preflight means a broadcast attempt has been
made (intentionally or otherwise). Investigate before proceeding to
mainnet activation.

## 8. Sample read-only script skeleton (placeholder)

A small operator-side script (NOT committed to this repo — lives in
operator's preflight workspace) that runs the §1-§7 checks
mechanically. Skeleton:

```bash
#!/usr/bin/env bash
set -euo pipefail

# Operator fills the placeholders in their own preflight workspace.
RPC_URL="<MAINNET_RPC>"
OPS_SAFE="0xce0e46Db1072B820CB5eCf30188ED76cb560C932"
GOV_SAFE="0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166"
TIMELOCK="<TIMELOCK_ADDRESS_PLACEHOLDER>"
OME="<OME_ADDRESS_PLACEHOLDER>"
PFV="<PFV_ADDRESS_PLACEHOLDER>"
FM_V2="<FM_V2_ADDRESS_PLACEHOLDER>"
CV="<CV_ADDRESS_PLACEHOLDER>"
USDC="<USDC_MAINNET_ADDRESS_KNOWN>"
BACKEND_URL="<BACKEND_HOST>"

echo "=== §1 Custody ==="
cast call --rpc-url "$RPC_URL" "$OPS_SAFE" "getThreshold()(uint256)"
cast call --rpc-url "$RPC_URL" "$OPS_SAFE" "getOwners()(address[])"
cast call --rpc-url "$RPC_URL" "$GOV_SAFE" "getThreshold()(uint256)"
cast call --rpc-url "$RPC_URL" "$GOV_SAFE" "getOwners()(address[])"

echo "=== §2 Contracts ==="
cast call --rpc-url "$RPC_URL" "$OME" "paused()(bool)"
cast call --rpc-url "$RPC_URL" "$PFV" "feeBalance(address)(uint256)" "$USDC"
cast call --rpc-url "$RPC_URL" "$PFV" "rebateReserve(address)(uint256)" "$USDC"
cast call --rpc-url "$RPC_URL" "$FM_V2" "rebateBudget(address)(uint256)" "$USDC"
cast call --rpc-url "$RPC_URL" "$CV" "balances(address,address)(uint256)" "$PFV" "$USDC"

echo "=== §3 Backend health ==="
curl -sf "$BACKEND_URL/health"
curl -sf "$BACKEND_URL/executor/health/v2" | jq '{overall_status, signer, live_provider_config, not_tracked_yet}'
```

This script is illustrative only; operator customises it within
their preflight workspace and never commits it with real values.

## 9. Cross-links

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — placeholder → real
  value map.
* `MAINNET_GO_NO_GO_CRITERIA.md` — what passes / what fails.
* `MAINNET_READ_ONLY_PREFLIGHT_NEXT_TASK.md` — copy/paste prompt to
  run this checklist.
* `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — 43-check AWS preflight.
* `MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — 7-phase rehearsal.
