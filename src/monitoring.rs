use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::fees::FeeEvent;
use crate::options::{OptionFillFilter, OptionOrderFilter, OptionSeriesFilter};
use crate::types::now_ms;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessResponse {
    pub ready: bool,
    pub service: &'static str,
    pub checks: Vec<ReadinessCheck>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessCheck {
    pub name: &'static str,
    pub ok: bool,
    pub status: &'static str,
}

pub async fn readiness(state: &AppState) -> ReadinessResponse {
    let mut checks = vec![
        ReadinessCheck {
            name: "process",
            ok: true,
            status: "up",
        },
        ReadinessCheck {
            name: "config",
            ok: true,
            status: "valid",
        },
    ];

    if state.persistence_enabled {
        match state.repository.clone() {
            Some(repository) => match repository.admin_ping().await {
                Ok(()) => checks.push(ReadinessCheck {
                    name: "database",
                    ok: true,
                    status: "up",
                }),
                Err(_) => checks.push(ReadinessCheck {
                    name: "database",
                    ok: false,
                    status: "db_ping_failed",
                }),
            },
            None => checks.push(ReadinessCheck {
                name: "database",
                ok: false,
                status: "repository_unavailable",
            }),
        }
    } else {
        checks.push(ReadinessCheck {
            name: "database",
            ok: true,
            status: "persistence_disabled",
        });
    }

    ReadinessResponse {
        ready: checks.iter().all(|check| check.ok),
        service: "deopt-v2-backend",
        checks,
    }
}

pub async fn render_metrics(state: &AppState) -> Result<String> {
    let mut metrics = MetricsText::new();

    metrics.gauge("deopt_backend_up", "Backend process is up.", 1);
    metrics.gauge(
        "deopt_admin_enabled",
        "Admin API enabled.",
        bool_value(state.admin_config.enabled),
    );
    metrics.gauge(
        "deopt_metrics_enabled",
        "Metrics endpoint enabled.",
        bool_value(state.metrics_config.enabled),
    );
    metrics.gauge(
        "deopt_metrics_require_admin_token",
        "Metrics endpoint requires the admin token.",
        bool_value(state.metrics_config.require_admin_token),
    );
    metrics.gauge(
        "deopt_persistence_enabled",
        "PostgreSQL persistence enabled.",
        bool_value(state.persistence_enabled),
    );
    metrics.gauge(
        "deopt_execution_enabled",
        "Execution worker enabled.",
        bool_value(state.execution_config.execution_enabled),
    );
    metrics.gauge(
        "deopt_real_broadcast_enabled",
        "Real transaction broadcast enabled.",
        bool_value(state.execution_config.real_broadcast_enabled),
    );
    metrics.gauge(
        "deopt_simulation_enabled",
        "Execution simulation enabled.",
        bool_value(state.execution_config.simulation_enabled),
    );
    metrics.gauge(
        "deopt_indexer_enabled",
        "Indexer enabled.",
        bool_value(state.indexer_config.enabled),
    );
    metrics.gauge(
        "deopt_reconciliation_enabled",
        "Reconciliation worker enabled.",
        bool_value(state.reconciliation_config.enabled),
    );
    metrics.gauge(
        "deopt_confirmation_enabled",
        "Confirmation worker enabled.",
        bool_value(state.confirmation_config.enabled),
    );
    metrics.gauge(
        "deopt_perp_nonce_sync_enabled",
        "Perp on-chain nonce sync enabled.",
        bool_value(state.perp_nonce_sync_config.enabled),
    );
    metrics.gauge(
        "deopt_option_nonce_sync_enabled",
        "Option on-chain nonce sync enabled.",
        bool_value(state.option_nonce_sync_config.enabled),
    );
    metrics.gauge(
        "deopt_mm_gateway_enabled",
        "Market Maker Gateway enabled.",
        bool_value(state.mm_gateway_config.enabled),
    );
    metrics.gauge(
        "deopt_rfq_enabled",
        "Perp RFQ service enabled.",
        bool_value(state.rfq_config.enabled),
    );
    metrics.gauge(
        "deopt_options_enabled",
        "Options service enabled.",
        bool_value(state.options_config.enabled),
    );
    metrics.gauge(
        "deopt_option_rfq_enabled",
        "Option RFQ service enabled.",
        bool_value(state.options_config.rfq_enabled),
    );
    metrics.gauge(
        "deopt_fees_enabled",
        "Fee ledger enabled.",
        bool_value(state.fees_config.enabled),
    );
    metrics.gauge(
        "deopt_rebates_enabled",
        "Fee rebate accrual enabled.",
        bool_value(state.fees_config.rebates_enabled),
    );

    append_database_metrics(state, &mut metrics).await?;
    append_execution_metrics(state, &mut metrics).await?;
    append_rfq_metrics(state, &mut metrics).await?;
    append_option_metrics(state, &mut metrics).await?;
    append_fee_metrics(state, &mut metrics).await?;
    append_mm_metrics(state, &mut metrics)?;

    Ok(metrics.finish())
}

async fn append_database_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let Some(repository) = state.repository.clone() else {
        metrics.gauge("deopt_db_up", "Database ping status.", 0);
        metrics.gauge(
            "deopt_db_migrations_installed",
            "Installed SQL migration count.",
            0,
        );
        return Ok(());
    };

    if repository.admin_ping().await.is_err() {
        metrics.gauge("deopt_db_up", "Database ping status.", 0);
        metrics.gauge(
            "deopt_db_migrations_installed",
            "Installed SQL migration count.",
            0,
        );
        return Ok(());
    }

    let migration_status = repository.admin_migration_status().await?;
    let installed_count = migration_status
        .get("installed_count")
        .and_then(|value| value.as_u64())
        .unwrap_or_default();
    metrics.gauge("deopt_db_up", "Database ping status.", 1);
    metrics.gauge(
        "deopt_db_migrations_installed",
        "Installed SQL migration count.",
        installed_count,
    );
    Ok(())
}

