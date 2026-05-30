# V2F-O — Live PERP V2 Fee Observability Verification

## Status

- Milestone: **V2F-O** (follows V2F-N regression coverage).
- Date: 2026-05-30.
- Mode: backend safe read-only; **no Solidity, no broadcast, no live
  chain mutation, no DB destructive operations**.
- Outcome: **PASS** — live `/admin/fees/onchain?tx_hash=<V2F-LM>` returns
  every expected V2F-LM PERP fee value.

## Backend launch safety summary

Launched the release binary with three local overlay env files
sourced in order on top of `.env`:

```
.env
.env.cutover.v2d_s.local       (V2D-S NEW MarginEngine + indexer addresses)
.env.preflight.v2e_f.local     (FeesManagerV2 wired + indexer enabled)
.env.observability.v2f_o.local (new, this milestone — broadcast OFF, ADMIN ON)
```

The V2F-O overlay enforces every broadcast surface OFF and unsets
`EXECUTOR_PRIVATE_KEY` defensively:

```
EXECUTION_ENABLED=false
EXECUTOR_DRY_RUN=true
EXECUTOR_REAL_BROADCAST_ENABLED=false
OPTION_EXECUTION_BROADCAST_ENABLED=false
unset EXECUTOR_PRIVATE_KEY
```

Boot log confirmed:

```
execution_enabled=false
executor_dry_run=true
indexer_enabled=false  (V1 generic perp-orderbook indexer; unused for V2F-O)
option_event_indexer_enabled=true
persistence_enabled=true
metrics_enabled=true
mm_gateway_enabled=false
options_enabled=true
fees_enabled=false  (backend ledger off; on-chain is source of truth)
```

`/health` returned `200`. `/admin/options/events` returned the indexer
state, confirmed FeesManagerV2 is wired as an emitter, and confirmed
the V2F-LM-relevant emitter contracts:

```json
"emitter_contracts": [
  { "role": "matching_engine",  "contract_address": "0xf2d1…420b" },
  { "role": "margin_engine",    "contract_address": "0x287c…48cc" },
  { "role": "collateral_vault", "contract_address": "0x0034…25d3" },
  { "role": "fees_manager",     "contract_address": "0xaef7…a0f0" },
  { "role": "fees_manager_v2",  "contract_address": "0x00da…774f" }
]
```

`FEES_MANAGER_V2 = 0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f` is the
single source emitting the V2F-LM PERP fee logs; it is shared between
the option flow (MarginEngineV2 consumer) and the perp flow
(PerpEngineV2 consumer) and the indexer subscribes to it by address,
not by engine.

**Operator note (not a blocker for V2F-O):** `PERP_ENGINE_ADDRESS` in
`.env` still points at OLD (`0xB36395…b53B`). This variable is only
consumed by `PERP_NONCE_SYNC` (disabled in the V2F-O run) and is not
used by the V2 fee indexing path. A follow-up env hygiene pass should
flip it to NEW (`0xc6C592…141c`); the V2F-LM/V2F-O state holds because
the FeesManagerV2 emitter and `FeesManagerV2.isFeeConsumer(NEW)`
control which engine can spend fees, not the env var.

## Indexer cursor: baseline → final

| Phase | Cursor (`option_events_base_sepolia.last_indexed_block`) |
| --- | --- |
| Baseline (before V2F-O) | **42149417** |
| After backend boot (one auto-tick) | 42154417 |
| After 7 admin one-shot ticks | **42189720** |
| Target block (V2F-LM) | 42188599 ✅ covered |

Each `POST /admin/options/events/tick` advanced ~5000 blocks (the
configured `OPTION_EVENT_INDEXER_BATCH_BLOCKS`) and clamped to the
safe head (`current_head − 3` confirmation blocks). Tick 7 was the
one that crossed block 42188599 and indexed **17 logs / 17 events
decoded / 17 events persisted**, including the two V2F-LM
`FeeChargedV2` events. No errors. No DB destructive operations.

DB confirmation after catch-up:

