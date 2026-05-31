# NEXT_TASK.md — Fee Observability, Env Hygiene And Executor Readiness Closure V2G-F

## Context

V2G-E completed the live PERP + OPTION rebate smoke campaign.

Validated:

* PERP FeeChargedV2 + FeeRebatedV2 live.
* OPTION FeeChargedV2 + FeeRebatedV2 live.
* Tier4 maker and Tier2 taker claims live.
* Merkle root and claim path live.
* Rebate budget path live.
* Backend `/admin/fees/onchain` decodes both PERP and OPTION live rebate txs.
* PERP metrics expose charged/rebated counters for `consumer="new"`.
* OLD stranded PerpEngine alert stays green.

Known remaining blockers:

1. Env hygiene:

   * committed/backend env still has `PERP_ENGINE_ADDRESS=OLD_PERP_ENGINE`.
   * missing explicit `OLD_PERP_ENGINE_ADDRESS`.
   * V2G-E used shell-only `PERP_ENGINE_ADDRESS=NEW` override.
2. OPTION metrics:

   * no `deopt_option_fee_charged_v2_total`.
   * no `deopt_option_fee_rebated_v2_total`.
   * OPTION is visible through `/admin/fees/onchain`, but not metrics.
3. Alerting:

   * V2F-Q rules exist, but live alert delivery / final rules cleanup still needs closure.
   * merkle-root-unset operational notice is obsolete now that root is live.
4. Backend executor:

   * backend `.env` BUYER/SELLER are not the V2G-D2 EOAs.
   * V2G-E used Solidity-script signing path.
   * future real-trader smokes need a clean non-secret signing-key surface or explicit operator-run packet.

Goal:
Close observability, env hygiene, alerting, and executor-readiness gaps after V2G-E.

This is an accelerated module. Do not split into microtasks. Complete all safe backend/docs/tests/config-example changes in one pass.

## Hard Rules

Do not broadcast.
Do not submit transactions.
Do not mutate live chain.
Do not print private keys.
Do not edit real secret `.env` files unless the file is explicitly a committed non-secret example/template.
Do not delete DB rows.
Do not hide OLD_PERP_ENGINE stranded state.
Do not weaken V2G-E results.
Do not alter Merkle root, rebate budget, fee schedules, or deployed contracts.

Allowed:

* backend code changes.
* metrics code.
* alert rule/docs.
* `.env.example` / `.env.*.example` / template files.
* docs.
* tests.
* read-only live checks.
* local backend/admin smoke if no secrets printed.

## Live References

Contracts:

* FEES_MANAGER_V2 = `0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`
* NEW_PERP_ENGINE = `0xc6C592100723Fe0C66343A16e95eC34cC0c2141c`
* OLD_PERP_ENGINE = `0xB36395b67D0798ADA981731c9Fa5239F4362b53B`
* NEW_MARGIN_ENGINE = `0x287Cef479be5889eEfCa847F9e73C860898f48Cc`
* mUSDC = `0x6eAe407f5640B006faC9965182e238582A3B412E`

Live rebate txs:

* PERP rebate tx = `0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394`
* OPTION rebate tx = `0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149`

Expected V2G-E accounting:

* PERP:

  * FeeChargedV2 fee = `6`
  * FeeRebatedV2 rebate = `3`
  * rebateBudget `1_000_000 -> 999_997`
* OPTION:

  * FeeChargedV2 fee = `25`
  * FeeRebatedV2 rebate = `10`
  * rebateBudget `999_997 -> 999_987`

## Phase 1 — Audit Current Env + Config Hygiene

Audit backend env/config references:

```bash
rg -n "PERP_ENGINE_ADDRESS|OLD_PERP_ENGINE|perp_engine|fees_manager_v2|OPTION.*ENGINE|BUYER|SELLER" . src docs
```

Determine:

* where `PERP_ENGINE_ADDRESS` is read.
* whether runtime uses OLD or NEW by default.
* whether `.env.example` points to OLD.
* whether docs instruct shell-only override.
* whether `OLD_PERP_ENGINE_ADDRESS` is already supported.
* whether option/margin engine config has equivalent explicit current/old split.

Required outcome:

* NEW_PERP_ENGINE must be the canonical current engine in examples/templates.
* OLD_PERP_ENGINE must be explicitly represented as stranded metadata.
* real secret `.env` must not be edited by the agent unless explicitly safe and non-secret.
* provide exact operator diff/instruction if real `.env` must be changed manually.

## Phase 2 — Env Template Cleanup

Update committed examples/templates only.

Required:

* `PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c`
* add `OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B`
* ensure `FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`
* ensure `MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc`
* document OLD as stranded, not active.

If the only file containing these is real `.env`, do not edit it. Create:

* `.env.base-sepolia.v2g_f.example`
  or update existing `.env.example`.