async fn append_execution_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let intent_status_counts;
    let simulation_status_counts;
    let transaction_status_counts;
    let confirmed_transactions;

    if let Some(repository) = metrics_repository(state).await {
        intent_status_counts = repository
            .admin_count_by_column("execution_intents", "status")
            .await?;
        simulation_status_counts = repository
            .admin_count_by_column("execution_simulations", "status")
            .await?;
        transaction_status_counts = repository
            .admin_count_by_column("execution_transactions", "status")
            .await?;
        confirmed_transactions = repository
            .admin_count_where(
                "execution_transactions",
                "confirmation_status = 'confirmed'",
            )
            .await?;
    } else {
        let mut counts = BTreeMap::new();
        for intent in state
            .engine
            .lock()
            .map_err(|_| BackendError::Config("engine lock poisoned".to_string()))?
            .execution_intents()
        {
            bump_count(&mut counts, execution_status_key(intent.status));
        }
        intent_status_counts = counts;
        simulation_status_counts = BTreeMap::new();
        transaction_status_counts = BTreeMap::new();
        confirmed_transactions = 0;
    }

    metrics.labeled_gauges(
        "deopt_execution_intents_total",
        "Execution intents by status.",
        &[("status", &intent_status_counts)],
    );
    metrics.labeled_gauges(
        "deopt_execution_simulations_total",
        "Execution simulations by status.",
        &[("status", &simulation_status_counts)],
    );
    metrics.labeled_gauges(
        "deopt_execution_transactions_total",
        "Execution transactions by status.",
        &[("status", &transaction_status_counts)],
    );
    metrics.gauge(
        "deopt_execution_confirmed_total",
        "Execution transactions confirmed.",
        confirmed_transactions,
    );
    Ok(())
}

