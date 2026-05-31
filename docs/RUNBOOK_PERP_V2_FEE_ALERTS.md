# Runbook — PERP + OPTION V2 Fee Alerts

This runbook covers the Prometheus alerts emitted from the FeesManagerV2
observability surface. Despite the historical name, the runbook now
spans both PERP (V2F) and OPTION (V2G-F) alerts — the diagnostic
procedure is identical, only the addresses change.

PERP alerts (V2F):

- `PerpFeeChargedFromOldEngine` (V2F-O / V2F-P)
- `PerpFeeRebatedFromOldEngine` (V2F-Q)
- `PerpFeeConsumerUnknown` (V2F-Q)

OPTION alerts (V2G-F, mirrors of the PERP set):

- `OptionFeeChargedFromOldMarginEngine`
- `OptionFeeRebatedFromOldMarginEngine`
- `OptionFeeConsumerUnknown`

Cross-cutting alerts (V2G-F + V2G-G):

- `FeesManagerV2RebateBudgetLow` (V2G-F)
- `FeesManagerV2RebateBudgetStale` (V2G-G — budget unchanged while
  rebates fire)
- `DeoptV2FeeMetricsAbsent` (V2G-G — metric pipeline down)

## Operator integration commands (V2G-H)

When standing the V2 fee observability surface up in a new operator
environment, refer to
`docs/V2_FEE_OBSERVABILITY_LIVE_STACK_WIRING_V2G_H.md` for the full
playbook. The short form:

1. **Prometheus** — drop
   `docs/monitoring/prometheus/v2_fee_alerts.bundle.yml` into the
   rules directory, reference it from `prometheus.yml`, validate via
   `promtool check rules` + `promtool test rules
   docs/monitoring/prometheus/v2_fee_alerts.test.yml`, graceful
   reload (`curl -X POST .../-/reload`). Expect 9 rules in 4 groups.
2. **Alertmanager** — merge
   `docs/monitoring/alertmanager/v2_fee_routing.example.yml` into
   the deployed `alertmanager.yml`, validate via `amtool
   check-config`, optionally `amtool config routes test` each of the
   four sample alerts, graceful reload.
3. **Grafana** — run
   `docs/monitoring/grafana/provisioning/render_dashboard.sh
   <prometheus_ds_name>` to materialise the dashboard JSON, drop it
   into the dashboards directory, copy the provisioning entry from
   `docs/monitoring/grafana/provisioning/dashboards/deopt_v2_fees.yaml`,
   reload Grafana.
4. **Operator `.env`** — apply
   `docs/operator/v2g_g_env_patch.example.env` to the gitignored
   local `.env`. Verify with the snippets at the bottom of that
   file.

## Quick admin probe (V2G-G)

Before going deep into any alert below, hit the backend's read-only
V2 fee observability snapshot — it returns the same data the four V2
fee gauges expose plus the configured engine addresses, in a single
JSON:

```sh
curl -sH "x-admin-token: $ADMIN_API_TOKEN" \
  http://<backend>/admin/fees/v2/observability | jq '
    .contracts,
    .anomaly_totals,
    .metrics.perp_fee_charged_v2_by_consumer,
    .metrics.perp_fee_rebated_v2_by_consumer,
    .metrics.option_fee_charged_v2_by_consumer,
    .metrics.option_fee_rebated_v2_by_consumer,
    .metrics.fees_manager_v2_rebate_budget_native'
```

The contracts block lets you confirm the metric classifier is using
the right NEW / OLD engines (alert misclassification — e.g. the
classifier was reset to the OLD address — is otherwise invisible).
The anomaly totals roll up `{old, unknown}` across PERP + OPTION
charged + rebated and mirror exactly what the alert preconditions
compute.

The frontend admin dashboard also renders this snapshot under the
"V2 Fee Observability (V2G-G)" section.

All consumer-bucket alerts are derived from the gauges:

```
deopt_perp_fee_charged_v2_total{consumer="new"|"old"|"unknown"}
deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}
deopt_option_fee_charged_v2_total{consumer="new"|"old"|"unknown"}
deopt_option_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}
deopt_fees_manager_v2_rebate_budget_native{asset="<lowercased address>"}
```