```sql
SELECT event_name, log_index, decoded->>'productKind' AS pk,
       decoded->>'flowKind' AS fk, decoded->>'feePpm' AS feeppm,
       decoded->>'basisAmount' AS basis, decoded->>'feeAmount' AS amt,
       account, contract_address
  FROM option_execution_events
 WHERE tx_hash = '0x400acedf…ff63a79a' AND event_name = 'FeeChargedV2'
 ORDER BY log_index;

 event_name   | log_index |  pk  |    fk     | feeppm | basis | amt |                  account                   |              contract_address
--------------+-----------+------+-----------+--------+-------+-----+--------------------------------------------+--------------------------------------------
 FeeChargedV2 |       183 | perp | orderbook | 300    | 30    | 1   | 0x8b94a83d…0bba9a34 (buyer/taker)          | 0x00da0b9876…7ad774f (FEES_MANAGER_V2)
 FeeChargedV2 |       190 | perp | orderbook | 50     | 30    | 1   | 0x475fe397…64bc0c   (seller/maker)         | 0x00da0b9876…7ad774f (FEES_MANAGER_V2)
```

Zero `TradingFeeCharged` rows for this `tx_hash` — PerpEngineV2 does
**not** emit the V1 fee compatibility breadcrumb that MarginEngineV2
emits for options, so the V2F-LM PERP tx is pure V2 (no mixed-model
double-counting risk on this specific tx).

## Live `/admin/fees/onchain` result

```
GET /admin/fees/onchain?tx_hash=0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a
Header: X-Admin-Token: v2d_s_local_admin_token
```

Full response (read-only, no rows mutated):

```json
{
  "source_of_truth": "onchain",
  "event_model": "v2",
  "source_priority": "",
  "backend_ledger_enabled": false,
  "backend_ledger_status": "disabled",
  "filter": { "limit": 50, "tx_hash": "0x400acedf…ff63a79a" },
  "trading_fee_event_count": 0,
  "fee_charged_v2_count": 2,
  "fee_rebated_v2_count": 0,
  "observed_total": "2",
  "observed_total_charged": "2",
  "observed_total_rebated": "0",
  "net_protocol_fee": "2",
  "by_trader": {
    "0x475fe397fa56884952d350aa9ee1c3946964bc0c": "1",
    "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34": "1"
  },
  "by_recipient": { "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588": "2" },
  "by_side": { "maker": "1", "taker": "1" },
  "rebated_by_trader": {},
  "reconciliation_status": "onchain_observed",
  "transactions": [
    {
      "tx_hash": "0x400acedf…ff63a79a",
      "event_model": "v2",
      "source_priority": "",
      "trading_fee_event_count": 0,
      "fee_charged_v2_count": 2,
      "fee_rebated_v2_count": 0,
      "observed_total": "2",
      "observed_total_charged": "2",
      "observed_total_rebated": "0",
      "net_protocol_fee": "2",
      "by_recipient": { "0xa67f8e8e…b588": "2" },
      "by_trader": { "0x475f…bc0c": "1", "0x8b94…9a34": "1" },
      "by_side": { "maker": "1", "taker": "1" },
      "rebated_by_trader": {}
    }
  ],
  "events": [
    {
      "event_name": "FeeChargedV2",
      "event_model": "v2",
      "log_index": 183,
      "block_number": 42188599,
      "source_contract": "0x00da0b9876…7ad774f",
      "trader": "0x8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34",
      "recipient": "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588",
      "settlement_asset": "0x6eae407f5640b006fac9965182e238582a3b412e",
      "product_kind": "perp",
      "flow_kind": "orderbook",
      "is_maker": false,
      "side": "taker",
      "fee_ppm": 300,
      "basis_amount": "30",
      "fee_amount": "1",
      "applied_fee": "1",
      "rebate_amount": null
    },
    {
      "event_name": "FeeChargedV2",
      "event_model": "v2",
      "log_index": 190,
      "block_number": 42188599,
      "source_contract": "0x00da0b9876…7ad774f",
      "trader": "0x475fe397fa56884952d350aa9ee1c3946964bc0c",
      "recipient": "0xa67f8e8e673ce4bb2fb563b0e6e9fa8f70e3b588",
      "settlement_asset": "0x6eae407f5640b006fac9965182e238582a3b412e",
      "product_kind": "perp",
      "flow_kind": "orderbook",
      "is_maker": true,
      "side": "maker",
      "fee_ppm": 50,
      "basis_amount": "30",
      "fee_amount": "1",
      "applied_fee": "1",
      "rebate_amount": null
    }
  ]
}
```

### Acceptance checklist (live)