async fn append_rfq_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let rfq_status_counts;
    let quote_status_counts;
    if let Some(repository) = metrics_repository(state).await {
        rfq_status_counts = repository.admin_count_by_column("rfqs", "status").await?;
        quote_status_counts = repository
            .admin_count_by_column("rfq_quotes", "status")
            .await?;
    } else {
        let now = now_ms();
        let store = state
            .rfq_store
            .lock()
            .map_err(|_| BackendError::Config("rfq store lock poisoned".to_string()))?;
        let rfqs = store.list_rfqs();
        let mut rfq_counts = BTreeMap::new();
        let mut quote_counts = BTreeMap::new();
        for rfq in &rfqs {
            bump_count(&mut rfq_counts, rfq.effective_status(now).as_str());
            for quote in store.list_quotes(rfq.rfq_id) {
                bump_count(&mut quote_counts, quote.effective_status(now).as_str());
            }
        }
        rfq_status_counts = rfq_counts;
        quote_status_counts = quote_counts;
    }

    metrics.labeled_gauges(
        "deopt_rfqs_total",
        "Perp RFQs by status.",
        &[("status", &rfq_status_counts)],
    );
    metrics.labeled_gauges(
        "deopt_rfq_quotes_total",
        "Perp RFQ quotes by status.",
        &[("status", &quote_status_counts)],
    );
    Ok(())
}

async fn append_option_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let series_status_counts;
    let order_status_counts;
    let option_fills_count;
    let option_rfq_status_counts;
    let option_rfq_quote_status_counts;

    if let Some(repository) = metrics_repository(state).await {
        series_status_counts = repository
            .admin_count_by_column("option_series", "status")
            .await?;
        order_status_counts = repository
            .admin_count_by_column("option_orders", "status")
            .await?;
        option_fills_count = repository.admin_count_where("option_fills", "TRUE").await?;
        option_rfq_status_counts = repository
            .admin_count_by_column("option_rfqs", "status")
            .await?;
        option_rfq_quote_status_counts = repository
            .admin_count_by_column("option_rfq_quotes", "status")
            .await?;
    } else {
        let now = now_ms();
        let now_sec = u64::try_from(now / 1000).unwrap_or_default();
        let store = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
        let series = store.list_series(&OptionSeriesFilter::default(), now_sec);
        let orders = store.list_orders(&OptionOrderFilter::default());
        let fills = store.list_fills(&OptionFillFilter::default());
        let option_rfqs = store.list_option_rfqs();

        let mut series_counts = BTreeMap::new();
        let mut order_counts = BTreeMap::new();
        let mut option_rfq_counts = BTreeMap::new();
        let mut option_quote_counts = BTreeMap::new();
        for item in &series {
            bump_count(&mut series_counts, item.effective_status(now_sec).as_str());
        }
        for order in &orders {
            bump_count(&mut order_counts, order.status.as_str());
        }
        for rfq in &option_rfqs {
            bump_count(&mut option_rfq_counts, rfq.effective_status(now).as_str());
            for quote in store.list_option_rfq_quotes(rfq.option_rfq_id) {
                bump_count(
                    &mut option_quote_counts,
                    quote.effective_status(now).as_str(),
                );
            }
        }
        series_status_counts = series_counts;
        order_status_counts = order_counts;
        option_fills_count = fills.len() as u64;
        option_rfq_status_counts = option_rfq_counts;
        option_rfq_quote_status_counts = option_quote_counts;
    }

    metrics.labeled_gauges(
        "deopt_option_series_total",
        "Option series by status.",
        &[("status", &series_status_counts)],
    );
    metrics.labeled_gauges(
        "deopt_option_orders_total",
        "Option orders by status.",
        &[("status", &order_status_counts)],
    );
    metrics.gauge(
        "deopt_option_fills_total",
        "Option orderbook fills.",
        option_fills_count,
    );
    metrics.labeled_gauges(
        "deopt_option_rfqs_total",
        "Option RFQs by status.",
        &[("status", &option_rfq_status_counts)],
    );
    metrics.labeled_gauges(
        "deopt_option_rfq_quotes_total",
        "Option RFQ quotes by status.",
        &[("status", &option_rfq_quote_status_counts)],
    );
    Ok(())
}

