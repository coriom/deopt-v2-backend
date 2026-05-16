# DeOpt E2E Harness V1A

`run_e2e.py` is a standard-library Python harness for reproducible local/runtime checks against the existing backend. It orchestrates HTTP calls, safe backend startup, and PostgreSQL read checks. It does not modify Solidity, deploy contracts, enable real broadcast, move funds, expose private keys, or edit `.env`.

## Usage

```sh
python3 scripts/e2e/run_e2e.py --flow admin
python3 scripts/e2e/run_e2e.py --flow fees-options --start-backend
python3 scripts/e2e/run_e2e.py --flow fees-perps --no-start-backend
python3 scripts/e2e/run_e2e.py --flow option-rfq --json-out /tmp/deopt-e2e-option-rfq.json
python3 scripts/e2e/run_e2e.py --flow all-safe --start-backend
```

Common flags:

```text
--backend-url http://127.0.0.1:8080
--database-url postgres://deopt:deopt@127.0.0.1:5432/deopt_v2_backend
--admin-token local-admin-token-runtime-test
--start-backend
--no-start-backend
--timeout-sec 120
--json-out /tmp/deopt-e2e-report.json
--cleanup
--verbose
```

`--no-start-backend` is the default. Use it when a backend is already running with the required feature flags. `--start-backend` runs `cargo run --bin deopt-v2-backend` with process-only overrides and stops that process at the end.

## Safety Defaults

When the harness starts the backend, it forces:

```text
EXECUTION_ENABLED=false
EXECUTOR_DRY_RUN=true
EXECUTOR_REAL_BROADCAST_ENABLED=false
EXECUTOR_PRIVATE_KEY=
SIMULATION_ENABLED=false
INDEXER_ENABLED=false
PERP_NONCE_SYNC_ENABLED=false
MM_GATEWAY_ENABLED=false
RFQ_QUOTE_SIGNATURE_MODE=disabled
OPTION_RFQ_QUOTE_SIGNATURE_MODE=disabled
SIGNATURE_VERIFICATION_MODE=disabled
```

It also enables only the read/write surfaces needed for safe local verification: persistence, admin reads, options, option RFQ, and fee ledgers. For `fees-perps` and `all-safe`, confirmation is enabled so an already submitted and confirmed transaction can be rechecked, but broadcast remains disabled.

The harness never calls `/executor/broadcast`. It does not create execution transactions. It redacts the admin token and database URL from errors, and backend logs are written to a temporary file path reported as an artifact.

## Flows

### `admin`

Checks:

- `GET /health`
- token-protected `GET /admin/status`
- sanitized `GET /admin/config`
- `GET /admin/db`
- `GET /admin/fees/summary`
- wrong-token rejection

The config check verifies that the raw admin token, raw database URL, and raw URL string values are absent from `/admin/config`.

### `fees-options`

Requires options, option RFQ, fees, persistence, and `psql`.

Checks:

- creates a manual active option series
- creates an off-chain option orderbook fill
- verifies two `fee_events` rows with `source_type=option_order_fill`
- creates and accepts an option RFQ quote
- verifies two `fee_events` rows with `source_type=option_rfq_fill`
- verifies `volume_buckets.market_type=option`
- verifies admin fee endpoints
- verifies zero new `execution_transactions`

If `MM_PERMISSIONS_ENABLED=true`, the harness temporarily seeds the maker account with option quote/order permissions and restores the prior row after the run. If option-series scopes already exist for the account, it inserts only a temporary matching scope and deletes that scope during cleanup.

### `fees-perps`

Requires persistence, `psql`, fees, confirmation, and a real historical confirmed/indexed/reconciled perp trade.

Checks:

- discovers a candidate by joining `execution_transactions`, `execution_reconciliations`, `indexed_perp_trades`, and `execution_intents`
- triggers `POST /executor/confirm/:intent_id`
- verifies two `fee_events` rows with `source_type=perp_trade`
- verifies `volume_buckets.market_type=perp`
- reruns the same trigger once
- verifies no duplicate `(source_type, source_id, payer, recipient)` fee rows
- verifies volume buckets did not change on the second trigger
- verifies admin fee endpoints
- verifies zero new `execution_transactions` and `realBroadcastEnabled=false`

If no candidate exists, the flow reports a skipped check with the reason and does not fake a trade.

### `option-rfq`

Requires options, option RFQ, persistence, and `psql`.

Checks:

- creates a manual active option series
- creates an option RFQ
- submits an unsigned quote in disabled signature mode
- accepts the quote
- verifies the `option_rfq_fills` row
- verifies zero new `execution_transactions`

If `MM_PERMISSIONS_ENABLED=true`, temporary maker permissions are seeded and restored as in `fees-options`.

### `mm-auth`

V1A includes a skipped placeholder. The existing `mm_wt_smoke auth` path requires live WebTransport cert/key setup and an `MM_PRIVATE_KEY`. This harness does not generate or expose private keys, so the runnable wrapper is deferred.

### `all-safe`

Runs:

- `admin`
- `fees-options`
- `fees-perps`
- `option-rfq`

`mm-auth` is excluded because it requires live WebTransport and private-key-backed wallet auth.

## Report Format

The harness prints a concise human summary to stderr and JSON to stdout. If `--json-out` is supplied, the same JSON is also written to that path.

Shape:

```json
{
  "ok": true,
  "flow": "admin",
  "started_at_ms": 1770000000000,
  "finished_at_ms": 1770000001234,
  "checks": [
    {
      "name": "health",
      "ok": true,
      "details": {}
    }
  ],
  "artifacts": {},
  "errors": []
}
```

Skipped checks are encoded with `"skipped": true` and remain non-failing when the skip is an expected V1A condition, such as no available confirmed perp candidate.

## DB Behavior

The harness calls `psql` through `subprocess` without `shell=True`. SQL values derived from runtime artifacts are quoted before interpolation. DB checks are read-only except for temporary MM permission scaffolding when an existing backend has MM permission enforcement enabled; those rows are restored automatically. Fee ledger evidence is retained.