Source-of-truth metric definitions: `docs/ALERTING_SPEC.md`.
Deployable Prometheus rules: `docs/alertmanager/perp_v2_fee_alerts.yml`.
Milestone records:
`docs/PERP_V2_FEE_METRICS_ALERTING_V2F_P.md`,
`docs/PERP_V2_FEE_REBATE_METRICS_ALERTING_V2F_Q.md`.

## Severity matrix

| Environment | Charged/Rebated from OLD | Unknown consumer |
| --- | --- | --- |
| Base Sepolia | **high** | **medium** |
| Mainnet | **critical** | **high** |

Severity is overridden at the Alertmanager rules layer, not at the
metric level. The shipped YAML defaults to Base Sepolia values.

## Common forensics checks (run for all three alerts)

Quote the alert payload verbatim (label values + the
`runbook_url` annotation) into the incident channel before running
any command — these alerts are written so the alert body alone is
enough to start an investigation.

1. **Scrape `/metrics` and read the three buckets directly:**

   ```
   curl -s http://<backend>/metrics \
     | grep -E 'deopt_perp_fee_(charged|rebated)_v2_total'
   ```

   You should see exactly three series per metric, with `consumer`
   in `{new, old, unknown}`. Any other label value is a regression
   in the metric pipeline itself — page the backend on-call before
   chasing a contract bug.

2. **Find the offending PERP fee/rebate row in the indexer:**

   ```sql
   SELECT tx_hash, log_index, block_number,
          decoded->>'consumer'    AS consumer,
          decoded->>'productKind' AS product_kind,
          decoded->>'flowKind'    AS flow_kind,
          decoded->>'feePpm'      AS fee_ppm,
          decoded->>'basisAmount' AS basis_amount,
          decoded->>'feeAmount'   AS fee_amount,
          decoded->>'rebateAmount' AS rebate_amount
     FROM option_execution_events
    WHERE event_name IN ('FeeChargedV2','FeeRebatedV2')
      AND decoded->>'productKind' = 'perp'
    ORDER BY block_number DESC, log_index DESC
    LIMIT 50;
   ```

   Preserve the rows; **do not delete**. The runbook never authorises
   a `DELETE`. If a row needs to be filtered, do it at the metric/
   analytics layer.

3. **Resolve the suspect tx through the admin endpoint:**

   ```
   GET /admin/fees/onchain?tx_hash=<tx>
   ```

   The per-event payload should include `product_kind = "perp"`,
   `flow_kind = "orderbook"`, and a `source_contract` equal to
   `FEES_MANAGER_V2`. The decoded `consumer` topic is in the raw
   `option_execution_events.decoded` JSON.

4. **Snapshot the on-chain fee wiring on both engines:**

   ```
   cast call $PERP_MATCHING_ENGINE_ADDRESS 'perpEngine()(address)'
   cast call $PERP_ENGINE_ADDRESS         'useFeesManagerV2()(bool)'
   cast call $PERP_ENGINE_ADDRESS         'feesManagerV2()(address)'
   cast call $OLD_PERP_ENGINE_ADDRESS     'useFeesManagerV2()(bool)'
   cast call $OLD_PERP_ENGINE_ADDRESS     'feesManagerV2()(address)'
   cast call $FEES_MANAGER_V2             'isFeeConsumer(address)(bool)' $PERP_ENGINE_ADDRESS
   cast call $FEES_MANAGER_V2             'isFeeConsumer(address)(bool)' $OLD_PERP_ENGINE_ADDRESS
   ```

   Compare to the V2F-LM acceptance snapshot:
   - `NEW.useFeesManagerV2() == true`
   - `NEW.feesManagerV2()    == FEES_MANAGER_V2`
   - `FeesManagerV2.isFeeConsumer(NEW) == true`
   - `FeesManagerV2.isFeeConsumer(OLD) == false`

5. **Cross-check the deployment manifest:**

   `~/DEOPT/deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`
   is the canonical post-V2F record of NEW vs OLD engine addresses.
   Any divergence from `.env`'s `PERP_ENGINE_ADDRESS` /
   `OLD_PERP_ENGINE_ADDRESS` is itself an investigation finding —
   capture it in the incident channel.

## Alert-specific responses

### PerpFeeChargedFromOldEngine
<a id="perpfeechargedfromoldengine"></a>