async fn append_fee_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let fee_counts;
    let rebate_status_counts;
    let perp_charged_raw_counts: BTreeMap<String, u64>;
    let perp_rebated_raw_counts: BTreeMap<String, u64>;
    let option_charged_raw_counts: BTreeMap<String, u64>;
    let option_rebated_raw_counts: BTreeMap<String, u64>;
    let rebate_budget_by_asset: BTreeMap<String, u64>;
    if let Some(repository) = metrics_repository(state).await {
        fee_counts = repository.admin_fee_event_label_counts().await?;
        rebate_status_counts = repository
            .admin_count_by_column("rebate_accruals", "status")
            .await?;
        perp_charged_raw_counts = repository.admin_perp_fee_v2_consumer_counts().await?;
        perp_rebated_raw_counts = repository
            .admin_perp_fee_v2_rebated_consumer_counts()
            .await?;
        option_charged_raw_counts = repository.admin_option_fee_v2_consumer_counts().await?;
        option_rebated_raw_counts = repository
            .admin_option_fee_v2_rebated_consumer_counts()
            .await?;
        rebate_budget_by_asset = repository
            .admin_fees_manager_v2_rebate_budget_by_asset()
            .await?;
    } else {
        let store = state
            .fees_store
            .lock()
            .map_err(|_| BackendError::Config("fees store lock poisoned".to_string()))?;
        let mut counts = BTreeMap::new();
        for event in store.list_fee_events(usize::MAX) {
            bump_fee_event_count(&mut counts, &event);
        }
        let mut rebate_counts = BTreeMap::new();
        for rebate in store.list_rebate_accruals(None) {
            bump_count(&mut rebate_counts, rebate.status.as_str());
        }
        fee_counts = counts;
        rebate_status_counts = rebate_counts;
        drop(store);
        let options_store = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?;
        perp_charged_raw_counts = options_store.perp_fee_v2_consumer_counts();
        perp_rebated_raw_counts = options_store.perp_fee_v2_rebated_consumer_counts();
        option_charged_raw_counts = options_store.option_fee_v2_consumer_counts();
        option_rebated_raw_counts = options_store.option_fee_v2_rebated_consumer_counts();
        rebate_budget_by_asset = options_store.fees_manager_v2_rebate_budget_by_asset();
    }

    metrics.fee_event_gauges(&fee_counts);
    metrics.labeled_gauges(
        "deopt_rebate_accruals_total",
        "Rebate accruals by status.",
        &[("status", &rebate_status_counts)],
    );
    append_perp_fee_v2_consumer_metric(
        state,
        metrics,
        "deopt_perp_fee_charged_v2_total",
        "PERP FeeChargedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither).",
        &perp_charged_raw_counts,
    );
    append_perp_fee_v2_consumer_metric(
        state,
        metrics,
        "deopt_perp_fee_rebated_v2_total",
        "PERP FeeRebatedV2 events bucketed by consumer engine (new=current, old=stranded, unknown=neither).",
        &perp_rebated_raw_counts,
    );
    append_option_fee_v2_consumer_metric(
        state,
        metrics,
        "deopt_option_fee_charged_v2_total",
        "OPTION FeeChargedV2 events bucketed by MarginEngine consumer (new=current, old=stranded, unknown=neither).",
        &option_charged_raw_counts,
    );
    append_option_fee_v2_consumer_metric(
        state,
        metrics,
        "deopt_option_fee_rebated_v2_total",
        "OPTION FeeRebatedV2 events bucketed by MarginEngine consumer (new=current, old=stranded, unknown=neither).",
        &option_rebated_raw_counts,
    );
    // V2G-F: derived FeesManagerV2 rebate budget per settlement asset.
    // Empty map => no event yet; we emit nothing (no zero baseline)
    // because the asset label is unknown until at least one RebateBudget
    // event has been indexed. The `FeesManagerV2RebateBudgetLow` alert
    // is keyed on a specific lowercased address (e.g. mUSDC).
    metrics.labeled_gauges(
        "deopt_fees_manager_v2_rebate_budget_native",
        "Derived FeesManagerV2 rebate budget per settlement asset (sum of indexed RebateBudgetFunded minus Spent and Withdrawn; clamped at 0). Labelled by the lowercased settlement-asset address.",
        &[("asset", &rebate_budget_by_asset)],
    );

    // V2G-R5-OBS-P0 — ProtocolFeeVault observability. Emits nothing
    // when the vault address is unset (the pre-deploy posture). Best
    // effort: any RPC failure means no rows for that asset are emitted
    // rather than a metrics-render error.
    append_protocol_fee_vault_metrics(state, metrics, &rebate_budget_by_asset).await;

    Ok(())
}

