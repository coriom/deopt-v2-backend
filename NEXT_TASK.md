# NEXT_TASK.md — Tiny Option Trade Preflight With FeesManagerV2 Enabled V2E-F

## Context

FeesManagerV2 is now deployed, wired, indexed by backend config, and enabled on NEW MarginEngine.

Live state:
- NEW_MARGIN_ENGINE = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
- FEES_MANAGER_V2 = 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
- NEW.feesManagerV2() = FEES_MANAGER_V2
- NEW.useFeesManagerV2() = true
- FeesManagerV2.isFeeConsumer(NEW) = true
- FeesManagerV2.merkleRoot() = bytes32(0)
- FeesManagerV2.rebateBudget(BASE_COLLATERAL_TOKEN) = 0
- Launch policy = Tier0 only
- Negative maker tiers unreachable
- Rebate probe deferred

V2E-E enable tx:
0x10c1acff8c496ee5b056b4cddb890bfdaef195569d7f16d04e12b6b6761a142d

Goal:
Prepare a tiny option trade preflight with FeesManagerV2 enabled.

This task must stop before broadcast.

Expected fee behavior:
- V2 fee path is used.
- Tier0 positive fee behavior only.
- FeeChargedV2 should be expected after live broadcast.
- No FeeRebatedV2 expected.
- No rebate budget needed.
- No Merkle claim.
- No negative maker tier reachable.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not call `/options/execution-intents/:id/broadcast`.
Do not call `/executor/broadcast`.
Do not call `eth_sendRawTransaction`.
Do not deploy.
Do not modify Solidity.
Do not modify frontend.
Do not disable FeesManagerV2.
Do not fund rebate budget.
Do not set Merkle root.
Do not create more than one fresh valid tiny intent.
Do not cleanup historical rows.
Do not print private keys.
Do not commit real `.env`.

Allowed:
- backend runtime env update.
- backend restart.
- read-only cast calls.
- oracle refresh using established testnet mock feed script only if stale.
- create one fresh tiny option execution intent.
- sign buyer/seller payloads using existing flow.
- calldata generation.
- simulation.
- gas estimate/safety preview.
- docs.

## Repo

Work in:

