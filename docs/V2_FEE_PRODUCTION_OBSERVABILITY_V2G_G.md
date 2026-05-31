# V2G-G — V2 Fee Production Observability Closure

## Status

- Milestone: **V2G-G** — productionises the V2 fee observability surface
  closed metric-side in V2G-F (PERP + OPTION metrics, derived rebate
  budget gauge, OLD/unknown classification).
- Date: 2026-05-31.
- Outcome:
  - Deployable Prometheus rules bundle
    (`docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`) consolidating
    every PERP + OPTION + budget alert into one file, plus two V2G-G
    additions (`FeesManagerV2RebateBudgetStale`, `DeoptV2FeeMetricsAbsent`).
  - Example Alertmanager routing config
    (`docs/monitoring/alertmanager/v2_fee_routing.example.yml`) with
    inhibit rule that suppresses downstream alerts when the metric
    pipeline is absent.
  - Grafana dashboard JSON + spec markdown
    (`docs/monitoring/grafana/v2_fee_observability_dashboard.json`,
    `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md`)
    rendering every V2 fee gauge plus anomaly-stat cards and the
    rebate-budget timeseries per asset.
  - New backend admin endpoint
    `GET /admin/fees/v2/observability` — read-only JSON snapshot of the
    V2 fee surface (consumer-bucketed counts + budget + configured
    engine addresses + feature flags); 4 new HTTP tests; 0 raw
    address leaks in the response body.
  - Frontend admin dashboard panel — new "V2 Fee Observability (V2G-G)"
    section reading the new endpoint, with anomaly tiles, four bucket
    cards (PERP charged/rebated, OPTION charged/rebated), a rebate
    budget table by asset, and configured-engine reference tiles.
  - Operator-only `.env` patch template at
    `docs/operator/v2g_g_env_patch.example.env`, including read-only
    verification commands (no private-key handling, no real `.env`
    edits).
  - Mainnet / multi-asset readiness matrix (this file, §"Mainnet /
    multi-asset readiness matrix").
  - Live read-only verification against the V2G-E PERP + OPTION
    rebate transactions reproduced 1:1; all eight V2 alerts
    (3 PERP + 3 OPTION + budget low + metric absent) evaluated **green**.
- Hard gates respected: no broadcasts, no on-chain mutation, no DB
  rows deleted, no real-secret `.env` edited, no Merkle root /
  rebate-budget mutation, no `OLD_PERP_ENGINE` traffic, no governance
  / timelock execution.

## Source-of-truth artefacts

| Artefact | Path |
| -------- | ---- |
| Deployable Prometheus rule bundle    | `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` |
| Alertmanager routing example         | `docs/monitoring/alertmanager/v2_fee_routing.example.yml` |
| Grafana dashboard JSON               | `docs/monitoring/grafana/v2_fee_observability_dashboard.json` |
| Grafana dashboard spec (md)          | `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md` |
| Operator-only `.env` patch template  | `docs/operator/v2g_g_env_patch.example.env` |
| Backend admin endpoint               | `GET /admin/fees/v2/observability` (`src/api/routes.rs::admin_fees_v2_observability`, `src/fees/v2_observability.rs`) |
| Frontend admin section               | `deopt-v2-frontend/src/app/admin/admin-dashboard.tsx::V2FeeObservabilitySection` |
| Per-product alert files (kept)       | `docs/alertmanager/perp_v2_fee_alerts.yml`, `docs/alertmanager/option_v2_fee_alerts.yml` |
| Runbook                              | `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` |
| ALERTING_SPEC                        | `docs/ALERTING_SPEC.md` |

## Phase 1 — Multi-repo audit

Backend (`deopt-v2-backend`):

- `/metrics` (`src/api/routes.rs::metrics`) renders the V2 fee gauges
  via `crate::monitoring::render_metrics` → `append_fee_metrics`. The
  consumer-bucket gauges are pre-seeded at zero, the budget gauge has
  no fake baseline.
- Admin fee endpoints: `summary` / `events` / `onchain` / `volumes` /
  `rebates` (V2D…V2F). No `/admin/fees/v2/*` endpoint existed before
  V2G-G.
- `.env.example` already canonical (V2G-F) — left untouched.
- No Grafana / monitoring directory existed before V2G-G.

Solidity (`deopt-v2-sol`):

- `deployments/base-sepolia.manifest.draft.json` is the canonical
  Base Sepolia engine snapshot (V2F-LM addresses).
- No mainnet manifest yet; `script/`, `deployments/` only carry
  template stubs.
- Solidity repo carries `MONITORING_SPEC.md` (deployment-stage
  observability spec); the V2 fee observability surface lives in the
  backend repo.

Frontend (`deopt-v2-frontend`):

- Admin route `/admin` exists with a single dashboard component
  (`src/app/admin/admin-dashboard.tsx`).
- Existing admin already surfaces V1/V2 fee events under the
  on-chain fees section. No dedicated V2 observability card before
  V2G-G.

## Phase 2 — Alert deployment package

### `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`

Single deployable rule file containing four rule groups:

1. `deopt_perp_v2_fee_alerts` — `PerpFeeChargedFromOldEngine`,
   `PerpFeeRebatedFromOldEngine`, `PerpFeeConsumerUnknown` (V2F-Q
   parity).
2. `deopt_option_v2_fee_alerts` — `OptionFeeChargedFromOldMarginEngine`,
   `OptionFeeRebatedFromOldMarginEngine`, `OptionFeeConsumerUnknown`
   (V2G-F parity).
3. `deopt_fees_manager_v2_budget_alerts` — `FeesManagerV2RebateBudgetLow`
   (V2G-F, pinned on Base Sepolia mUSDC) plus **V2G-G new**
   `FeesManagerV2RebateBudgetStale` (catches a frozen budget gauge
   while V2 rebates are still firing — indexer-stall detector).
4. `deopt_v2_fee_metrics_liveness` — **V2G-G new**
   `DeoptV2FeeMetricsAbsent` (catches a missing metric pipeline; the
   downstream alerts cannot fire while the gauge is absent, so this
   is the first signal of a dead scrape target).

All low-cardinality: only `severity`, `subsystem`, `milestone`, and
`asset_symbol` labels. No addresses, tx hashes, traders, recipients,
or option series ids are ever promoted to a label.

### Validation

`promtool` is not present in the local toolchain (operator note in
the bundle header), so V2G-G falls back to a Python YAML smoke gate:

```
python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" \
  docs/monitoring/prometheus/v2_fee_alerts.bundle.yml
```

Run + result (V2G-G validation step):

```
docs/monitoring/prometheus/v2_fee_alerts.bundle.yml: 9 rules across 4 groups ✅
docs/alertmanager/perp_v2_fee_alerts.yml:          3 rules across 1 groups ✅
docs/alertmanager/option_v2_fee_alerts.yml:        4 rules across 2 groups ✅
```

Per-rule structural validation (alert name, expr present, severity
label, annotations present) also passed.

### Alertmanager routing example

`docs/monitoring/alertmanager/v2_fee_routing.example.yml` ships an
example route tree:

- PERP/OPTION OLD-engine alerts → smart-contract on-call (paging).
- Unknown-consumer alerts → ticket queue (anomaly, not paging).
- Rebate budget alerts → ops chat with `top up budget` runbook link.
- `DeoptV2FeeMetricsAbsent` → backend on-call (not contract on-call,
  because the metric pipeline is a backend concern).
- Inhibit rule: `DeoptV2FeeMetricsAbsent` suppresses
  perp/option/budget alerts while it is firing, so a dead scrape
  target does not cascade into noise.

Severity is overridden per environment at the route layer:

| Environment   | charged/rebated from OLD | unknown consumer | budget low |
| ------------- | ------------------------ | ---------------- | ---------- |
| Base Sepolia  | `high`                   | `medium`         | `medium`   |
| Mainnet       | `critical`               | `high`           | `high`     |

## Phase 3 — Grafana dashboard

### Files

- `docs/monitoring/grafana/v2_fee_observability_dashboard.json` — Grafana
  9+ import file. Uses the standard `${DS_PROMETHEUS}` datasource
  variable. Tag set: `deopt`, `v2-fees`, `feesmanagerv2`, `observability`,
  `V2G-G`. UID `deopt-v2g-g-v2-fees`.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md` —
  panel-by-panel spec; useful when porting to DataDog / CloudWatch /
  custom HTML.

### Panels

| # | Title                                        | Type       | Metric expression |
|---|----------------------------------------------|------------|-------------------|
| 1 | PERP FeeChargedV2 by consumer                | timeseries | `deopt_perp_fee_charged_v2_total{consumer=~"$consumer"}` |
| 2 | PERP FeeRebatedV2 by consumer                | timeseries | `deopt_perp_fee_rebated_v2_total{consumer=~"$consumer"}` |
| 3 | OPTION FeeChargedV2 by consumer              | timeseries | `deopt_option_fee_charged_v2_total{consumer=~"$consumer"}` |
| 4 | OPTION FeeRebatedV2 by consumer              | timeseries | `deopt_option_fee_rebated_v2_total{consumer=~"$consumer"}` |
| 5 | Rebate budget by settlement asset            | timeseries | `deopt_fees_manager_v2_rebate_budget_native` |
| 6 | Base Sepolia mUSDC rebate budget (latest)    | stat       | `deopt_fees_manager_v2_rebate_budget_native{asset="0x6eae407f5640b006fac9965182e238582a3b412e"}` |
| 7 | OLD consumer events (PERP)                   | stat       | `deopt_perp_fee_charged_v2_total{consumer="old"} + deopt_perp_fee_rebated_v2_total{consumer="old"}` |
| 8 | OLD consumer events (OPTION)                 | stat       | `deopt_option_fee_charged_v2_total{consumer="old"} + deopt_option_fee_rebated_v2_total{consumer="old"}` |
| 9 | Unknown consumer events (PERP + OPTION)      | stat       | sum of the four `…{consumer="unknown"}` series |
| 10| Active engine addresses (reference)          | table      | static reference card; see `/admin/fees/v2/observability` for live values |

Color thresholds:

- PERP/OPTION time series: classic palette, `consumer="old"` red,
  `consumer="unknown"` orange.
- Budget panels: red `< 1 000`, orange `< 100 000`, green otherwise.
- Anomaly stats: green when zero; red on OLD events; orange on
  unknown events.

## Phase 4 — Frontend admin section

`deopt-v2-frontend/src/app/admin/admin-dashboard.tsx`:

- New `V2FeeObservabilitySection` (and `V2ObservabilityEndpointStatus`,
  `V2FeeObservabilityView`, `BucketCountsCard`, `RebateBudgetTable`).
- Reads `/admin/fees/v2/observability` via a new
  `fetchAdminFeesV2Observability` helper in `src/lib/admin-api.ts`.
- Auto-loads once on token-ready; manual refresh button surfaces the
  HTTP status + last-fetched timestamp.
- Tile groups:
  - **Anomaly totals.** OLD-event count (red when > 0) and
    unknown-event count (orange when > 0), plus network + milestone.
  - **PERP charged / rebated by consumer.** Three-bucket cards per
    metric; `old` red when > 0, `unknown` orange when > 0, `new`
    green.
  - **OPTION charged / rebated by consumer.** Same shape.
  - **Rebate budget per asset.** Table of `{asset, native units}` for
    every settlement asset that has emitted a `RebateBudget*` event.
  - **Active engine wiring.** Five reference tiles (NEW PERP, OLD
    PERP, NEW MARGIN, OLD MARGIN, FeesManagerV2). NEW tiles warn when
    unset / zero address (visible env-hygiene drift).
  - **Runtime feature flags.** metrics_enabled / option_event_indexer
    enabled / fees_enabled / rebates_enabled / persistence_enabled.
- Quick-fill buttons populate the existing "On-chain Fee Events"
  tx-hash input with the V2G-E PERP / OPTION rebate transaction hashes.

Frontend types added to `src/types/admin.ts`:
`AdminFeeV2ObservabilityBuckets`, `AdminFeeV2ObservabilitySuccess`,
`AdminFeeV2ObservabilityFailure`, `AdminFeeV2ObservabilityResult`.

No wallet actions, no write endpoints, no signer surface. Strictly
read-only.

## Phase 5 — Backend admin observability endpoint

`GET /admin/fees/v2/observability` (admin-token-gated). Returns:

```jsonc
{
  "milestone": "V2G-G",
  "network": { "chain_id": 84532, "network_name": "base-sepolia" },
  "features": {
    "metrics_enabled": true,
    "option_event_indexer_enabled": true,
    "fees_enabled": false,
    "rebates_enabled": false,
    "persistence_enabled": true
  },
  "contracts": {
    "perp_engine_new":    "0xc6C5...141c",
    "perp_engine_old":    "0xB363...b53B",
    "margin_engine_new":  "0x287C...48Cc",
    "margin_engine_old":  "0x6C56...b5F8",
    "fees_manager_v2":    "0x00dA...774f"
  },
  "metrics": {
    "perp_fee_charged_v2_by_consumer":    {"new": 3, "old": 0, "unknown": 0},
    "perp_fee_rebated_v2_by_consumer":    {"new": 1, "old": 0, "unknown": 0},
    "option_fee_charged_v2_by_consumer":  {"new": 3, "old": 0, "unknown": 0},
    "option_fee_rebated_v2_by_consumer":  {"new": 1, "old": 0, "unknown": 0},
    "fees_manager_v2_rebate_budget_native": {
      "0x6eae407f5640b006fac9965182e238582a3b412e": 999987
    }
  },
  "anomaly_totals": {
    "old_consumer_events":     0,
    "unknown_consumer_events": 0
  },
  "notes": [
    "Raw addresses are never promoted to bucket labels (consumer in {new,old,unknown}).",
    "rebate_budget_native is derived from indexed RebateBudgetFunded/Spent/Withdrawn events, clamped at zero.",
    "Read-only snapshot. See docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md."
  ]
}
```

Implementation (`src/fees/v2_observability.rs::admin_v2_observability`):

- Loads raw consumer counts via the repository (when available) or the
  in-memory `OptionSeriesStore` (V2F-Q / V2G-F mirror helpers — no
  duplicated SQL).
- Bucket classification reuses
  `crate::fees::perp_consumer::classify_perp_fee_consumer` and
  `crate::fees::option_consumer::classify_option_fee_consumer` verbatim
  so the JSON view of the metric pipeline is bit-equivalent to the
  Prometheus rendering.
- Address-omission contract: any zero / empty engine address is
  emitted as `null` (the classifier ignores zero-address inputs in
  V2F-P / V2G-F invariant tests; the endpoint signals the same to
  operators).

### Tests

Four new HTTP integration tests in `src/api/routes.rs::tests`:

| Test                                                                | Asserts |
|---------------------------------------------------------------------|---------|
| `admin_v2_observability_snapshot_emits_three_buckets_at_zero`       | Empty backend renders all four consumer-bucket metrics with `new=0,old=0,unknown=0`. No per-asset budget series. `anomaly_totals` are zero. `milestone == V2G-G`. |
| `admin_v2_observability_requires_admin_token_when_configured`       | Missing or empty token → 403. Correct token → 200. |
| `admin_v2_observability_classifies_new_old_and_unknown_buckets`     | Inserts PERP NEW + PERP OLD + OPTION NEW + OPTION NEW rebate + OPTION unknown + budget Funded/Spent. Asserts NEW PERP=1, OLD PERP=1, NEW OPTION=1, OPTION rebate NEW=1, OPTION unknown=1, mUSDC budget = 999_987, anomaly totals {old:1, unknown:1}. Cardinality contract: serialized metrics block contains NONE of the trader / stray-consumer raw strings. |
| `admin_v2_observability_omits_zero_address_contracts`               | Zero / unset PERP/MARGIN/FeesManagerV2 addresses surface as `null` in the JSON, never as the literal `0x000…000`. |

Test suite: **675 → 679 passed (+4 V2G-G tests), 0 failed, 0 ignored.**

## Phase 6 — Operator `.env` patch (documentation only)

`docs/operator/v2g_g_env_patch.example.env` lays out the exact
key/value pairs the operator should paste into the gitignored local
`.env`. The agent did **not** apply this patch — the V2G hard-rules
forbid touching the real `.env`. The patch ships:

- The five canonical Base Sepolia addresses (`PERP_ENGINE_ADDRESS`,
  `OLD_PERP_ENGINE_ADDRESS`, `MARGIN_ENGINE`, `OLD_MARGIN_ENGINE_ADDRESS`,
  `FEES_MANAGER_V2`) with inline comments pointing at the milestone
  records.
- Three verification command blocks operators can run after applying:
  1. Grep the running shell env for only the V2 fee classifier vars +
     non-secret toggles (no `*_PRIVATE_KEY` anywhere).
  2. `curl` `/admin/config` and `/admin/fees/v2/observability` with
     the admin token, plus a `jq` filter that prints the contracts +
     anomaly totals + consumer buckets + budget map.
  3. `curl` `/metrics` and grep the four V2 gauges; cross-check the
     `consumer="old"` / `consumer="unknown"` arms read `0` and the
     budget gauge matches `FeesManagerV2.rebateBudget(mUSDC)` from
     `cast call`.
- Explicit "hard refusals" list at the bottom: do not point active
  PERP at OLD; do not commit resolved values; do not mutate the
  Merkle root / budget / consumer wiring from this file.

## Phase 7 — Mainnet / multi-asset readiness matrix

The V2 fee metric pipeline is asset-and-network-agnostic by
construction. Promoting it to mainnet (Base mainnet first, then any
EVM target) requires updating the network-specific knobs only:

| Knob                                  | Base Sepolia (today)          | Mainnet (USDC / WETH)                              | Where to change                                                                                  |
| ------------------------------------- | ------------------------------ | -------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `FEES_MANAGER_V2`                     | `0x00dA…774f`                  | new mainnet address                                | `.env` + `docs/operator/v2g_g_env_patch.example.env`                                             |
| `PERP_ENGINE_ADDRESS`                 | `0xc6C5…141c`                  | new mainnet NEW PerpEngine                          | `.env` (NEVER OLD)                                                                                |
| `OLD_PERP_ENGINE_ADDRESS`             | `0xB363…b53B`                  | mainnet legacy PerpEngine if any (else unset)       | `.env`                                                                                            |
| `MARGIN_ENGINE`                       | `0x287C…48Cc`                  | new mainnet NEW MarginEngine                        | `.env`                                                                                            |
| `OLD_MARGIN_ENGINE_ADDRESS`           | `0x6C56…b5F8`                  | mainnet legacy MarginEngine if any (else unset)     | `.env`                                                                                            |
| Settlement asset (rebate budget gauge label) | `0x6eae407f…412e` (mUSDC)      | `0x833589fc…2913` (USDC); add `0x4200…0006` (WETH) | Per-asset rule in `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` + Grafana panel #6 clone  |
| `FeesManagerV2RebateBudgetLow` threshold | `< 1000` (= 0.001 mUSDC, 6 dp) | `< 100000` (= 0.1 USDC, 6 dp); `< 1e15` (= 0.001 WETH, 18 dp) | per-asset rule (one rule per `asset` label)                                                  |
| Alert severity                        | high / medium                  | critical / high                                    | Alertmanager route (`docs/monitoring/alertmanager/v2_fee_routing.example.yml`)                   |
| `OPTION_EVENT_INDEXER_RPC_URL` / `EXECUTOR_RPC_URL` | Base Sepolia RPC | Mainnet RPC                                       | `.env` (no code change)                                                                           |
| Oracle adapter addresses              | `0x3eb9cdd2…6Cc` (ETH primary) | mainnet oracle adapters                            | scripts under `deopt-v2-sol/script/` + manifest                                                  |
| `chain_id`                            | `84532`                         | `8453` (Base mainnet)                              | `EXECUTOR_CHAIN_ID` / EIP-712 domain configs                                                     |
| Grafana panel #6 (asset card)         | mUSDC stat panel               | Clone per supported asset                          | Grafana JSON                                                                                      |

Multi-asset rebate budget alert pattern (per asset, one rule):

```yaml
- alert: FeesManagerV2RebateBudgetLow_<symbol>
  expr: |
    deopt_fees_manager_v2_rebate_budget_native{
      asset="<lowercased mainnet address>"
    } < <threshold-native>
  for: 0m
  labels:
    severity: critical          # mainnet override
    subsystem: fees_manager_v2
    milestone: V2G-G
    asset_symbol: <symbol>
    chain: <chain-label>
  annotations:
    summary: "FeesManagerV2 <symbol> rebate budget low (<chain>)"
    description: "Top up via FeesManagerV2.fundRebateBudget(<symbol>, amount)."
    runbook_url: "docs/RUNBOOK_PERP_V2_FEE_ALERTS.md#feesmanagerv2rebatebudgetlow"
```

Multi-asset gauge series: the metric pipeline already emits one
`asset=...` series per indexed settlement asset
(`fees_manager_v2_rebate_budget_metric_reflects_funded_minus_spent_and_withdrawn`
in `src/api/routes.rs::tests` confirms multi-asset behaviour). No
backend code change is required to support a new asset.

Out of scope for V2G-G (correctly):

- Mainnet contract deployment, manifest write-back, governance/timelock
  rewiring. Those are downstream deploy milestones.
- Frontend's "active engine wiring" cards already pick up mainnet
  addresses via the same admin endpoint — no code change there
  either.

## Phase 8 — Read-only live verification

Backend rebuilt (`cargo build --release` + touch-rebuild for V2G-G
sources) and started in **read-only mode** with the V2D-S + V2E-F +
V2F-O env stack plus shell-only V2G-G overrides:

```
PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
OPTION_EXECUTION_BROADCAST_ENABLED=false
# unset: EXECUTOR_PRIVATE_KEY, BUYER_PRIVATE_KEY, SELLER_PRIVATE_KEY,
#        DEPLOYER_PRIVATE_KEY, SIGNER_PRIVATE_KEY,
#        PERP_SMOKE_*_PRIVATE_KEY, OPTION_SMOKE_*_PRIVATE_KEY
```

### `/health`

```
{"ok":true,"service":"deopt-v2-backend"}
```

### `/admin/fees/v2/observability` (new V2G-G endpoint)

Reproduces V2G-F closure exactly:

```
contracts.perp_engine_new   = 0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
contracts.perp_engine_old   = 0xB36395b67D0798ADA981731c9Fa5239F4362b53B
contracts.margin_engine_new = 0x287Cef479be5889eEfCa847F9e73C860898f48Cc
contracts.margin_engine_old = 0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
contracts.fees_manager_v2   = 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
metrics.perp_fee_charged_v2_by_consumer   = {"new":3,"old":0,"unknown":0}
metrics.perp_fee_rebated_v2_by_consumer   = {"new":1,"old":0,"unknown":0}
metrics.option_fee_charged_v2_by_consumer = {"new":3,"old":0,"unknown":0}
metrics.option_fee_rebated_v2_by_consumer = {"new":1,"old":0,"unknown":0}
metrics.fees_manager_v2_rebate_budget_native = {"0x6eae407f…412e": 999987}
anomaly_totals.old_consumer_events     = 0
anomaly_totals.unknown_consumer_events = 0
```

### `/admin/fees/onchain?tx_hash=…` (PERP V2G-E)

```
tx_hash:                0x5c15e9233d49729cf21058a89f49bc6fdf0f7295cda5a7f313c96556728aa394
event_model:            v2
fee_charged_v2_count:   1
fee_rebated_v2_count:   1
observed_total_charged: 6
observed_total_rebated: 3
net_protocol_fee:       3
reconciliation_status:  onchain_observed
source_of_truth:        onchain
events:
  - FeeChargedV2  perp orderbook fee_amount=6
  - FeeRebatedV2  perp orderbook rebate_amount=3
```

### `/admin/fees/onchain?tx_hash=…` (OPTION V2G-E)

```
tx_hash:                0x9a85cbced2216bf3c18049111cce68883cb0b035e194b3dcbaaf4fe7d5293149
event_model:            mixed
fee_charged_v2_count:   1
fee_rebated_v2_count:   1
observed_total_charged: 25
observed_total_rebated: 10
net_protocol_fee:       15
trading_fee_event_count: 1   (V1-compat for the taker leg)
reconciliation_status:  onchain_observed
source_of_truth:        onchain
events:
  - TradingFeeCharged                        fee_amount=25
  - FeeChargedV2  option orderbook            fee_amount=25
  - FeeRebatedV2  option orderbook            rebate_amount=10
```

### `/metrics` scrape

```
deopt_perp_fee_charged_v2_total{consumer="new"} 3
deopt_perp_fee_charged_v2_total{consumer="old"} 0
deopt_perp_fee_charged_v2_total{consumer="unknown"} 0
deopt_perp_fee_rebated_v2_total{consumer="new"} 1
deopt_perp_fee_rebated_v2_total{consumer="old"} 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} 0
deopt_option_fee_charged_v2_total{consumer="new"} 3
deopt_option_fee_charged_v2_total{consumer="old"} 0
deopt_option_fee_charged_v2_total{consumer="unknown"} 0
deopt_option_fee_rebated_v2_total{consumer="new"} 1
deopt_option_fee_rebated_v2_total{consumer="old"} 0
deopt_option_fee_rebated_v2_total{consumer="unknown"} 0
deopt_fees_manager_v2_rebate_budget_native{
  asset="0x6eae407f5640b006fac9965182e238582a3b412e"
} 999987
```

### Alert expression sanity check

Every V2G-G alert evaluated against the current `/metrics` shape
(cumulative gauges; a nonzero `{old}` arm since boot implies a recent
event):

```
PerpFeeChargedFromOldEngine             would_fire=False  (0)
PerpFeeRebatedFromOldEngine             would_fire=False  (0)
PerpFeeConsumerUnknown                  would_fire=False  (0)
OptionFeeChargedFromOldMarginEngine     would_fire=False  (0)
OptionFeeRebatedFromOldMarginEngine     would_fire=False  (0)
OptionFeeConsumerUnknown                would_fire=False  (0)
FeesManagerV2RebateBudgetLow (mUSDC<1000)  would_fire=False  (999987)
DeoptV2FeeMetricsAbsent                 would_fire=False
```

All eight V2 alerts are **green** under the V2G-E post-state.

### Backend stop

`pkill -f 'target/release/deopt-v2-backend'`. Process tree drained.
No DB rows mutated; the V2F-N event indexer was not invoked
(`OPTION_EVENT_INDEXER_POLL_INTERVAL_MS=86400000` in the
observability env, and no `/admin/options/events/tick` was hit in
this V2G-G run because V2G-F already caught the indexer up past
block `42206003`).

## Phase 9 — Docs

Created:

- `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` (this file).
- `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml`.
- `docs/monitoring/alertmanager/v2_fee_routing.example.yml`.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.json`.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md`.
- `docs/operator/v2g_g_env_patch.example.env`.

Updated:

- `docs/ALERTING_SPEC.md` — V2G-G section (new bundle path, two new
  alerts, multi-asset extension pattern).
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-G entries for
  `DeoptV2FeeMetricsAbsent`, `FeesManagerV2RebateBudgetStale`, and a
  new "Quick admin probe" section that points operators at the new
  `/admin/fees/v2/observability` endpoint.
- `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` — V2G-G
  closure note (appended to the existing V2G-F closure note).
- `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` — V2G-G
  closure note (appended).

## Phase 10 — Validation

Backend touched (this milestone):

| Command | Result |
|---|---|
| `cargo fmt --all`                                            | ✅ clean (rustfmt re-wrapped some long lines on first run; second run shows no diff) |
| `cargo clippy --all-targets --all-features -- -D warnings`   | ✅ clean |
| `cargo build --all-targets --all-features`                   | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast`     | ✅ **679 passed, 0 failed, 0 ignored** (was 675 in V2G-F; +4 V2G-G tests) |