async fn append_protocol_fee_vault_metrics(
    state: &AppState,
    metrics: &mut MetricsText,
    rebate_budget_by_asset: &BTreeMap<String, u64>,
) {
    use crate::fees::vault_observability as vault_obs;

    let fallback_assets: Vec<String> = rebate_budget_by_asset.keys().cloned().collect();
    let cfg = vault_obs::build_config(
        state.execution_config.rpc_url.clone(),
        Some(
            state
                .option_event_indexer_config
                .collateral_vault_address
                .clone(),
        ),
        state
            .option_event_indexer_config
            .fees_manager_v2_address
            .clone(),
        fallback_assets,
    );
    if !cfg.is_configured() {
        // Emit the single global "configured" gauge so the operator
        // can tell from /metrics alone whether the vault env is set.
        metrics.gauge(
            "deopt_protocol_fee_vault_configured",
            "Set to 1 once PROTOCOL_FEE_VAULT_ADDRESS is configured. 0 means the V2G-R5 cutover has not happened yet.",
            0,
        );
        return;
    }
    let Ok(snapshot) = vault_obs::read_snapshot(&cfg, rebate_budget_by_asset).await else {
        metrics.gauge(
            "deopt_protocol_fee_vault_configured",
            "Set to 1 once PROTOCOL_FEE_VAULT_ADDRESS is configured. 0 means the V2G-R5 cutover has not happened yet.",
            1,
        );
        return;
    };
    metrics.gauge(
        "deopt_protocol_fee_vault_configured",
        "Set to 1 once PROTOCOL_FEE_VAULT_ADDRESS is configured. 0 means the V2G-R5 cutover has not happened yet.",
        1,
    );
    metrics.gauge(
        "deopt_protocol_fee_vault_rebates_paused",
        "Set to 1 when ProtocolFeeVault.rebatesPaused() is true.",
        if vault_obs::rebates_paused(&snapshot) {
            1
        } else {
            0
        },
    );
    metrics.gauge(
        "deopt_protocol_fee_vault_drift_present",
        "Set to 1 when at least one configured asset has non-zero feeBalance+rebateReserve vs CV.balances drift.",
        if vault_obs::any_drift_present(&snapshot) { 1 } else { 0 },
    );

    // Per-asset gauges. Each metric is described once via labeled_gauges
    // with a fresh BTreeMap so the help/type header is emitted exactly
    // once per metric name even when multiple assets are configured.
    let rows = vault_obs::metric_rows(&snapshot);
    let mut by_metric: BTreeMap<&'static str, BTreeMap<String, u64>> = BTreeMap::new();
    for r in &rows {
        // Saturating parse: U256 values that exceed u64 are clamped so
        // the metric remains numeric. Operators inspecting the JSON
        // endpoint get the full uint string.
        let value: u64 = r
            .value
            .parse::<u128>()
            .map(|v| v.min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        by_metric
            .entry(r.metric)
            .or_default()
            .entry(r.asset.clone())
            .and_modify(|v| *v = v.saturating_add(value))
            .or_insert(value);
    }
    for (metric, by_asset) in by_metric {
        let help: &'static str = vault_metric_help(metric);
        metrics.labeled_gauges(metric, help, &[("asset", &by_asset)]);
    }
}

fn vault_metric_help(metric: &str) -> &'static str {
    match metric {
        "deopt_protocol_fee_vault_fee_balance_native" => {
            "ProtocolFeeVault.feeBalance(asset). Spendable positive-fee balance per settlement asset."
        }
        "deopt_protocol_fee_vault_rebate_reserve_native" => {
            "ProtocolFeeVault.rebateReserve(asset). Spendable rebate-reserve balance per settlement asset."
        }
        "deopt_protocol_fee_vault_gross_fees_collected_native" => {
            "ProtocolFeeVault.grossFeesCollected(asset). Monotonic cumulative positive-fee inflow per settlement asset."
        }
        "deopt_protocol_fee_vault_rebates_paid_native" => {
            "ProtocolFeeVault.rebatesPaid(asset). Monotonic cumulative rebate outflow per settlement asset."
        }
        "deopt_protocol_fee_vault_net_revenue_native" => {
            "ProtocolFeeVault.netRevenue(asset). Cached grossFeesCollected − rebatesPaid per settlement asset."
        }
        "deopt_protocol_fee_vault_internal_collateral_vault_balance_native" => {
            "CollateralVault.balances(vault, asset). Internal-account funds the vault controls in the CV ledger."
        }
        "deopt_protocol_fee_vault_raw_erc20_balance_native" => {
            "IERC20(asset).balanceOf(vault). Raw ERC-20 dust the vault holds outside its CV ledger account; expected to be 0."
        }
        "deopt_protocol_fee_vault_drift_native" => {
            "abs(CV.balances(vault, asset) − feeBalance(asset) − rebateReserve(asset)). Invariant 2 demands 0; non-zero triggers ProtocolFeeVaultDrift."
        }
        "deopt_protocol_fee_vault_reserve_shortfall_native" => {
            "max(0, deopt_fees_manager_v2_rebate_budget_native − rebateReserve). Non-zero predicts future rebate trades will revert at the vault hook."
        }
        _ => "ProtocolFeeVault observability metric (V2G-R5-OBS-P0).",
    }
}