```text
~/DEOPT/deopt-v2-backend
Step 1 — Runtime Env

Load env without printing secrets:

cd ~/DEOPT/deopt-v2-backend
set -a
source .env.cutover.v2d_s.local 2>/dev/null || source .env
set +a

Export:

export RPC_URL="<paid Base Sepolia RPC already set in shell>"

export MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
export OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc

export FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
export OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f

export OPTION_EVENT_INDEXER_ENABLED=true
export OPTION_EVENT_INDEXER_REQUIRE_RPC=true
export OPTION_EVENT_INDEXER_CONFIRMATION_BLOCKS=3
export OPTION_EVENT_INDEXER_BATCH_BLOCKS=5000

export OPTION_CONFIRMATION_WORKER_ENABLED=true
export OPTION_RECONCILIATION_WORKER_ENABLED=true
export OPTION_NONCE_SYNC_ENABLED=true
export OPTION_NONCE_SYNC_REQUIRE_RPC=true
export OPTION_NONCE_SYNC_STRICT=true

export OPTION_EXECUTION_BROADCAST_ENABLED=true
export EXECUTION_ENABLED=true
export EXECUTOR_REAL_BROADCAST_ENABLED=true
export EXECUTOR_DRY_RUN=false

export OPTION_EXECUTION_BROADCAST_GAS_LIMIT=1500000
export OPTION_EXECUTION_GAS_SAFETY_BPS=12500
export EXECUTOR_MAX_FEE_PER_GAS_WEI=1000000000
export EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI=1000000

For this preflight, if agent cannot access EXECUTOR_PRIVATE_KEY, it must not ask for it in chat. Use preflight mode without broadcast surfaces if needed, or ask operator to launch backend manually with the key in shell.

Step 2 — Baseline

Set:

V2E_F_START_MS=$(date +%s%3N)

Record DB counts:

select count(*) as option_execution_intents from option_execution_intents;
select count(*) as option_execution_transactions from option_execution_transactions;
select count(*) as execution_transactions from execution_transactions;
select count(*) as option_execution_events from option_execution_events;
select count(*) as option_execution_reconciliations from option_execution_reconciliations;
select count(*) as fee_events from fee_events;

select last_indexed_block, last_error
from option_event_indexer_state
order by updated_at_ms desc
limit 5;
Step 3 — Start Backend

Start/restart backend.

Verify:

curl -s http://127.0.0.1:8080/health

curl -s http://127.0.0.1:8080/admin/config \
  -H "X-Admin-Token: $ADMIN_TOKEN"

Expected:

backend healthy.
margin engine = NEW.
feesManagerV2/indexer emitter = FEES_MANAGER_V2.
broadcast surfaces may be enabled for preflight, but must not be called.
no secrets exposed.
Step 4 — On-chain V2 Checks

Run read-only checks:

cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "feesManagerV2()(address)" --rpc-url "$RPC_URL"
cast call 0x287Cef479be5889eEfCa847F9e73C860898f48Cc "useFeesManagerV2()(bool)" --rpc-url "$RPC_URL"

cast call 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f "isFeeConsumer(address)(bool)" 0x287Cef479be5889eEfCa847F9e73C860898f48Cc --rpc-url "$RPC_URL"
cast call 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f "merkleRoot()(bytes32)" --rpc-url "$RPC_URL"
cast call 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f "rebateBudget(address)(uint256)" <BASE_COLLATERAL_TOKEN> --rpc-url "$RPC_URL"

Expected:

feesManagerV2 = FEES_MANAGER_V2.
useFeesManagerV2 = true.
isFeeConsumer(NEW) = true.
merkleRoot = bytes32(0).
rebateBudget = 0.
Step 5 — Oracle Freshness

Check WETH/USDC oracle freshness with the existing read-only/preflight method.

If stale:

refresh testnet mock feeds using established script.
re-check immediately.

Abort if oracle cannot be fresh.

Step 6 — Create Fresh Tiny Intent

Create one fresh tiny option execution intent using the established option endpoint flow.

Requirements:

fresh buyer/seller nonces from on-chain sync.
quantity = 1.
premium high enough to produce a non-zero positive Tier0 V2 fee if possible.
no rebate expectation.
existing listed valid option series.
buyer/seller funded.

Important:
If premium is too small and fees round to zero, document it and either:

keep it as a no-fee V2 path preflight, or
create a replacement intent only if the task explicitly records the first one as stale/invalid and does not exceed one fresh valid broadcast candidate.

Preferred:

one final valid candidate with expected positive FeeChargedV2.

Record:

intent_id.
buyer/seller.
buyer/seller nonces.
option_id.
quantity.
premium.
expected V2 fee behavior.
Step 7 — Signatures / Calldata / Simulation

Use existing flow:

fetch EIP-712 payload.
sign buyer.
sign seller.
submit strict signatures.
fetch calldata.
simulate.

Expected:

signatures accepted.
calldata ready.
simulation_status = simulation_ok.
no revert.
simulation targets OptionMatchingEngine -> NEW MarginEngine.
expected V2 fee path.
Step 8 — Gas Safety Preview

Estimate gas and compute:

required_gas = estimated_gas * EXECUTION_GAS_SAFETY_BPS / 10000

Expected:

OPTION_EXECUTION_BROADCAST_GAS_LIMIT >= required_gas.
gas safety status = ok.
executor balance enough.
Step 9 — No Broadcast / No Forbidden Mutation Check

Allowed:

one new option_execution_intent.
signature/calldata/simulation records for that intent.

Forbidden:

no option_execution_transactions.
no execution_transactions.
no broadcast tx hash.
no confirmed/reconciled row for new intent.

Query:

select count(*) from option_execution_transactions where created_at_ms >= ${V2E_F_START_MS};
select count(*) from execution_transactions where created_at_ms >= ${V2E_F_START_MS};

Expected:

both 0.
Required Docs

Create:

docs/FEES_MANAGER_V2_TINY_TRADE_PREFLIGHT_V2E_F.md

Update if useful:

docs/FEES_MANAGER_V2_ENABLE_BROADCAST_RESULT_V2E_E.md

Include:

runtime config summary.
baseline DB counts.
on-chain V2 checks.
oracle freshness.
fresh intent id.
expected fee behavior.
signature/calldata status.
simulation result.
gas safety preview.
no-broadcast proof.
remaining blocker before human V2 tiny broadcast.
Validation

Run:

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
Acceptance Criteria

Complete only if:

backend reports NEW + FeesManagerV2.
on-chain useFeesManagerV2 = true.
oracle fresh.
one final valid tiny intent exists.
signatures accepted.
calldata ready.
simulation_ok.
gas safety ok.
no broadcast endpoint called.
no option_execution_transactions created.
no generic execution_transactions created.
docs created.
validations pass.
Final Report

Return:

backend config summary.
on-chain V2 checks.
oracle status.
tiny intent id.
expected V2 fee behavior.
signature/calldata status.
simulation result.
gas safety preview.
no-broadcast verification.
docs updated.
validation commands run.
remaining blocker before human V2 tiny broadcast.