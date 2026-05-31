# V2G-F — Fee Observability, Env Hygiene, and Executor Readiness Closure

## Status

- Milestone: **V2G-F** — closes the observability / env-hygiene /
  executor-readiness gaps carried forward from V2G-E.
- Date: 2026-05-31.
- Outcome:
  - `.env.example` flipped to canonical NEW addresses + explicit OLD
    stranded metadata for both `PerpEngine` and `MarginEngine` (committed).
  - OPTION V2 metrics (`deopt_option_fee_charged_v2_total` +
    `deopt_option_fee_rebated_v2_total`) now emitted with the same
    three-bucket (`new`/`old`/`unknown`) cardinality contract as PERP.
  - Derived rebate budget gauge
    `deopt_fees_manager_v2_rebate_budget_native{asset=...}` added with
    a corresponding low-budget alert.
  - Alert rule file `docs/alertmanager/option_v2_fee_alerts.yml` added.
  - PERP runbook extended with OPTION + budget alert procedures
    (filename retained for backwards-link stability).
  - V2G-A merkle-root-unset operational notice **retired**.
  - Backend executor signing readiness audit recorded — no code change
    needed; documented operator pattern.
  - Live read-only verification against the V2G-E PERP + OPTION txs
    reproduced cleanly with all metric buckets routed correctly.
- Hard gates respected: no broadcasts, no on-chain mutation, no DB
  rows deleted, no real-secret `.env` edited, no Merkle root /
  rebate-budget mutation, no `OLD_PERP_ENGINE` traffic.

## Phase 1 — Env / config audit

`PERP_ENGINE_ADDRESS` / `OLD_PERP_ENGINE_ADDRESS`:

- Read in `src/config/env.rs:101–108` and stored on
  `ExecutionConfig` (`src/execution/config.rs:44–50`).
- Consumed by `src/monitoring.rs::append_perp_fee_v2_consumer_metric`
  (V2F-P/V2F-Q).
- Committed `.env.example` already (correctly) points at the NEW
  PerpEngine + records OLD as stranded observability metadata. The
  V2F-O carry-over was **not** in the example — it was in the
  operator's local-only real `.env` (gitignored), which V2G-F leaves
  untouched per the hard-rule "do not edit real `.env`".

`MARGIN_ENGINE` / `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS`:

- Read in `src/config/env.rs:368–372` and stored on
  `OptionEventIndexerConfig.margin_engine_address`
  (`src/options/event_indexer.rs:64`).
- Used by the OPTION event indexer (subscribes V2 fee topics on
  this address via the FeesManagerV2 contract).
- V2G-F adds an optional sibling `OLD_MARGIN_ENGINE_ADDRESS` env var
  (`src/config/env.rs::option_event_indexer_old_margin_engine_address`)
  stored on the same struct
  (`OptionEventIndexerConfig.old_margin_engine_address`). Used solely
  by the OPTION metric classifier; never wired into broadcast or
  execution traffic.

`FEES_MANAGER_V2` / `OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS`:

- Read in `src/config/env.rs:382–385`. Stored on
  `OptionEventIndexerConfig.fees_manager_v2_address`. Subscribed by
  the indexer for the V2 fee + rebate + budget events.

Backend executor / signing-key env (`EXECUTOR_PRIVATE_KEY`,
`BUYER_PRIVATE_KEY`, `SELLER_PRIVATE_KEY`, `SIGNER_PRIVATE_KEY`):

- `EXECUTOR_PRIVATE_KEY` — only required when
  `EXECUTOR_REAL_BROADCAST_ENABLED=true` /
  `OPTION_EXECUTION_BROADCAST_ENABLED=true`. Loaded from env via
  `src/execution/signer.rs`. Never written to disk; never echoed by
  any redacted-Debug path.
- `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` — read **only** by the
  standalone CLIs `src/bin/sign_perp_trade.rs` and
  `src/bin/sign_option_execution_intent.rs`. Neither name is read
  by the backend HTTP server itself; the server's runtime intent
  signing path uses `EXECUTOR_PRIVATE_KEY`, not BUYER/SELLER. The
  conflation in V2D-V / V2E-G was that the **orderbook+RFQ test
  fixtures** in `scripts/e2e/run_e2e.py` use the `.env` BUYER/SELLER
  ADDRESSES to construct intents — but at smoke time the CLI accepts
  a fully shell-supplied key.