/// V2F-P / V2F-Q: collapse the raw `decoded.consumer` strings returned
/// by the repository / store into the three low-cardinality buckets
/// (`"new"`, `"old"`, `"unknown"`) and emit a gauge named `metric_name`.
///
/// Always emits all three label values (even when their count is zero)
/// so Prometheus alert rules of the shape
/// `increase(<metric>{consumer="old"}[5m]) > 0` have a stable time
/// series from the first scrape. Raw addresses are never promoted to a
/// label value.
///
/// Used for both the V2F-P PERP `FeeChargedV2` metric
/// (`deopt_perp_fee_charged_v2_total`) and the V2F-Q PERP
/// `FeeRebatedV2` metric (`deopt_perp_fee_rebated_v2_total`); the only
/// thing that differs between the two is which event family the
/// `raw_counts` map reflects.
fn append_perp_fee_v2_consumer_metric(
    state: &AppState,
    metrics: &mut MetricsText,
    metric_name: &'static str,
    metric_help: &'static str,
    raw_counts: &BTreeMap<String, u64>,
) {
    use crate::fees::perp_consumer::{
        classify_perp_fee_consumer, CONSUMER_NEW, CONSUMER_OLD, CONSUMER_UNKNOWN,
    };

    let new_addr = non_zero_address(&state.execution_config.perp_engine_address.0);
    let old_addr = state
        .execution_config
        .old_perp_engine_address
        .as_ref()
        .and_then(|addr| non_zero_address(&addr.0));

    let mut bucketed: BTreeMap<String, u64> = BTreeMap::new();
    bucketed.insert(CONSUMER_NEW.to_string(), 0);
    bucketed.insert(CONSUMER_OLD.to_string(), 0);
    bucketed.insert(CONSUMER_UNKNOWN.to_string(), 0);
    for (consumer, count) in raw_counts {
        let bucket = classify_perp_fee_consumer(consumer, new_addr, old_addr);
        let entry = bucketed.entry(bucket.to_string()).or_default();
        *entry = entry.saturating_add(*count);
    }

    metrics.labeled_gauges(metric_name, metric_help, &[("consumer", &bucketed)]);
}