| Expected | Observed | Status |
| --- | --- | --- |
| `event_model = "v2"` (no V1 breadcrumb on perp path) | `"v2"` | ✅ |
| `productKind = perp` on every V2 event | both `"perp"` | ✅ |
| `flowKind = orderbook` on every V2 event | both `"orderbook"` | ✅ |
| `fee_charged_v2_count = 2` | `2` | ✅ |
| `fee_rebated_v2_count = 0` | `0` | ✅ |
| `basisAmount = 30` on both V2 events | both `"30"` | ✅ |
| Taker fee = 1 (`feePpm = 300`) | `feePpm=300`, `fee_amount="1"`, `side="taker"` | ✅ |
| Maker fee = 1 (`feePpm = 50`) | `feePpm=50`, `fee_amount="1"`, `side="maker"` | ✅ |
| `observed_total_charged = 2` | `"2"` | ✅ |
| `observed_total_rebated = 0` | `"0"` | ✅ |
| `net_protocol_fee = 2` | `"2"` | ✅ |
| recipient = `0xa67f8e8e…b588` | `"0xa67f8e8e…b588"` | ✅ |
| No rebate event | `rebated_by_trader = {}`, `fee_rebated_v2_count = 0` | ✅ |
| `reconciliation_status` reflects observed events | `"onchain_observed"` | ✅ |

## No-double-counting proof from live data

PerpEngineV2 emits **zero V1 `TradingFeeCharged` breadcrumbs** for this
transaction (verified by `SELECT count(*) FROM option_execution_events
WHERE tx_hash = '0x400acedf…ff63a79a' AND event_name =
'TradingFeeCharged'` → `0`). The summary therefore resolves to
`event_model = "v2"` with `source_priority = ""` (no resolution
needed) and `trading_fee_event_count = 0`. There is no V1 leg to
double-count, and `observed_total = "2" = 1 (taker) + 1 (maker)`.

The mixed V1+V2 PERP path (where PerpEngineV2 *would* additionally
emit a V1 breadcrumb during a bridging window) is regression-pinned
at both the aggregator level
(`fees::onchain_summary::tests::v2f_lm_mixed_v1_v2_perp_does_not_double_count`)
and the admin-endpoint level
(`api::routes::tests::admin_fees_onchain_v2f_lm_perp_mixed_does_not_double_count`)
added in V2F-N. Both pass `cargo test`.

## Admin UI result (code-level verification only)

A live frontend boot was not executed because the existing admin UI
already renders the per-event payload generically. Code review of
`~/DEOPT/deopt-v2-frontend/src/app/admin/admin-dashboard.tsx`
(lines `1830-1905`) confirms the `FeeChargedV2` / `FeeRebatedV2`
cards build their fields from per-event keys:

- `Product Kind` ← `entry.product_kind ?? entry.productKind`
- `Flow Kind` ← `entry.flow_kind ?? entry.flowKind`
- `Basis Amount` ← `entry.basis_amount ?? entry.basisAmount`
- `Fee Ppm` ← `entry.fee_ppm ?? entry.feePpm`
- `Fee Amount` ← `entry.fee_amount ?? entry.feeAmount`
- `Is Maker` ← `entry.is_maker ?? entry.isMaker`

Our live response populates all six fields per FeeChargedV2 entry
(`"perp"`, `"orderbook"`, `"30"`, `300/50`, `"1"`,
`false/true`). No frontend code change is required for V2F-O.

A live admin UI smoke is deferred to the next session that boots the
Next.js dev server; nothing in the V2F-O scope requires it.

## OLD stranded alert

Added a new entry under `docs/ALERTING_SPEC.md` →
**PERP FeeChargedV2 From OLD PerpEngine (V2F-O)**:

- name: `perp_fee_charged_from_old_engine`
- condition: `decoded.productKind == "perp"` AND
  `decoded.consumer == OLD_PERP_ENGINE` on any indexed `FeeChargedV2`
  event.
- severity: high on Base Sepolia, critical on mainnet.
- expected value: zero (validated against the V2F-LM live data, where
  `decoded.consumer == NEW_PERP_ENGINE` for both events).
- delivery is spec-only per the V1B alerting policy; the metric must
  be low-cardinality (e.g., `consumer="old"|"new"`) and must not
  promote the OLD address itself to a label.

## Backend changes made

None. The V2F-N work already shipped:
- product-agnostic FeeChargedV2/FeeRebatedV2 decoding (`productKind`
  raw → label),
- FeesManagerV2 emitter subscription (PerpEngineV2-emitted fees flow
  through unchanged because the indexer keys on the FeesManagerV2
  address, not on which engine called `chargeFee`),
