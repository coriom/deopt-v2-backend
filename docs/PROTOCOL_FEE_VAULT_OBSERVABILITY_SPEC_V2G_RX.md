# V2G-RX — ProtocolFeeVault Observability Spec

## Status

- Milestone: **V2G-RX** — Prometheus metrics + Grafana panels +
  Alertmanager routing spec for the ProtocolFeeVault cutover.
  **Docs / spec only.** No backend code, no live config change.
- Date: 2026-06-01.

---

## 1. New Prometheus metrics

| Metric | Type | Labels | Source |
|---|---|---|---|
| `deopt_protocol_fee_vault_fee_balance_native` | gauge | `asset` | `vault.feeBalance(asset)` view call |
| `deopt_protocol_fee_vault_rebate_reserve_native` | gauge | `asset` | `vault.rebateReserve(asset)` |
| `deopt_protocol_fee_vault_gross_fees_collected_native` | counter | `asset` | `vault.grossFeesCollected(asset)` (monotonic) |
| `deopt_protocol_fee_vault_rebates_paid_native` | counter | `asset` | `vault.rebatesPaid(asset)` (monotonic) |
| `deopt_protocol_fee_vault_net_revenue_native` | gauge | `asset` | `vault.netRevenue(asset)` |
| `deopt_protocol_fee_vault_cv_internal_balance_native` | gauge | `asset` | `collateralVault.balances(vault, asset)` |
| `deopt_protocol_fee_vault_drift_native` | gauge | `asset` | computed: `feeBalance + rebateReserve − cv_internal_balance` (should be zero) |
| `deopt_protocol_fee_vault_rebates_paused` | gauge (0/1) | none | `vault.rebatesPaused()` |
| `deopt_protocol_fee_vault_bootstrapped` | gauge (0/1) | `asset` | `vault.bootstrapped(asset)` |
| `deopt_protocol_fee_vault_hook_failure_total` | counter | `kind` (`charge` / `rebate`) | derived from `tracing::warn!(target: "deopt.fees.vault.hook_failed", …)` — fires when the indexer observes a FM-V2 → vault hook revert |

### 1.1 Cardinality

- `asset` labels are limited to the configured settlement assets
  (mUSDC today, future asset additions per governance proposal).
- `kind` is a bounded enum.
- No per-trader labels — trader-level accounting belongs in
  `/admin/fees/onchain`, not Prometheus.

### 1.2 Scrape pattern

Backend exposes vault state via two new `/admin` endpoints
(documented but not implemented in V2G-RX scope):

| Endpoint | Returns |
|---|---|
| `GET /admin/fees/vault/snapshot?asset=<asset>` | full bucket snapshot for one asset — `{feeBalance, rebateReserve, grossFeesCollected, rebatesPaid, netRevenue, cvInternalBalance, drift, bootstrapped}` |
| `GET /admin/fees/vault/observability` | aggregate snapshot — all configured assets + `rebatesPaused` + `revenueReceiver` |

Both endpoints are role-gated `viewer` per V2G-W2 route mapping.

The Prometheus exporter scrapes the aggregate endpoint at the
standard interval (15s default), reads the per-asset rows, emits
the metric series.

---

## 2. Alert specs

### 2.1 `ProtocolFeeVaultDrift` (critical)

```yaml
- alert: ProtocolFeeVaultDrift
  expr: abs(deopt_protocol_fee_vault_drift_native) > 0
  for: 2m
  labels:
    severity: critical
  annotations:
    summary: "ProtocolFeeVault accounting drift detected"
    description: |
      asset {{ $labels.asset }} has
      feeBalance + rebateReserve != CV.balances(vault, asset).
      V2G-R1 invariant 2 is violated.
      Pause rebates immediately (RB-PAUSE-REBATES) and investigate.
```

### 2.2 `ProtocolFeeVaultReserveShortfall` (high)

```yaml
- alert: ProtocolFeeVaultReserveShortfall
  expr: deopt_protocol_fee_vault_rebate_reserve_native
        < deopt_fees_manager_v2_rebate_budget_native
  for: 5m
  labels:
    severity: high
  annotations:
    summary: "Vault rebate reserve below FM-V2 budget"
    description: |
      asset {{ $labels.asset }} has reserve {{ $value }} less than
      the FM-V2 rebateBudget. Future rebate trades will revert at
      the vault hook (InsufficientRebateReserve). Refill via
      vault.allocateToRebateReserve(asset, amount) or wait for
      organic fee inflow to top up.
```

### 2.3 `ProtocolFeeVaultRebatesPaused` (critical)

```yaml
- alert: ProtocolFeeVaultRebatesPaused
  expr: deopt_protocol_fee_vault_rebates_paused == 1
  for: 0m
  labels:
    severity: critical
  annotations:
    summary: "Vault rebate hook is paused"
    description: |
      Every FM-V2 rebate consumeFees call will revert at the
      vault hook (RebatesPausedError). Operator intervention required.
```

### 2.4 `ProtocolFeeVaultHookFailure` (critical)