Add docs explaining the manual operator patch for local `.env`.

## Phase 3 — OPTION V2 Metrics

Mirror PERP V2 metrics for OPTION.

Add metrics:

```text
deopt_option_fee_charged_v2_total{consumer="new"|"old"|"unknown"}
deopt_option_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}
```

Classification:

* productKind must be OPTION.
* FeeChargedV2 increments charged metric.
* FeeRebatedV2 increments rebated metric.
* consumer classification:

  * NEW_MARGIN_ENGINE => `new`
  * known old margin engine if configured => `old`
  * otherwise => `unknown`
* no raw address labels.
* pre-seed new/old/unknown buckets.
* preserve PERP metrics behavior.

If no old margin engine exists:

* support optional `OLD_MARGIN_ENGINE_ADDRESS`.
* if unset and consumer != NEW, classify unknown.

Tests required:

1. OPTION FeeChargedV2 from NEW_MARGIN increments `consumer="new"`.
2. OPTION FeeRebatedV2 from NEW_MARGIN increments `consumer="new"`.
3. OPTION events from unknown consumer increment `unknown`.
4. OPTION events do not affect PERP metrics.
5. PERP events do not affect OPTION metrics.
6. raw addresses do not appear in metric labels.
7. empty metrics expose zero buckets if current PERP implementation does that.

## Phase 4 — Alert Rules Cleanup

Update alert rules/docs:

* PERP old-consumer charged alert remains.
* PERP old-consumer rebated alert remains.
* add OPTION unknown-consumer alert.
* add OPTION old-consumer alert only if old margin config exists; otherwise document unknown only.
* retire or downgrade merkle-root-unset operational notice because root is now live.
* add rebate-budget-low alert if not already present:

```yaml
alert: FeesManagerV2RebateBudgetLow
expr: deopt_fees_manager_v2_rebate_budget_native{asset="mUSDC"} < 1000
for: 0m
labels:
  severity: medium
annotations:
  summary: "FeesManagerV2 rebate budget is low"
```

If that budget metric does not exist:

* either implement it from existing on-chain summary path, or document as future if implementation would be too large.
* do not fake it.

## Phase 5 — Backend Executor Readiness For V2G-D2 EOAs

Audit backend signing/executor config:

* buyer key env name.
* seller key env name.
* option executor signing path.
* perp executor signing path.
* whether it can load arbitrary operator-provided keys without writing them to `.env`.

Goal:
Produce a safe readiness pattern for future smoke:

* keys loaded only from shell.
* no printing.
* no committed secret.
* explicit addresses derived and checked.
* admin endpoint or CLI can sign with supplied shell keys if already supported.

If backend does not support this safely:

* document exact blocker.
* propose minimal future task.
* do not implement invasive secret handling unless small and safe.

## Phase 6 — Live Read-Only Verification

Run read-only verification if backend can be started safely without secrets.

Verify:

* `/admin/fees/onchain?tx_hash=<PERP_TX>`
* `/admin/fees/onchain?tx_hash=<OPTION_TX>`
* `/metrics` includes:

  * PERP charged/rebated new.
  * OPTION charged/rebated new.
  * old/unknown zero unless justified.
* OLD stranded alert green.
* no double counting.

Do not mutate DB destructively.
If indexer catch-up is needed:

* only use existing safe admin tick if it is read-only/idempotent.
* document exact command and result.

## Phase 7 — Docs

Create:

```text
docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md
```

Update:

* `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md`
* `docs/ALERTING_SPEC.md`
* `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`
* add or update option fee alert runbook if needed.

Include:

* env hygiene diff.
* active vs stranded engines.
* OPTION metrics.
* alert status.
* executor-readiness status.
* live read-only verification.
* remaining blockers.

## Phase 8 — Validation

Backend:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --no-fail-fast
cargo build --all-targets --all-features
```

Frontend if touched:

```bash
npm run lint
npx tsc --noEmit
npm run build
```

Solidity if touched:

```bash
forge fmt
forge fmt --check
forge build
forge test --no-match-path 'test/fork/*'
```

## Acceptance Criteria

Complete if:

* env examples/templates point to NEW_PERP_ENGINE.
* OLD_PERP_ENGINE is explicit stranded metadata.
* real `.env` untouched or operator-only patch documented.
* OPTION charged/rebated V2 metrics exist.
* OPTION metrics tests pass.
* PERP metrics remain passing.
* alert docs/rules updated.
* merkle-root-unset notice retired/downgraded.
* backend executor signing readiness documented.
* live read-only admin/metrics verification attempted or exact blocker documented.
* validations pass.

## Final Report

Return:

* env hygiene changes.
* option metrics implementation.
* alerting changes.
* executor-readiness result.
* live verification result.
* files changed.
* tests added.
* docs updated.
* validation commands run.
* remaining blockers.
* next recommended milestone.