/// V2G-F: OPTION analogue of [`append_perp_fee_v2_consumer_metric`].
/// Reads the NEW MarginEngine address from `option_event_indexer_config`
/// and the optional OLD MarginEngine address from the same struct
/// (`OLD_MARGIN_ENGINE_ADDRESS` env var). Pre-seeds all three label
/// values to zero so Prometheus alert rules of the shape
/// `increase(<metric>{consumer="old"}[5m]) > 0` have a stable time
/// series from the first scrape. Raw addresses are never promoted to a
/// label value.
fn append_option_fee_v2_consumer_metric(
    state: &AppState,
    metrics: &mut MetricsText,
    metric_name: &'static str,
    metric_help: &'static str,
    raw_counts: &BTreeMap<String, u64>,
) {
    use crate::fees::option_consumer::{
        classify_option_fee_consumer, CONSUMER_NEW, CONSUMER_OLD, CONSUMER_UNKNOWN,
    };

    let new_addr = non_zero_address(&state.option_event_indexer_config.margin_engine_address.0);
    let old_addr = state
        .option_event_indexer_config
        .old_margin_engine_address
        .as_ref()
        .and_then(|addr| non_zero_address(&addr.0));

    let mut bucketed: BTreeMap<String, u64> = BTreeMap::new();
    bucketed.insert(CONSUMER_NEW.to_string(), 0);
    bucketed.insert(CONSUMER_OLD.to_string(), 0);
    bucketed.insert(CONSUMER_UNKNOWN.to_string(), 0);
    for (consumer, count) in raw_counts {
        let bucket = classify_option_fee_consumer(consumer, new_addr, old_addr);
        let entry = bucketed.entry(bucket.to_string()).or_default();
        *entry = entry.saturating_add(*count);
    }

    metrics.labeled_gauges(metric_name, metric_help, &[("consumer", &bucketed)]);
}

fn non_zero_address(address: &str) -> Option<&str> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return None;
    }
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if !stripped.is_empty() && stripped.bytes().all(|byte| byte == b'0') {
        return None;
    }
    Some(trimmed)
}

fn append_mm_metrics(state: &AppState, metrics: &mut MetricsText) -> Result<()> {
    let sessions = if state.mm_gateway_config.enabled {
        state.mm_sessions.list_active()?
    } else {
        Vec::new()
    };
    let authenticated = sessions
        .iter()
        .filter(|session| session.authenticated)
        .count() as u64;
    metrics.gauge(
        "deopt_mm_sessions_total",
        "Active Market Maker Gateway sessions.",
        sessions.len() as u64,
    );
    metrics.gauge(
        "deopt_mm_sessions_authenticated_total",
        "Authenticated Market Maker Gateway sessions.",
        authenticated,
    );
    Ok(())
}

async fn metrics_repository(state: &AppState) -> Option<crate::db::PgRepository> {
    let repository = state.repository.clone()?;
    repository.admin_ping().await.ok()?;
    Some(repository)
}

fn bump_fee_event_count(counts: &mut BTreeMap<FeeEventLabels, u64>, event: &FeeEvent) {
    let count = counts
        .entry(FeeEventLabels {
            market_type: event.market_type.as_str().to_string(),
            flow_type: event.flow_type.as_str().to_string(),
            source_type: event.source_type.as_str().to_string(),
            status: event.status.as_str().to_string(),
        })
        .or_default();
    *count = count.saturating_add(1);
}