```yaml
- alert: ProtocolFeeVaultHookFailure
  expr: increase(deopt_protocol_fee_vault_hook_failure_total[5m]) > 0
  for: 0m
  labels:
    severity: critical
  annotations:
    summary: "Vault hook reverted in the last 5 minutes"
    description: |
      A FeeChargedV2 or FeeRebatedV2 event was observed for which
      the corresponding vault hook call (onFeeCharged /
      onRebatePaid) reverted. This implies a FM-V2 consumeFees
      revert too — affected trade(s) failed atomically.
      Inspect via /admin/fees/onchain?tx_hash=<failing tx>.
```

### 2.5 `ProtocolFeeVaultBootstrapMissing` (warn)

```yaml
- alert: ProtocolFeeVaultBootstrapMissing
  expr: deopt_protocol_fee_vault_bootstrapped == 0
  for: 30m
  labels:
    severity: warn
  annotations:
    summary: "Vault is wired but not bootstrapped for {{ $labels.asset }}"
    description: |
      FM-V2 is routing fees to the vault but vault.bootstrapped({{ $labels.asset }})
      is still false. Bootstrap must run within the cutover
      window so per-asset counters reflect history correctly.
```

---

## 3. Grafana dashboard additions

Add panels to the existing `deopt-v2g-g-v2-fees` dashboard (folder
`DeOpt`, uid `deopt-v2g-g-v2-fees`):

| Panel | Type | Query |
|---|---|---|
| **Vault: gross vs net revenue** | timeseries | overlay of `deopt_protocol_fee_vault_gross_fees_collected_native{asset=…}` and `deopt_protocol_fee_vault_net_revenue_native{asset=…}` |
| **Vault: fee balance + rebate reserve (stacked)** | stacked timeseries | `deopt_protocol_fee_vault_fee_balance_native` and `deopt_protocol_fee_vault_rebate_reserve_native` per asset |
| **Vault: CV internal balance vs sum-of-buckets** | timeseries | overlay of `deopt_protocol_fee_vault_cv_internal_balance_native` and `deopt_protocol_fee_vault_fee_balance_native + deopt_protocol_fee_vault_rebate_reserve_native`. Should be identical lines; visual drift = bug. |
| **Vault: drift gauge** | stat | `deopt_protocol_fee_vault_drift_native` (red on non-zero) |
| **Vault: rebates paused** | stat (0/1) | `deopt_protocol_fee_vault_rebates_paused` |
| **Vault: hook failures (5m rate)** | stat | `increase(deopt_protocol_fee_vault_hook_failure_total[5m])` |
| **Vault: reserve vs FM-V2 budget** | overlay | `deopt_protocol_fee_vault_rebate_reserve_native` and `deopt_fees_manager_v2_rebate_budget_native` per asset |

---

## 4. Alertmanager routing

Add to `docs/monitoring/alertmanager/v2_fee_routing.example.yml`:

```yaml
routes:
  - matchers:
      - alertname=~"ProtocolFeeVault.*"
    receiver: "vault_ops"
    continue: false
    group_wait: 10s
    group_interval: 5m
    repeat_interval: 1h

receivers:
  - name: "vault_ops"
    webhook_configs:
      - url: "http://webhook-sink:9095/vault-ops"
        send_resolved: true
```

The `vault_ops` receiver should be wired to a dedicated
incident-response channel (PagerDuty / Slack / similar) on
the target host.

---

## 5. Backend implementation status

| Item | Status |
|---|---|
| `/admin/fees/vault/snapshot` endpoint | **not implemented** — spec only; design slot in `src/api/routes.rs` |
| `/admin/fees/vault/observability` endpoint | **not implemented** — spec only |
| Prometheus exporter for the new metrics | **not implemented** — depends on the two admin endpoints |
| `deopt.fees.vault.hook_failed` tracing event | **not implemented** — indexer-side derivation; needs an indexer hook on FM-V2 hook-reverted txs |
| Alert rules YAML | **draft only** — see §2 |
| Grafana dashboard JSON updates | **draft only** — see §3 |
| Alertmanager routing update | **draft only** — see §4 |

All of the above land at the V2G-RX backend impl milestone
(separate from the Solidity hook + CV extension shipped in
V2G-RX-Solidity offline). The backend impl requires a backend
restart to pick up the new endpoints; that restart is the
operator's next maintenance window.

---

## 6. Acceptance criteria for the observability spec close

- [x] Metric surface enumerated (§1).
- [x] 5 alert specs drafted (§2).
- [x] 7 Grafana panel specs drafted (§3).
- [x] Alertmanager routing drafted (§4).
- [x] Implementation gap explicitly listed (§5).
- [ ] Backend endpoints implemented (V2G-RX-backend milestone).
- [ ] Prometheus rules deployed (V2G-RX-monitoring deploy).
- [ ] Grafana panels deployed (V2G-RX-monitoring deploy).

---

## 7. Cross-links

- V2G-RX cutover runbook: `deopt-v2-sol/docs/PROTOCOL_FEE_VAULT_CUTOVER_RUNBOOK_V2G_RX.md`
- V2G-RX integration runbook: `PROTOCOL_FEE_VAULT_INTEGRATION_RUNBOOK_V2G_RX.md`
- V2G-G observability foundation: `V2_FEE_PRODUCTION_OBSERVABILITY_V2G_G.md`
- V2G-K alerts runbook: `RUNBOOK_PERP_V2_FEE_ALERTS.md`
- V2G-Y emergency runbooks: `GOVERNANCE_ADMIN_SAFETY_MATRIX_V2G_Y.md` §3