- per-tx admin grouping (`summarize_admin_onchain` is product-agnostic),
- V1/V2 double-counting policy (`classify_event_model`).

V2F-O contributed a non-committed runtime overlay
(`.env.observability.v2f_o.local`) and the alerting spec entry.
No source files in `src/` were modified during V2F-O.

## Frontend changes made

None. Admin UI is product-agnostic and renders `productKind = perp`
and `flowKind = orderbook` from the existing per-event payload keys
without modification.

## Docs updated

- `docs/ALERTING_SPEC.md` — added the `perp_fee_charged_from_old_engine`
  alert (V2F-O).
- `docs/PERP_V2_FEE_LIVE_OBSERVABILITY_VERIFICATION_V2F_O.md` — this
  file (new).
- `docs/PERP_V2_FEE_BACKEND_CUTOVER_V2F_N.md` — appended a "V2F-O live
  verification confirmation" section pointing readers at the live
  results above.

## Validation commands run

```
cargo fmt --all                                         # no diff
cargo clippy --all-targets --all-features -- -D warnings  # clean
cargo test --all-targets --all-features --no-fail-fast    # all suites pass
cargo build --all-targets --all-features                  # finishes clean
```

Frontend validations not run (no frontend changes).

## V2F-P metric & alert instrumentation (2026-05-30 follow-up)

V2F-P landed the observability primitive the alert spec assumed:

- New metric
  `deopt_perp_fee_charged_v2_total{consumer="new"|"old"|"unknown"}`
  exposed at `/metrics`, derived at scrape time from the persisted
  PERP `FeeChargedV2` rows via
  `src/fees/perp_consumer.rs::classify_perp_fee_consumer` (raw
  addresses never become labels).
- New env var `OLD_PERP_ENGINE_ADDRESS` (default unset) wired into
  `ExecutionConfig::old_perp_engine_address` and consumed only by
  the metric path — never by execution / simulation / broadcast.
- Prometheus alert rule
  `PerpFeeChargedFromOldEngine` (`increase(...{consumer="old"}[5m]) > 0`)
  recorded as deployable YAML in `docs/ALERTING_SPEC.md`.
- `.env.example` flipped to NEW PerpEngine on `PERP_ENGINE_ADDRESS`,
  with `OLD_PERP_ENGINE_ADDRESS` provided as observability metadata.
- 8 classifier unit tests + 4 endpoint metric tests now pin the
  cardinality and exclusion contract.

See `docs/PERP_V2_FEE_METRICS_ALERTING_V2F_P.md` for the full
milestone record.

## Remaining gaps

- ~~The alert metric for `perp_fee_charged_from_old_engine` is spec-only;
  no Prometheus counter (`deopt_perp_fee_charged_v2_total{consumer}`)
  has been added yet. That belongs to a metrics-instrumentation
  milestone, not V2F-O.~~ Closed by V2F-P on 2026-05-30.
- `PERP_ENGINE_ADDRESS` in `.env` still references OLD. Env-hygiene
  fix is deferred (no operational impact while `PERP_NONCE_SYNC_*`
  paths remain disabled).
- Live admin UI smoke (visually verifying the PERP fee row in the
  dashboard) is deferred to the next frontend-running session.
- A `FeeRebatedV2` live PERP regression sample does not exist yet
  (V2F-LM emitted zero rebates by design); first PERP rebate will be
  the natural follow-up.

## Next recommended milestone

**V2F-P — Instrument `perp_fee_charged_from_old_engine` Metric**
(backend-only, no Solidity, no broadcast):

1. Add a counter `deopt_perp_fee_charged_v2_total{consumer}` where
   `consumer = "new"|"old"|"unknown"` based on the
   `decoded.consumer` field at indexing time.
2. Increment from the `FeeChargedV2` decoder (or, alternatively,
   from a post-persist hook) so the count survives restarts.
3. Add a regression test asserting that processing a PERP
   `FeeChargedV2` log emitted by NEW increments only the `"new"` arm.
4. Flip `PERP_ENGINE_ADDRESS` in `.env.example` to NEW and document
   the cutover step for operators.
5. Document the Prometheus rule against the new counter in
   `docs/ALERTING_SPEC.md`.

This locks in the observability primitive that today's alert spec
relies on, and converts V2F-O's spec-only entry into a deployable
rule without touching Solidity, broadcast, or DB state.