fn bump_count(counts: &mut BTreeMap<String, u64>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn execution_status_key(status: crate::execution::ExecutionIntentStatus) -> &'static str {
    match status {
        crate::execution::ExecutionIntentStatus::Pending => "pending",
        crate::execution::ExecutionIntentStatus::DryRun => "dry_run",
        crate::execution::ExecutionIntentStatus::CalldataReady => "calldata_ready",
        crate::execution::ExecutionIntentStatus::SimulationOk => "simulation_ok",
        crate::execution::ExecutionIntentStatus::SimulationFailed => "simulation_failed",
        crate::execution::ExecutionIntentStatus::Submitted => "submitted",
        crate::execution::ExecutionIntentStatus::Confirmed => "confirmed",
        crate::execution::ExecutionIntentStatus::Failed => "failed",
    }
}

fn bool_value(value: bool) -> u64 {
    if value {
        1
    } else {
        0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FeeEventLabels {
    pub market_type: String,
    pub flow_type: String,
    pub source_type: String,
    pub status: String,
}

struct MetricsText {
    output: String,
    described: BTreeSet<&'static str>,
}

impl MetricsText {
    fn new() -> Self {
        Self {
            output: String::new(),
            described: BTreeSet::new(),
        }
    }

    fn gauge(&mut self, name: &'static str, help: &'static str, value: u64) {
        self.describe(name, help, "gauge");
        self.output.push_str(name);
        self.output.push(' ');
        self.output.push_str(&value.to_string());
        self.output.push('\n');
    }

    fn labeled_gauges(
        &mut self,
        name: &'static str,
        help: &'static str,
        dimensions: &[(&'static str, &BTreeMap<String, u64>)],
    ) {
        self.describe(name, help, "gauge");
        if dimensions.len() != 1 {
            return;
        }
        let (label_name, counts) = dimensions[0];
        let mut sanitized_counts = BTreeMap::new();
        for (label_value, value) in counts {
            let count = sanitized_counts
                .entry(safe_label_value(label_value))
                .or_insert(0u64);
            *count = count.saturating_add(*value);
        }
        for (label_value, value) in sanitized_counts {
            self.output.push_str(name);
            self.output.push('{');
            self.output.push_str(label_name);
            self.output.push_str("=\"");
            self.output.push_str(&label_value);
            self.output.push_str("\"} ");
            self.output.push_str(&value.to_string());
            self.output.push('\n');
        }
    }

    fn fee_event_gauges(&mut self, counts: &BTreeMap<FeeEventLabels, u64>) {
        let name = "deopt_fee_events_total";
        self.describe(
            name,
            "Fee ledger events by low-cardinality labels.",
            "gauge",
        );
        let mut sanitized_counts = BTreeMap::new();
        for (labels, value) in counts {
            let labels = FeeEventLabels {
                market_type: safe_label_value(&labels.market_type),
                flow_type: safe_label_value(&labels.flow_type),
                source_type: safe_label_value(&labels.source_type),
                status: safe_label_value(&labels.status),
            };
            let count = sanitized_counts.entry(labels).or_insert(0u64);
            *count = count.saturating_add(*value);
        }
        for (labels, value) in sanitized_counts {
            self.output.push_str(name);
            self.output.push('{');
            self.output.push_str("market_type=\"");
            self.output.push_str(&labels.market_type);
            self.output.push_str("\",flow_type=\"");
            self.output.push_str(&labels.flow_type);
            self.output.push_str("\",source_type=\"");
            self.output.push_str(&labels.source_type);
            self.output.push_str("\",status=\"");
            self.output.push_str(&labels.status);
            self.output.push_str("\"} ");
            self.output.push_str(&value.to_string());
            self.output.push('\n');
        }
    }

    fn describe(&mut self, name: &'static str, help: &'static str, metric_type: &'static str) {
        if self.described.insert(name) {
            self.output.push_str("# HELP ");
            self.output.push_str(name);
            self.output.push(' ');
            self.output.push_str(help);
            self.output.push('\n');
            self.output.push_str("# TYPE ");
            self.output.push_str(name);
            self.output.push(' ');
            self.output.push_str(metric_type);
            self.output.push('\n');
        }
    }

    fn finish(self) -> String {
        self.output
    }
}

fn safe_label_value(value: &str) -> String {
    if value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        value.to_string()
    } else {
        "other".to_string()
    }
}