- **Meaning.** An indexed PERP `FeeChargedV2` event was emitted with
  `decoded.consumer == OLD_PERP_ENGINE_ADDRESS`. After V2F-LM, OLD
  is stranded under the A3 fallback and is expected to emit zero
  PERP V2 fees.
- **Expected current value.** Zero.
- **First action (under 5 min).** Run forensics check 1 to confirm
  the metric is real (not a label-leak regression).
- **Investigation.** Run checks 2–5 to identify the tx, the
  emitter, the wiring state, and the manifest expectation.
- **Remediation (in order).**
  1. If `FeesManagerV2.isFeeConsumer(OLD)` is now `true`, pause
     PerpEngine writes (`PerpMatchingEngine.setPaused(true)` or
     equivalent) on **both** OLD and NEW until on-call decides how
     to revert.
  2. If `OLD.useFeesManagerV2()` is now `true`, disable it
     (`OLD.setUseFeesManagerV2(false)`).
  3. If neither is the case but `OLD` still produced the event,
     freeze the matching engine pointing at `OLD` and escalate to
     Solidity on-call — this implies a deeper contract regression.
  4. **Do not** delete the offending DB rows. They are the
     forensics record.
  5. After remediation, leave the alert firing until the
     `increase(...)[5m]` window has closed (no new OLD events for
     5 minutes).
- **Severity.** Base Sepolia: high. Mainnet: critical.

### PerpFeeRebatedFromOldEngine
<a id="perpfeerebatedfromoldengine"></a>

- **Meaning.** Same as `PerpFeeChargedFromOldEngine` but for the
  rebate event family. Implies OLD is paying out (or accruing)
  PERP rebates from the `FeesManagerV2` rebate budget.
- **Expected current value.** Zero (no PERP rebate has been live on
  Base Sepolia yet; the V2F-LM PERP smoke emitted zero rebates).
- **First action.** Same as `PerpFeeChargedFromOldEngine` — confirm
  the metric pipeline is healthy, then run forensics 2–5.
- **Additional check specific to rebates.** Snapshot the rebate
  budget on `FeesManagerV2` for the settlement asset that the
  offending event reported:

  ```
  cast call $FEES_MANAGER_V2 'rebateBudget(address)(uint256)' <settlementAsset>
  ```

  A nonzero budget against the wrong consumer is the regression
  signal; capture the before/after delta in the incident channel.
- **Remediation.** Same three steps as the charged variant, plus:
  if `OLD` accrued a rebate that has not yet been claimed, set the
  matching consumer's rebate cap to zero before any other write to
  pin the loss.
- **Severity.** Base Sepolia: high. Mainnet: critical.

### PerpFeeConsumerUnknown
<a id="perpfeeconsumerunknown"></a>

- **Meaning.** A PERP `FeeChargedV2` or `FeeRebatedV2` event was
  indexed whose `decoded.consumer` matches neither
  `PERP_ENGINE_ADDRESS` (NEW) nor `OLD_PERP_ENGINE_ADDRESS`. The
  classifier put it in the `unknown` bucket.
- **Expected current value.** Zero.
- **Most common causes (rank-ordered).**
  1. Env-var drift — a redeploy did not roll the new
     `PERP_ENGINE_ADDRESS` to the backend. Check the running
     pod/container's env and compare to the deployment manifest.
  2. `OLD_PERP_ENGINE_ADDRESS` is unset on the backend but the
     event genuinely came from OLD. Set the env var, restart, and
     verify the bucket flips from `unknown` to `old`.
  3. A third engine has been deployed and added as a FeesManagerV2
     consumer without updating the classifier. File a follow-up
     milestone to extend the bucket vocabulary (e.g.
     `consumer="next_gen"`); do **not** widen `new` or `old` to
     absorb it silently.
  4. Decoder regression — `decoded.consumer` is null or zero.
     Inspect `option_execution_events.raw_topics` / `raw_data` for
     the suspect tx and re-run the decoder locally.
- **First action.** Run forensics check 1 (metric shape) and check
  2 (find the row). The decoded `consumer` value tells you
  which of the four causes applies.
- **Remediation.** Drive the metric back to zero through the cause
  identified — config rollout, classifier extension, or decoder
  fix. Never silently re-bucket the address.
- **Severity.** Base Sepolia: medium. Mainnet: high.

