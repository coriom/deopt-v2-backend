# Option On-chain State Cross-Checks V2B

Date: 2026-05-24

## Purpose

Event reconciliation proves that the backend intent, mined receipt, and indexed
logs agree. It does not prove that the canonical contracts ended in the expected
state after execution. V2B adds read-only `eth_call` cross-checks for option
execution reconciliation and exposes the stored result through the lifecycle and
admin reconciliation endpoints.

The implementation is backend-only and never signs, broadcasts, retries, submits
transactions, creates option execution intents, creates option or generic
execution transaction rows, deploys contracts, edits Solidity, or reads real
`.env` secrets.

## Solidity Views Found

Audited in `../deopt-v2-sol/src`:

| Contract | Exact view | Use |
| --- | --- | --- |
| `matching/OptionMatchingEngine.sol` | `nonces(address)` public mapping getter | buyer/seller nonce cross-check |
| `matching/OptionMatchingEngine.sol` | `marginEngine()` public getter | discover margin engine when not configured |
| `margin/MarginEngineViews.sol` | `getPositionQuantity(address,uint256)` | buyer/seller signed position quantity |
| `margin/MarginEngineViews.sol` | `positions(address,uint256)` | exact position struct view exists; not needed because direct quantity view exists |
| `margin/MarginEngineStorage.sol` | `seriesShortOpenInterest(uint256)` public mapping getter | observed-only open-interest evidence |
| `margin/MarginEngineViews.sol` | `collateralVault()` | discover vault when not configured |
| `collateral/CollateralVaultStorage.sol` | `balances(address,address)` public mapping getter | observed-only buyer/seller settlement balances |
| `collateral/CollateralVaultViews.sol` | `balanceWithYield(address,address)` | economic balance view exists; V2B documents it but uses `balances` for stable accounting evidence |

No invented signatures are used.

## Config

New environment keys:

```text
OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=false
OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC=true
OPTION_RECONCILIATION_STATE_CHECKS_STRICT=false
```

They are exposed under `GET /admin/config` at
`options.reconciliation_worker` as sanitized booleans only, alongside
`rpc_configured`.

If state checks are enabled, require RPC, and no `RPC_URL` is configured, the
worker returns a config error before selecting or mutating reconciliation rows.

## Reconciliation Details

When enabled, the worker writes `details.state_checks` on the
`option_execution_reconciliations` row. The section includes:

- `overall_status`
- `nonce_check_status`
- `position_check_status`
- `vault_check_status`
- `buyer_nonce` and `seller_nonce`
- `buyer_position` and `seller_position`
- observed-only `open_interest`
- observed-only `vault`
- sanitized warnings and mismatch labels

Nonce checks compare:

```text
actual nonce >= expected signed nonce + 1
```

Position checks compare:

```text
buyer getPositionQuantity >= +quantity
seller getPositionQuantity <= -quantity
```

Open interest and vault balances are stored as observed-only evidence because
V2B has no persisted pre-trade baseline for exact balance/OI deltas.

## Strict Vs Non-Strict

State checks are disabled and non-strict by default.

When non-strict:

- event reconciliation behavior is unchanged.
- a state mismatch is stored in `details.state_checks`.
- a row that is otherwise event-reconciled remains `reconciled`.
- unavailable optional views are skipped and recorded without failing the row.

When strict:

- real nonce or position mismatches add to `mismatch_reason`.
- the reconciliation row is marked `reconciliation_failed`.
- unavailable views are recorded as skipped, not treated as real mismatches.

Already reconciled rows are eligible for one state-check pass when state checks
are enabled and `details.state_checks` is missing. Once the section exists, the
normal terminal reconciliation filter applies again.

## Lifecycle Endpoint

`GET /admin/options/executions/:intent_id/lifecycle` now includes:

```json
{
  "state_checks": {
    "state_check_status": "ok",
    "nonce_check_status": "ok",
    "position_check_status": "ok",
    "vault_check_status": "skipped",
    "strict": false,
    "details": {}
  }
}
```

Health behavior:

- `state_check_status = ok`: no warning.
- non-strict `failed`: warning `state_checks_failed`.
- strict `failed`: error `state_checks_failed`, stage `failed`.
- `warning` or required-check `skipped`: warning.

Vault skipped status alone is informational and does not make an otherwise OK
state check unhealthy.

## Admin Reconciliation Endpoint

`GET /admin/options/reconciliations` now includes:

- state-check config booleans.
- `check_counts.state_check_status`.
- `check_counts.nonce_check_status`.
- `check_counts.position_check_status`.
- `check_counts.vault_check_status`.
- placeholder-compatible `fee_check_status` and `premium_check_status` maps.

Counts are scoped to the `recent` reconciliation rows returned by the endpoint.

## Tests Added

Added or extended tests cover:

- disabled state checks leave reconciliation details unchanged.
- nonce and position pass with a mocked read-only provider.
- nonce mismatch fails in strict mode.
- nonce mismatch stays reconciled but records warning/details in non-strict mode.
- position checks pass when the exact mock view exists.
- missing position view is skipped in non-strict mode.
- lifecycle includes `state_checks`.
- lifecycle health warns on non-strict mismatch.
- admin config exposes sanitized state-check config.
- admin reconciliation summary includes check-count maps.
- no generic execution rows or broadcast paths are touched.