Frontend touched:

| Command           | Result |
|---|---|
| `npx tsc --noEmit` | ✅ clean |
| `npm run lint`     | ✅ clean |
| `npm run build`    | ✅ Next.js production build clean (3 pages, 0 type errors, 0 lint warnings) |

Solidity:

- Untouched in V2G-G scope (no contract changes). The Sol repo
  carries `MONITORING_SPEC.md` and `deployments/base-sepolia.manifest.draft.json`
  as background; cross-referenced from this doc, but no V2G-G writes.
- `forge fmt --check` / `forge build` / `forge test --no-match-path 'test/fork/*'`
  not re-run here because no Sol source changed.

## Files changed

Backend:

- `src/fees/mod.rs` — added `pub mod v2_observability;`.
- `src/fees/v2_observability.rs` — **NEW** (snapshot builder).
- `src/api/routes.rs` — wired `/admin/fees/v2/observability` handler +
  4 new HTTP tests.
- `docs/V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md` — **NEW** (this file).
- `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` — **NEW**.
- `docs/monitoring/alertmanager/v2_fee_routing.example.yml` — **NEW**.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.json` — **NEW**.
- `docs/monitoring/grafana/v2_fee_observability_dashboard.spec.md` — **NEW**.
- `docs/operator/v2g_g_env_patch.example.env` — **NEW**.
- `docs/ALERTING_SPEC.md` — V2G-G additions.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — V2G-G additions.
- `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` — V2G-G note.
- `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` — V2G-G note.

Frontend:

- `src/types/admin.ts` — new V2 observability result types.
- `src/lib/admin-api.ts` — `fetchAdminFeesV2Observability` helper.
- `src/app/admin/admin-dashboard.tsx` — new V2 observability section +
  rebate budget table + bucket cards + auto-load on token-ready.

## Remaining blockers

1. **Real `.env` patch still belongs to the operator.** V2G-G ships
   the diff at `docs/operator/v2g_g_env_patch.example.env`; the agent
   cannot apply it under the V2G hard-rules. The shell-only override
   pattern remains a valid fallback (used in this milestone's
   verification).
2. **promtool not in the local toolchain.** V2G-G falls back to a
   Python YAML smoke + per-rule structural validation. Re-running
   `promtool check rules` in CI is recommended once the operator
   wires Prometheus into the deploy.
3. **Mainnet manifest absent.** The Sol repo carries only Base Sepolia
   manifests today; per-asset multi-asset budget rule rollout depends
   on a mainnet manifest landing first (see §"Mainnet / multi-asset
   readiness matrix").
4. **`PerpRebateStalled` cadence alert deferred.** Commented out in the
   Prometheus bundle (no daily cadence established yet on Base Sepolia
   — V2G-E is the only PERP rebate event live). Enable after the
   V2G band ships ongoing flow.

## V2G-H closure (appended 2026-05-31)

V2G-H validated the V2G-G artefacts with the real Prometheus +
Alertmanager toolchains and prepared the operator-facing integration
package. See
`docs/V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md` for the full
record.

Key V2G-H outcomes:

- Stack discovery: local host has no Prometheus / Alertmanager /
  Grafana installed. V2G-H is therefore a "prepare + validate + plan"
  milestone; live wire-up is V2G-I.
- `promtool check rules` ✅ for the bundle and both legacy per-product
  files. New `docs/monitoring/prometheus/v2_fee_alerts.test.yml`
  exercises 5 scenarios (green / PERP OLD / OPTION unknown / budget
  low / metrics absent); `promtool test rules` ✅ on all of them.
- `amtool check-config` ✅ on the Alertmanager routing example.
  `amtool config routes test` resolves 4 of 5 sample alerts to the
  correct receiver (one expected `continue: true` double-resolution
  documented).
- Grafana provisioning entry + datasource template +
  `render_dashboard.sh` substitution helper added under
  `docs/monitoring/grafana/provisioning/`.
- Backend rebuilt + re-run in read-only mode; `/health`,
  `/admin/fees/v2/observability`, `/admin/fees/onchain` for both
  V2G-E txs, and `/metrics` all reproduced the V2G-F / V2G-G closure
  state byte-for-byte. All 8 V2 alerts logically green against the
  live scrape.
- Operator `.env` patch documented but unapplied (real `.env` still
  carries the V2F-O `PERP_ENGINE_ADDRESS=OLD` line); exact diff +
  rollback recorded.

## Next recommended milestone

**V2G-H — flip the V2 fee observability surface to PRODUCTION-FIRING
in the operator's actual Alertmanager + Grafana stack.**

- Drop `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` into the
  Prometheus rules directory, reload Prometheus, confirm the seven
  rules show up in `/alerts` with `state=inactive` (Base Sepolia
  post-V2G-E).
- Import `docs/monitoring/grafana/v2_fee_observability_dashboard.json`
  into Grafana, attach the Prometheus datasource, pin to the team
  folder.
- Wire the example Alertmanager route fragment into the deployed
  routing tree.
- Validate end-to-end by toggling `OLD_MARGIN_ENGINE_ADDRESS` to a
  bogus address in a staging shell; confirm the OPTION OLD alerts
  fire as expected and clear within 5 minutes after revert.
- Apply the operator `.env` patch documented in
  `docs/operator/v2g_g_env_patch.example.env` to the local backend.
- Optionally: cut the next V2G alert family scope — `OracleStale`,
  `LiquidationStarted`, `BadDebtIncurred` — which V1B deferred.