## Do-not list

- Do **not** delete `option_execution_events` rows during or after
  the incident — they are the forensics record.
- Do **not** broadcast a transaction from the backend host while
  investigating. The runbook authorises read-only checks
  (`cast call`, `eth_call`-style queries, admin GETs) only. Any
  on-chain mitigation (pausing the matching engine, flipping
  `useFeesManagerV2`) is done from the operator workstation
  signing path, not from the backend.
- Do **not** mutate `.env` or `.env.*.local` during the incident.
  Snapshot the running config (`/proc/<pid>/environ` for the
  backend process, or the container env) into the incident
  channel; roll any config change through the normal deployment
  flow afterward.
- Do **not** widen the classifier buckets to swallow an unknown
  consumer. If a new engine is legitimate, extend the vocabulary;
  if it is not, fix the source.

### OptionFeeChargedFromOldMarginEngine
<a id="optionfeechargedfromoldmarginengine"></a>

- **Meaning.** An indexed OPTION `FeeChargedV2` event was emitted
  with `decoded.consumer == OLD_MARGIN_ENGINE_ADDRESS`. Since V2E-E
  the legacy MarginEngine (`0x6C5665De…b5F8`) is superseded by the
  V2 MarginEngine (`0x287Cef…48Cc`); the OLD address is observability-
  only and is expected to emit zero V2 OPTION fees.
- **Expected current value.** Zero.
- **First action.** Run forensics check 1 (substitute the OPTION
  metric name) to confirm the metric is real (not a label-leak
  regression).
- **Investigation.** Mirror checks 2–5 with the OPTION analogues:
  - SQL filter: `decoded->>'productKind' = 'option'`.
  - `cast call $MARGIN_ENGINE 'useFeesManagerV2()(bool)'` (NEW) and
    `cast call $OLD_MARGIN_ENGINE_ADDRESS 'useFeesManagerV2()(bool)'`
    (legacy — should be `false` if the env var is even set).
  - `cast call $FEES_MANAGER_V2 'isFeeConsumer(address)(bool)'
     $MARGIN_ENGINE` (expect `true`) and the same for the OLD
    address (expect `false`).
- **Remediation.**
  1. If `FeesManagerV2.isFeeConsumer(OLD_MARGIN)` is now `true`,
     freeze the OptionMatchingEngine writes pointing at the legacy
     MarginEngine until on-call decides how to revert.
  2. If `OLD_MARGIN.useFeesManagerV2()` is `true`, disable it.
  3. **Do not** delete the offending DB rows — forensics record.
- **Severity.** Base Sepolia: high. Mainnet: critical.

### OptionFeeRebatedFromOldMarginEngine
<a id="optionfeerebatedfromoldmarginengine"></a>

- **Meaning.** Mirror of `OptionFeeChargedFromOldMarginEngine` for
  the rebate path. Implies the legacy MarginEngine is paying out
  OPTION rebates from the FeesManagerV2 rebate budget.
- **Expected current value.** Zero (V2G-E is the only live OPTION
  rebate event so far, emitted from NEW).
- **First action / investigation.** Identical to the charged
  variant; add the `rebateBudget(<asset>)` snapshot check from the
  PERP rebate runbook so the before/after budget delta is captured.
- **Severity.** Base Sepolia: high. Mainnet: critical.

### OptionFeeConsumerUnknown
<a id="optionfeeconsumerunknown"></a>

- **Meaning.** An OPTION `FeeChargedV2` or `FeeRebatedV2` event was
  indexed whose `decoded.consumer` matches neither `MARGIN_ENGINE`
  (NEW) nor `OLD_MARGIN_ENGINE_ADDRESS`. The
  `classify_option_fee_consumer` classifier put it in `unknown`.
- **Expected current value.** Zero.
- **Most common causes** (rank-ordered, parallel to the PERP cases):
  1. Env-var drift on `MARGIN_ENGINE` /
     `OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS`.
  2. `OLD_MARGIN_ENGINE_ADDRESS` unset but the event genuinely came
     from the legacy address.
  3. A third MarginEngine was deployed and added as a FeesManagerV2
     consumer without updating the classifier.
  4. Decoder regression — `decoded.consumer` is null or zero.
- **Remediation.** Same as `PerpFeeConsumerUnknown`. Never widen
  `new` / `old` to silently absorb the address.