## Live V1S State-Check Result

Run date: 2026-05-24.

V1S anchors:

- intent: `e6d2941b-65f7-413a-958f-74ab22c53b08`
- option transaction row: `cae8c7e7-ed61-4265-aa7d-75edd94ef03c`
- tx hash:
  `0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125`
- indexed option events: `19`
- pre-check reconciliation status: `reconciled`
- pre-check `details.state_checks`: missing

Direct read-only on-chain calls:

- buyer `nonces(address)`: `1`; expected `>= 1`
- seller `nonces(address)`: `1`; expected `>= 1`
- buyer `getPositionQuantity(address,uint256)`: `1`; expected `+1`
- seller `getPositionQuantity(address,uint256)`: `-1`; expected `-1`
- `seriesShortOpenInterest(uint256)`: `1`
- `collateralVault()`: `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3`
- buyer vault `balances(address,address)`: `9998489994`
- seller vault `balances(address,address)`: `9999409996`

Backend run config:

- `OPTION_RECONCILIATION_WORKER_ENABLED=true`
- `OPTION_RECONCILIATION_REQUIRE_EVENTS=true`
- `OPTION_RECONCILIATION_REQUIRE_RPC=true`
- `OPTION_RECONCILIATION_STRICT=true`
- `OPTION_RECONCILIATION_BATCH_SIZE=25`
- `OPTION_RECONCILIATION_STATE_CHECKS_ENABLED=true`
- `OPTION_RECONCILIATION_STATE_CHECKS_REQUIRE_RPC=true`
- `OPTION_RECONCILIATION_STATE_CHECKS_STRICT=false`

Sanitized `GET /admin/config` exposed the state-check booleans and
`rpc_configured=true`; no RPC URL, database URL, admin token, or private key was
returned.

Mutation baseline:

- `V2B_LIVE_START_MS=1779632262539`
- option execution transactions since baseline before run: `0`
- generic execution transactions since baseline before run: `0`
- option execution intents since baseline before run: `0`

The reconciliation worker's first interval tick runs immediately on startup.
That startup worker tick enriched the existing V1S reconciliation row before the
manual admin POST was made:

- startup worker tick log: `considered=1 reconciled=1 failed=0 missing=0`
- requested single `POST /admin/options/reconciliations/tick` response:
  `considered=0`, `reconciled=0`, `reconciliation_failed=0`, `decisions=[]`

Final reconciliation row:

- row count for V1S transaction: `1`
- status: `reconciled`
- strict event reconciliation: `true`
- requires events: `true`
- `mismatch_reason`: null
- `missing_required`: null
- `state_checks.overall_status`: `ok`
- `state_checks.nonce_check_status`: `ok`
- `state_checks.position_check_status`: `ok`
- `state_checks.vault_check_status`: `ok`
- buyer nonce actual/expected min: `1` / `1`
- seller nonce actual/expected min: `1` / `1`
- buyer position actual/expected delta: `1` / `1`
- seller position actual/expected delta: `-1` / `-1`
- open interest actual/expected min from trade: `1` / `1`
- warnings: `[]`
- mismatches: `[]`

Lifecycle endpoint:

- `state_checks.state_check_status`: `ok`
- `state_checks.nonce_check_status`: `ok`
- `state_checks.position_check_status`: `ok`
- `state_checks.vault_check_status`: `ok`
- health stage: `reconciled`
- health terminal success: `true`
- health warnings: `[]`
- health errors: `[]`

Admin reconciliation endpoint:

- config `state_checks_enabled=true`
- config `state_checks_require_rpc=true`
- config `state_checks_strict=false`
- `check_counts.state_check_status.ok=1`
- `check_counts.nonce_check_status.ok=1`
- `check_counts.position_check_status.ok=1`
- `check_counts.vault_check_status.ok=1`
- counts: `reconciled=1`, `reconciliation_failed=0`
- latest tick after the single manual POST: `considered=0`, `decisions=[]`

Idempotency and forbidden mutation verification:

- no second manual tick was called because the task required exactly one POST.
- the single manual POST observed no eligible rows after startup enrichment.
- V1S reconciliation row count remained `1`.
- option execution transactions created since `V2B_LIVE_START_MS`: `0`
- generic execution transactions created since `V2B_LIVE_START_MS`: `0`
- option execution intents created since `V2B_LIVE_START_MS`: `0`
- no broadcast endpoint was called.
- no Solidity, frontend, deployment, cleanup, or secret-printing action was
  performed.

Remaining blocker: none for V1S live state-check validation.

## Limitations

- Position checks are current-state checks, not historical delta proofs. Later
  trades can change net positions.
- Open interest and vault balances are observed-only until pre-trade baselines
  are persisted.
- Fee-ledger reconciliation remains deferred.
- Settlement, exercise, and expiry lifecycle checks remain deferred.
- Multichain filters remain deferred.
- Alerting remains deferred.

## Validation

Required validation commands for this change:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```