- `SIGNER_PRIVATE_KEY` — fallback for `sign_perp_trade` when neither
  BUYER nor SELLER is set.

## Phase 2 — Env template cleanup

Updated `.env.example` only. **Real `.env` left untouched.** Diff:

```diff
@@ committed .env.example @@
 # V2F-P observability metadata.
 # OLD_PERP_ENGINE_ADDRESS is used SOLELY by the
 # `deopt_perp_fee_charged_v2_total{consumer="old"}` metric and the
 # `PerpFeeChargedFromOldEngine` Prometheus alert ...
 OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
+
+# Base Sepolia V2G state — MarginEngine (OPTION).
+# - NEW (active, V2-fees enabled per V2E-E) MarginEngine =
+#   0x287Cef479be5889eEfCa847F9e73C860898f48Cc
+# - OLD MarginEngine (legacy V2D-R; superseded — never use as active) =
+#   0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
+MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
+
+# V2G-F observability metadata.
+# OLD_MARGIN_ENGINE_ADDRESS is used SOLELY by the
+# `deopt_option_fee_charged_v2_total{consumer="old"}` metric ...
+OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
+
+# Base Sepolia V2G state — FeesManagerV2 (live since V2E-E).
+FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
```

### Operator patch for local (gitignored) `.env`

If the operator wants the metric classifier to route correctly
without the shell-only override the agent used in V2G-E, apply this
exact diff to the local `.env`:

```diff
-PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
+PERP_ENGINE_ADDRESS=0xc6C592100723Fe0C66343A16e95eC34cC0c2141c
+OLD_PERP_ENGINE_ADDRESS=0xB36395b67D0798ADA981731c9Fa5239F4362b53B
+MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
+OLD_MARGIN_ENGINE_ADDRESS=0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8
+FEES_MANAGER_V2=0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f
```

This is a non-destructive patch (existing other env vars unchanged).
The agent **did not apply this patch** because the hard rules forbid
editing the real `.env`. Operator: apply it whenever convenient — the
shell-only override path remains the documented fallback.

## Phase 3 — OPTION V2 metrics

### New module

`src/fees/option_consumer.rs` — V2G-F classifier mirror of the V2F-P
`perp_consumer`. Re-exports the same three-bucket vocabulary
(`CONSUMER_NEW` / `CONSUMER_OLD` / `CONSUMER_UNKNOWN`) and delegates
to `classify_perp_fee_consumer` for the actual address match — so
case-insensitivity, zero-address handling, and the raw-address
suppression invariant are shared 1:1 with the PERP path.

### Backend metrics wired

`src/monitoring.rs::append_fee_metrics`:

- Calls `admin_option_fee_v2_consumer_counts` (charged) and
  `admin_option_fee_v2_rebated_consumer_counts` (rebated) on the
  repository, plus their `OptionSeriesStore` in-memory mirrors.
- Calls the new helper
  `append_option_fee_v2_consumer_metric(state, metrics, name, help,
  raw_counts)` which:
  - reads NEW MarginEngine from
    `option_event_indexer_config.margin_engine_address`;
  - reads optional OLD MarginEngine from
    `option_event_indexer_config.old_margin_engine_address`;
  - pre-seeds the three bucket labels at zero so the
    `increase(...)[5m]` alert query has a stable time series from
    the first scrape.

Emitted metrics:

```
deopt_option_fee_charged_v2_total{consumer="new"|"old"|"unknown"}
deopt_option_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}
deopt_fees_manager_v2_rebate_budget_native{asset="<lowercased address>"}
```

Repository helpers (`src/db/repository.rs`):

- `admin_option_fee_v2_consumer_counts()` — SQL group-by
  `decoded.consumer`, filtered to
  `event_name='FeeChargedV2' AND productKind='option'`.
- `admin_option_fee_v2_rebated_consumer_counts()` — same but for
  `FeeRebatedV2`.