- **Severity.** Base Sepolia: medium. Mainnet: high.

### FeesManagerV2RebateBudgetLow
<a id="feesmanagerv2rebatebudgetlow"></a>

- **Meaning.** The derived
  `deopt_fees_manager_v2_rebate_budget_native{asset="<addr>"}` gauge
  for the canonical settlement asset (mUSDC on Base Sepolia) has
  fallen below `1000` native units (`0.001 mUSDC` at 6 decimals).
- **Derivation.** Sum of indexed `RebateBudgetFunded.amount` minus
  `RebateBudgetSpent.amount` minus `RebateBudgetWithdrawn.amount`,
  per `decoded.settlementAsset`, clamped at `0`. If the indexer is
  briefly behind a withdraw the gauge floors at zero rather than
  wrap-underflowing.
- **Expected current value.** Greater than the threshold; on V2G-D2
  the on-chain budget is `1 000 000` native and V2G-E spent only
  `13`.
- **First action (under 5 min).** Read the on-chain ground truth:

  ```
  cast call $FEES_MANAGER_V2 'rebateBudget(address)(uint256)' $MUSDC
  ```

  - If on-chain matches the metric (low), proceed to remediation.
  - If on-chain is healthy but the metric is low, suspect indexer
    lag or a missed `RebateBudgetFunded` event. Catch the indexer
    up via the existing admin tick and recompute.
- **Remediation.** Top up via `FeesManagerV2.fundRebateBudget(mUSDC,
  amount)` from the operator workstation. Document the operator
  packet in the incident channel before broadcast.
- **Do-not.** Do not silence the alert by raising the threshold
  inline; if the alert is too noisy for testnet, override at the
  Alertmanager rules layer per the severity matrix.
- **Severity.** Base Sepolia: medium. Mainnet: high.

### FeesManagerV2RebateBudgetStale (V2G-G)
<a id="feesmanagerv2rebatebudgetstale"></a>

- **Meaning.** The derived
  `deopt_fees_manager_v2_rebate_budget_native{asset="<addr>"}` gauge
  has not moved in 30 m, but the OPTION or PERP
  `FeeRebatedV2{consumer="new"}` counter has incremented inside the
  same window. The metric pipeline expects every rebate event to
  produce a `RebateBudgetSpent` event in the same tx; if the budget
  gauge does not move, the indexer is most likely lagging behind
  those `RebateBudgetSpent` events.
- **Expected current value.** No firing — every V2G-E rebate event
  was paired with a `RebateBudgetSpent` in the same block.
- **First action (under 5 min).** Cross-check the on-chain ground
  truth and the running gauge:

  ```sh
  cast call $FEES_MANAGER_V2 'rebateBudget(address)(uint256)' $MUSDC --rpc-url $RPC_URL
  curl -sH "x-admin-token: $ADMIN_API_TOKEN" \
    http://<backend>/admin/fees/v2/observability \
    | jq '.metrics.fees_manager_v2_rebate_budget_native'
  ```

  - If on-chain and gauge agree, the alert is a false positive —
    likely the cadence assumption (one PERP rebate every 24 h on
    testnet) is wrong; downgrade severity at the Alertmanager layer
    or relax the rule window.
  - If on-chain and gauge disagree, the indexer has fallen behind.
- **Investigation.** Tail the indexer cursor and the most-recent
  `RebateBudget*` event row:

  ```
  GET /admin/options/events                # event tail
  POST /admin/options/events/tick          # idempotent catch-up
  ```

  After ticking, recompute the gauge from `/admin/fees/v2/observability`
  and confirm the delta clears.
- **Remediation.** Bring the indexer cursor up; if `RebateBudgetSpent`
  rows are missing for known rebate txs, re-index the affected block
  range via the admin tick. Do **not** delete rows.
- **Severity.** Base Sepolia: medium. Mainnet: high.

### DeoptV2PerpRebateStalled (V2G-I, opt-in)
<a id="deoptv2perprebatestalled"></a>

- **Meaning.** `deopt_perp_fee_rebated_v2_total{consumer="new"}` has
  not incremented for 24 hours. Either the rebate path is silently
  broken (classifier put events in `unknown`, every maker landed on
  a non-rebate tier, fee consumer rewired) or the network has no
  rebate-eligible trade activity in the period.
