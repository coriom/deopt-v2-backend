# Runbook — PERP V2 Fee Alerts

This runbook covers the three Prometheus alerts emitted from the V2F
PERP V2 fee observability surface:

- `PerpFeeChargedFromOldEngine` (V2F-O / V2F-P)
- `PerpFeeRebatedFromOldEngine` (V2F-Q)
- `PerpFeeConsumerUnknown` (V2F-Q)

All three are derived from the gauges:

```
deopt_perp_fee_charged_v2_total{consumer="new"|"old"|"unknown"}
deopt_perp_fee_rebated_v2_total{consumer="new"|"old"|"unknown"}
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