- `admin_fees_manager_v2_rebate_budget_by_asset()` — derived budget
  per settlement asset, computed as
  `SUM(Funded.amount) − SUM(Spent.amount) − SUM(Withdrawn.amount)`
  grouped by lowercased `decoded.settlementAsset`. NUMERIC summed
  in SQL and cast to TEXT; negative net clamps to zero.
- Internal helper `admin_fee_v2_consumer_counts_for_event_and_product`
  generalises the V2F-P PERP helper to take a `product_kind`
  parameter, deduplicating SQL between PERP and OPTION.

In-memory `OptionSeriesStore`:

- `option_fee_v2_consumer_counts` / `option_fee_v2_rebated_consumer_counts`
  — mirrors of the repository for non-persistence test paths.
- `fees_manager_v2_rebate_budget_by_asset` — same derivation, same
  clamping rule.

Cardinality contract:

- `consumer` label is exactly one of `new`/`old`/`unknown`. No raw
  address ever reaches the label.
- `asset` label is the lowercased settlement-asset address
  (currently only `0x6eae407f5640b006fac9965182e238582a3b412e` on
  Base Sepolia — pinned in the alert rule).

### Tests added

`src/api/routes.rs::tests`:

| Test | Asserts |
|---|---|
| `option_fee_charged_v2_metric_emits_three_buckets_at_zero` | All three buckets pre-seeded at 0 on empty backend; same for rebated sibling. |
| `option_fee_charged_v2_metric_classifies_new_and_excludes_perp_and_rebate` | NEW OPTION FeeChargedV2 → `new=1`. PERP-flavoured charged + OPTION rebate do **not** leak into the OPTION charged counter. No raw addresses in metric body. |
| `option_fee_charged_v2_metric_classifies_old_consumer` | OLD-emitted OPTION FeeChargedV2 → `old=1` when `OLD_MARGIN_ENGINE_ADDRESS` is configured. |
| `option_fee_charged_v2_metric_classifies_unknown_consumer` | Stray consumer + OLD unset → `unknown=1`. |
| `option_fee_rebated_v2_metric_classifies_new_and_excludes_perp` | NEW OPTION FeeRebatedV2 → `new=1`; PERP rebate stays in PERP counter. |
| `option_fee_rebated_v2_metric_classifies_unknown_consumer` | Stray consumer → `unknown=1`. |
| `fees_manager_v2_rebate_budget_metric_reflects_funded_minus_spent_and_withdrawn` | Multi-asset derivation correct; empty backend emits no per-asset series (no fake baseline). |

`src/fees/option_consumer.rs::tests`:

| Test | Asserts |
|---|---|
| `matches_new_margin_engine` | Case-insensitive NEW match. |
| `matches_old_margin_engine_when_configured` | OLD match when configured. |
| `unknown_when_old_unset_and_consumer_is_not_new` | OLD unset → unknown for any non-NEW consumer. |
| `unknown_when_no_addresses_configured` | Both unset → unknown. |
| `zero_address_never_matches` | Zero address as either consumer or configured target → unknown. |
| `empty_consumer_resolves_to_unknown` | Empty string consumer → unknown. |
| `classifier_never_emits_raw_address` | Every fuzz case stays inside the three-bucket vocabulary. |

Total new tests: **14** (7 metric tests + 7 classifier tests).
Test suite size: **661 → 675 passed, 0 failed**.

## Phase 4 — Alerts

New alert rule file: `docs/alertmanager/option_v2_fee_alerts.yml`
containing:

- `OptionFeeChargedFromOldMarginEngine` (high) — mirror of
  `PerpFeeChargedFromOldEngine`.
- `OptionFeeRebatedFromOldMarginEngine` (high) — mirror of
  `PerpFeeRebatedFromOldEngine`.
- `OptionFeeConsumerUnknown` (medium) — mirror of
  `PerpFeeConsumerUnknown`.
- `FeesManagerV2RebateBudgetLow` (medium) — pinned on the canonical
  Base Sepolia mUSDC address; operator overrides per network.

PERP alert file (`docs/alertmanager/perp_v2_fee_alerts.yml`) left
unchanged (V2F-Q already covered both charged + rebated PERP).

`docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` extended with sections for the
three OPTION alerts plus `FeesManagerV2RebateBudgetLow`. Title
preserved as "PERP V2 Fee Alerts" for stable inbound link IDs; the
introduction now says it covers both products.

`docs/ALERTING_SPEC.md` extended with V2G-F entries:

- OPTION FeeChargedV2 / FeeRebatedV2 From OLD MarginEngine (V2G-F).
- OPTION V2 Fee Event From Unknown Consumer (V2G-F).
- FeesManagerV2 Rebate Budget Low (V2G-F).
- New "Retired / Downgraded Operational Notices" section covering
  the V2G-A merkle-root-unset notice (retired now that the root
  has been live since V2G-C / rotated under V2G-D2 / claimed in
  V2G-D3 / used live in V2G-E).

## Phase 5 — Backend executor readiness

The backend supports two signing surfaces today, both of which can
sign V2G-D2 EOA trades **without any committed secret or `.env`
edit**:

1. **In-process executor** (used by the orderbook / RFQ flow):
   reads `EXECUTOR_PRIVATE_KEY` from the process env at startup.
   Operator workflow:

   ```sh
   # Operator-only — keys never echoed, never written to disk, never
   # sent to chat. The shell-only export pattern leaves the gitignored
   # .env file unchanged.
   export EXECUTOR_PRIVATE_KEY=<operator supplies>
   export EXECUTOR_REAL_BROADCAST_ENABLED=true
   export EXECUTOR_DRY_RUN=false
   export OPTION_EXECUTION_BROADCAST_ENABLED=true
   ./target/release/deopt-v2-backend
   ```

   The intent itself still needs maker/taker EIP-712 signatures —
   those come from the standalone CLIs below (or from any wallet
   client that constructs a backend-API-shaped order with a valid
   signature).

2. **Standalone signing CLIs** (used for ad-hoc smoke signing):

   - `cargo run --bin sign_perp_trade -- --payload <path> --role buyer|seller`
     reads `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` from env, with
     `SIGNER_PRIVATE_KEY` as a fallback. All three are shell-only.
   - `cargo run --bin sign_option_execution_intent -- --payload-file
     <path> [--private-key-env <VAR>]` lets the operator point at any
     env var name (default `BUYER_PRIVATE_KEY`). Same shell-only
     pattern.

   Operator workflow for V2G-D2 EOAs without `.env` mutation:

   ```sh
   # Operator-only — neither variable is ever written to disk or
   # committed; `--private-key-env` lets `sign_option_execution_intent`
   # accept the V2G-D2 maker/taker key under any env name.
   export MAKER_PK=$(jq -r '.[0].private_key' \
     ~/.local/secrets/deopt-v2g-d2/tier4_maker.json)
   export TAKER_PK=$(jq -r '.[0].private_key' \
     ~/.local/secrets/deopt-v2g-d2/tier2_taker.json)

   MAKER_PK="$MAKER_PK" cargo run --bin sign_option_execution_intent -- \
     --payload-file <maker-payload>.json --private-key-env MAKER_PK \
     > maker-sig.json
   TAKER_PK="$TAKER_PK" cargo run --bin sign_option_execution_intent -- \
     --payload-file <taker-payload>.json --private-key-env TAKER_PK \
     > taker-sig.json

   # POST the resulting signed intent to the backend's intent
   # creation path (RFQ acceptance or order submission).
   unset MAKER_PK TAKER_PK
   ```

   `sign_perp_trade` uses the role-named env vars; for V2G-D2 set
   `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` in the shell only.

Result: **no code change required for V2G-F to support real-trader
smokes via the backend executor**. The only friction with V2G-E was
that the orderbook/RFQ flow requires the maker/taker addresses to
match the on-chain trader (which is checked by `--role` in
sign_perp_trade); operators using V2G-D2 EOAs need to either set the
matching addresses in the shell or use `--allow-address-mismatch`
intentionally. Documented in `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md`
implicitly via the existing operator pattern.

A future small task could add a single-flag shortcut
(`--keys-file <path>`) to either CLI to read both maker and taker
keys from a redacted JSON file in one call, removing the per-role
env juggling. Out of scope for V2G-F.

## Phase 6 — Live read-only verification