- **Source-of-truth file.** This rule is **not** in the default
  `v2_fee_alerts.bundle.yml`. It ships as an opt-in file at
  `docs/monitoring/prometheus/v2_fee_alerts.stalled.yml`. Activation
  is operator-controlled — only enable once the target network has
  a stable rebate cadence (e.g. mainnet post-launch).
- **Expected current value.** N/A — Base Sepolia today has only V2G-E
  worth of PERP rebate flow. Activating the rule there would fire
  continuously until the next live trade.
- **First action.** Hit `/admin/fees/v2/observability` (the V2G-G
  admin probe) and read `perp_fee_rebated_v2_by_consumer`. If
  `unknown > 0` or `old > 0`, the classifier bucketed rebate events
  away from `new` — this is the silent-breakage scenario, not a
  cadence drop. Drive a forensics check via the
  `PerpFeeConsumerUnknown` / `PerpFeeRebatedFromOldEngine` runbook
  entries above.
- **If `unknown == old == 0`**, no rebate event was emitted at all in
  the 24h window. Compare PERP trade volume (via the
  `deopt_perp_*` counters or admin endpoints) to the rebate count:
  if volume is healthy but rebates are zero, every maker in the
  period landed on a non-rebate tier — investigate the tier
  classifier (`/admin/fees/v2/observability` doesn't surface tier
  state directly; query the FeesManagerV2 tier merkle leaves
  on chain).
- **Remediation.** Restore the rebate path or widen the alert
  window. The rule defaults to 24h; widen to 7d if the operator
  expects sparse rebate flow.
- **Severity.** Base Sepolia: not enabled. Mainnet: medium
  (anomaly, not regression).

### DeoptV2FeeMetricsAbsent (V2G-G)
<a id="deoptv2feemetricsabsent"></a>

- **Meaning.** One of the four V2 fee consumer-bucket gauges has
  been absent from `/metrics` for 5 m. The backend pre-seeds every
  bucket at zero on boot (see
  `src/monitoring.rs::append_perp_fee_v2_consumer_metric` /
  `append_option_fee_v2_consumer_metric`), so absence is a backend
  regression, not a quiet-period side effect.
- **Likely causes.**
  1. `METRICS_ENABLED=false` on the running backend.
  2. The scrape target is unreachable (process down, container
     restart loop, networking issue).
  3. The metric pipeline panicked or returned an error between
     scrapes (look at backend logs for `render_metrics` errors).
  4. Someone removed the V2 fee metric emission code (regression in
     `append_fee_metrics`).
- **First action.** Hit `/admin/fees/v2/observability` with the
  admin token. If the snapshot endpoint also fails, the backend is
  unreachable / not serving admin traffic — page the backend on-call,
  not the contract on-call. If the snapshot endpoint works but
  `/metrics` does not, check `METRICS_ENABLED`.
- **Remediation.** Restart the backend with `METRICS_ENABLED=true`
  and confirm `/metrics` is serving the four V2 fee gauges. If the
  metric code is missing, revert the regressing commit; the V2G-G
  / V2G-F test suite covers the metric emission contract.
- **Severity.** Base Sepolia: high. Mainnet: high. This alert
  should NEVER auto-resolve at lower severity — by construction the
  downstream OLD / unknown alerts cannot fire while the gauge is
  absent, so this alert is the safety net.

## Verifying the all-clear

The shipped rules use `increase(...)[5m] > 0`, so the alert clears
naturally once no new events land in the bucket for five minutes
after remediation. To confirm before paging out:

1. Scrape `/metrics` and check that the relevant bucket
   (`{consumer="old"}` or `{consumer="unknown"}`) is no longer
   advancing per scrape.
2. Query the DB:

   ```sql
   SELECT count(*)
     FROM option_execution_events
    WHERE event_name IN ('FeeChargedV2','FeeRebatedV2')
      AND decoded->>'productKind' = 'perp'
      AND lower(COALESCE(decoded->>'consumer','')) NOT IN
            (lower($1 /* NEW */), lower($2 /* OLD */))
      AND block_number > $3 /* block at incident start */;
   ```

   Bind `$1`/`$2` to the configured addresses and `$3` to the block
   the alert first fired at; the result should be zero after
   remediation.
