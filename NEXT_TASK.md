# NEXT_TASK.md — FeesManagerV2 Deploy/Wire Preflight Against NEW MarginEngine V2E-A

## Context

V2D-V validated the full V1-fee option execution path against NEW MarginEngine.

Current live state:
- OLD_MARGIN_ENGINE = 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
- NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
- OptionMatchingEngine.marginEngine = NEW
- CollateralVault.marginEngine = NEW
- RiskModule.marginEngine = NEW
- MatchingEngine.marginEngine = NEW
- InsuranceFund NEW backstop enabled
- RiskGovernor.marginEngine = NEW
- backend MARGIN_ENGINE = NEW
- event indexer cursor past NEW trade block
- tiny trade against NEW succeeded and reconciled

V2D-V tx:
0x07a8e6795e2082ceabaa242543ee424cffd5037c0d918cf1a81bcee1b2d7de10

V2D-V result:
- receipt status = 1
- lifecycle = reconciled
- events indexed = 7
- NEW MarginEngine event indexed
- FeesManagerV2 disabled:
  - NEW.feesManagerV2 = address(0)
  - NEW.useFeesManagerV2 = false
- V1 fee branch production-validated against NEW

Goal:
Prepare FeesManagerV2 deploy/wire/enable preflight against NEW MarginEngine.

This task must not broadcast.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not deploy live.
Do not call scripts with `--broadcast`.
Do not enable FeesManagerV2 live.
Do not call `setUseFeesManagerV2` live.
Do not modify frontend.
Do not modify deployed contracts.
Do not create option execution intents.
Do not create option execution transactions.
Do not call option broadcast endpoints.
Do not print private keys.
Do not commit real `.env`.

Allowed:
- Solidity script dry-runs.
- read-only cast calls.
- docs/config templates.
- backend env plan documentation.

## Repo

Work in:

```text
~/DEOPT/deopt-v2-sol
Required Existing Files

Read:

src/fees/FeesManagerV2.sol
src/fees/IFeesManagerV2.sol
src/margin/MarginEngineAdmin.sol
script/DeployFeesManagerV2.s.sol
script/WireFeesManagerV2Option.s.sol
docs/FEES_MANAGER_V2_DEPLOYMENT_PREFLIGHT_V2D_F.md
docs/MARGIN_ENGINE_V2_TINY_TRADE_BROADCAST_RESULT_V2D_V.md
Launch Policy

Use conservative launch policy:

Launch mode = Tier0 only
Merkle root = bytes32(0)
Initial rebate budget = 0
Negative maker tiers unreachable
Positive-fee probe only
Rebate probe deferred
Required Address Decisions

Use:

FEES_MANAGER_V2_FEE_RECIPIENT = ProtocolTimelock if already ratified in V2D-I
FEES_MANAGER_V2_REBATE_FUNDING_ACCOUNT = same as feeRecipient
NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc

If ProtocolTimelock address is not present in docs/manifest, audit and record it.
Do not invent unknowns.

Step 1 — Live Read-only Checks

Load env without printing secrets:

cd ~/DEOPT/deopt-v2-sol
set -a
source .env.base-sepolia
set +a

Verify:

cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "owner()(address)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManagerV2()(address)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "useFeesManagerV2()(bool)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManager()(address)" --rpc-url "$RPC_URL"

Expected:

owner = deployer/governance owner
feesManagerV2 = address(0)
useFeesManagerV2 = false
feesManager = V1 manager

Abort on mismatch.

Step 2 — Deploy FeesManagerV2 Dry-run

Set env:

export DEPLOY_FEES_MANAGER_V2_CONFIRM=true
export FUND_REBATE_BUDGET_CONFIRM=false

export FEES_MANAGER_V2_FEE_RECIPIENT=<ProtocolTimelock>
export FEES_MANAGER_V2_REBATE_FUNDING_ACCOUNT=<ProtocolTimelock>

Run without broadcast:

forge script script/DeployFeesManagerV2.s.sol:DeployFeesManagerV2 \
  --rpc-url "$RPC_URL"

Expected:

deploy simulation complete.
no broadcast.
feeRecipient correct.
rebateFundingAccount correct.
rebateBudget = 0.
owner correct.
no MarginEngine wiring.
Step 3 — Wire FeesManagerV2 Dry-run

If deploy dry-run predicts a deterministic address, use it. Otherwise use placeholder and document that live deployed address is required after Phase 1 deploy.

Set env:

export FEES_MANAGER_V2_ADDRESS=<predicted-or-placeholder>
export MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export WIRE_FEES_MANAGER_V2_CONFIRM=true
export ENABLE_FEES_MANAGER_V2_CONFIRM=false

Run without broadcast:

forge script script/WireFeesManagerV2Option.s.sol:WireFeesManagerV2Option \
  --rpc-url "$RPC_URL"

Expected:

if address exists only in dry-run context, document limitation.
planned calls:
FeesManagerV2.setFeeConsumer(NEW_MARGIN_ENGINE,true)
NEW_MARGIN_ENGINE.setFeesManagerV2(FEES_MANAGER_V2_ADDRESS)
no setUseFeesManagerV2 when enable confirm=false.
useFeesManagerV2 remains false.
Step 4 — Enable Dry-run Separately

Only simulate separate enable phase.

Set:

export ENABLE_FEES_MANAGER_V2_CONFIRM=true

Run wire script dry-run again without broadcast.

Expected:

includes NEW_MARGIN_ENGINE.setUseFeesManagerV2(true)
no other unexpected calls.
document as separate future phase.
Required Backend Env Plan

After live FeesManagerV2 deploy, backend must set:

OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=<deployed FeesManagerV2>
FEES_MANAGER_V2=<deployed FeesManagerV2>

Do not modify backend code in this task.

Required Docs

Create:

docs/FEES_MANAGER_V2_NEW_MARGIN_ENGINE_PREFLIGHT_V2E_A.md

Update:

docs/FEES_MANAGER_V2_DEPLOYMENT_PREFLIGHT_V2D_F.md
docs/MARGIN_ENGINE_V2_TINY_TRADE_BROADCAST_RESULT_V2D_V.md if useful

Include:

live NEW checks.
deploy dry-run result.
wire dry-run result.
enable dry-run result.
launch policy.
backend env plan.
final human approval prompts for:
deploy FeesManagerV2 only
wire FeesManagerV2 only
enable FeesManagerV2 only
no-broadcast proof.
Validation

Run:

forge fmt --check
forge build
forge test
Acceptance Criteria

Complete only if:

NEW engine live reads pass.
FeesManagerV2 deploy dry-run passes.
wire dry-run prepared.
enable dry-run prepared as separate phase.
no live broadcast.
no live chain mutation.
docs created.
validation passes.
Final Report

Return:

NEW engine checks.
feeRecipient / rebateFundingAccount decision.
deploy dry-run result.
wire dry-run result.
enable dry-run result.
backend env plan.
docs updated.
validation commands run.
remaining blocker before FeesManagerV2 deploy.