Backend rebuilt (`cargo build --release`) with the V2G-F additions
and started in read-only mode (`EXECUTION_ENABLED=false`,
`EXECUTOR_REAL_BROADCAST_ENABLED=false`, `EXECUTOR_PRIVATE_KEY`
unset) with shell-only env overrides for the metric classifier
(`PERP_ENGINE_ADDRESS=NEW`, `OLD_PERP_ENGINE_ADDRESS=OLD`,
`MARGIN_ENGINE=NEW`, `OLD_MARGIN_ENGINE_ADDRESS=OLD`).

`/admin/fees/onchain` for both V2G-E txs:

| Field | PERP tx `0x5c15…aa394` | OPTION tx `0x9a85…3149` |
|---|---|---|
| `event_model` | `v2` | `mixed` |
| `fee_charged_v2_count` | `1` | `1` |
| `fee_rebated_v2_count` | `1` | `1` |
| `observed_total_charged` | `6` | `25` |
| `observed_total_rebated` | `3` | `10` |
| `net_protocol_fee` | `3` | `15` |
| `reconciliation_status` | `onchain_observed` | `onchain_observed` |
| `source_of_truth` | `onchain` | `onchain` |
| `trading_fee_event_count` | n/a | `1` (V1-compat for the taker fee leg) |

`/metrics` scrape:

```
deopt_perp_fee_charged_v2_total{consumer="new"}     = 3      # 2 V2F-LM + 1 V2G-E
deopt_perp_fee_charged_v2_total{consumer="old"}     = 0
deopt_perp_fee_charged_v2_total{consumer="unknown"} = 0
deopt_perp_fee_rebated_v2_total{consumer="new"}     = 1      # V2G-E
deopt_perp_fee_rebated_v2_total{consumer="old"}     = 0
deopt_perp_fee_rebated_v2_total{consumer="unknown"} = 0

deopt_option_fee_charged_v2_total{consumer="new"}     = 3    # 2 V2E-G + 1 V2G-E
deopt_option_fee_charged_v2_total{consumer="old"}     = 0
deopt_option_fee_charged_v2_total{consumer="unknown"} = 0
deopt_option_fee_rebated_v2_total{consumer="new"}     = 1    # V2G-E
deopt_option_fee_rebated_v2_total{consumer="old"}     = 0
deopt_option_fee_rebated_v2_total{consumer="unknown"} = 0

deopt_fees_manager_v2_rebate_budget_native{
  asset="0x6eae407f5640b006fac9965182e238582a3b412e"
} = 999987   # matches FMv2.rebateBudget(mUSDC) on chain
```

OLD-consumer alerts (PERP + OPTION) stay **green** (`old=0` on every
arm). Unknown-consumer alerts stay **green** (`unknown=0`). The
derived budget gauge matches the on-chain
`FeesManagerV2.rebateBudget(mUSDC)` reading exactly (the operator
last queried it post-V2G-E as `999_987`). No double counting.

The indexer was kept idempotent — only `POST /admin/options/events/tick`
was called (read-only / idempotent admin endpoint, not destructive).

Backend was then stopped (`pkill -f 'target/release/deopt-v2-backend'`).

## Phase 7 — Docs

Created:
- `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` (this file).
- `docs/alertmanager/option_v2_fee_alerts.yml`.

Updated:
- `.env.example` — MARGIN_ENGINE, OLD_MARGIN_ENGINE_ADDRESS, FEES_MANAGER_V2 entries with documentation.
- `docs/ALERTING_SPEC.md` — OPTION V2 fee alerts (V2G-F), derived budget alert (V2G-F), retired merkle-root-unset notice.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — title + intro now cover PERP+OPTION; new sections for the four V2G-F alerts.
- `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` — V2G-F closure note (see "V2G-F follow-up").

## Phase 8 — Validation

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo build --all-targets --all-features` | ✅ clean |
| `cargo test --all-targets --all-features --no-fail-fast` | ✅ **675 passed, 0 failed, 0 ignored** (was 661 in V2G-E, +14 V2G-F tests) |

Solidity / frontend: untouched in V2G-F scope.

## Files changed

Backend:

- `src/fees/mod.rs` — added `pub mod option_consumer;`.
- `src/fees/option_consumer.rs` — **NEW**.
- `src/options/event_indexer.rs` — added
  `OptionEventIndexerConfig.old_margin_engine_address: Option<AccountId>`
  + threaded through the disabled-constructor and tests.
- `src/options/store.rs` — added `option_fee_v2_consumer_counts`,
  `option_fee_v2_rebated_consumer_counts`,
  `fees_manager_v2_rebate_budget_by_asset`; generalised the internal
  helper to take `product_kind`.
- `src/db/repository.rs` — added
  `admin_option_fee_v2_consumer_counts`,
  `admin_option_fee_v2_rebated_consumer_counts`,
  `admin_fees_manager_v2_rebate_budget_by_asset`; refactored the
  PERP helper to delegate to a generic
  `admin_fee_v2_consumer_counts_for_event_and_product`.
- `src/monitoring.rs` — wired OPTION + budget metrics into
  `append_fee_metrics`; added `append_option_fee_v2_consumer_metric`.
- `src/config/env.rs` — read `OLD_MARGIN_ENGINE_ADDRESS` and thread
  into the indexer config.
- `src/api/routes.rs` — added OPTION-flavoured fee log builders
  (`build_fee_charged_v2_option_log_row`,
  `build_fee_charged_v2_option_log_row_for_consumer`,
  `build_fee_rebated_v2_option_log_row`,
  `build_fee_rebated_v2_option_log_row_for_consumer`); added
  `build_rebate_budget_event` helper; added 7 OPTION metric +
  budget gauge tests; patched 3 fixture constructors with the new
  `old_margin_engine_address: None` field.

Configs / templates:

- `.env.example` — MARGIN_ENGINE, OLD_MARGIN_ENGINE_ADDRESS,
  FEES_MANAGER_V2 entries with documentation.

Docs:

- `docs/alertmanager/option_v2_fee_alerts.yml` — **NEW**.
- `docs/ALERTING_SPEC.md` — appended V2G-F section + retired
  merkle-root notice.
- `docs/RUNBOOK_PERP_V2_FEE_ALERTS.md` — extended for PERP+OPTION +
  budget alert.
- `docs/FEE_OBSERVABILITY_ENV_HYGIENE_CLOSURE_V2G_F.md` — **NEW**.
- `docs/FEES_MANAGER_V2_LIVE_REBATE_SMOKE_RESULT_V2G_E.md` — V2G-F
  closure note.

## Remaining blockers

1. The local-only real `.env` still references the OLD PerpEngine in
   `PERP_ENGINE_ADDRESS` (V2F-O carry-over). V2G-F provides the
   operator-only patch above but does not apply it because the hard
   rules forbid editing the real `.env`. The shell-only override
   pattern remains valid as a fallback.
2. Multi-asset settlement: the rebate-budget alert is keyed on the
   canonical mUSDC address today; multi-asset environments need one
   rule per supported asset (the metric pipeline already supports
   multiple `asset=...` series — see
   `fees_manager_v2_rebate_budget_metric_reflects_funded_minus_spent_and_withdrawn`).
3. The `sign_option_execution_intent` / `sign_perp_trade` CLIs work
   today for V2G-D2-EOA signing; a future minor task could add a
   `--keys-file` flag that reads both maker + taker keys from a
   single redacted JSON to remove per-role env juggling. Out of
   scope here.

## Next recommended milestone

**V2G-G — productionise the V2 fee surface beyond Base Sepolia.**

- Configure the V2G-F alert rules in the actual Alertmanager
  deployment and confirm delivery channels (Slack / PagerDuty /
  whatever the operator picks). The rules are deployable today;
  V2G-G is the rollout step.
- Build a Grafana dashboard fed by the V2F-P/V2F-Q PERP metrics +
  V2G-F OPTION metrics + budget gauge, with a panel per consumer
  bucket and a budget timeseries per supported asset.
- Wire the merkle-root continuous probe into the dashboard (V2G-F
  retired the operational notice; V2G-G can make it visible).
- Optional: extend the metric pipeline to mainnet's settlement
  asset(s) once V2G band ships beyond Base Sepolia.
