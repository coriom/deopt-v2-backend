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
    /// PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — operator visibility for
    /// the two periodic Perps workers + their kill-switches +
    /// last-tick summary. Absent when neither worker has been
    /// configured (all defaults). Populated even when both workers are
    /// disabled so an operator can see the safe-default posture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perps_workers: Option<PerpsWorkersReadiness>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessCheck {
    pub name: &'static str,
    pub ok: bool,
    pub status: &'static str,
}

/// PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — public-safe view of the
/// funding + liquidation worker state. Never contains wallets,
/// signatures, RPC URLs, DB URLs, envelope digests, or subaccount
/// detail; only the flags an operator flipped and the last-tick
/// summary shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PerpsWorkersReadiness {
    pub funding_worker_enabled: bool,
    pub funding_tick_enabled: bool,
    pub funding_interval_sec: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_last_tick: Option<crate::perps::PerpsWorkerTickRecord>,
    pub liquidation_worker_enabled: bool,
    pub liquidation_tick_enabled: bool,
    pub liquidation_interval_sec: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidation_last_tick: Option<crate::perps::PerpsWorkerTickRecord>,
    /// Mirror of the public trading gate so an operator can see both
    /// the fail-closed posture and the worker state in one place. Not
    /// a security field — the gate itself lives on `AppState`.
    pub perps_public_trading_enabled: bool,
    /// Mirror of the closed-test gate. Same posture as above.
    pub perps_closed_test_enabled: bool,
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
        // PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — surface the Perps
        // public-route gate so ops tooling can observe whether the
        // opt-in flag has been flipped. `ok=true` because the readiness
        // probe cares about *serviceability*, not whether Perps
        // trading is on; a `false` public-route state is the safe
        // default posture, not an error.
        ReadinessCheck {
            name: "perps_public_routes",
            ok: true,
            status: if state.perps_public_trading_enabled {
                "enabled_flagged_closed_test"
            } else {
                "fail_closed"
            },
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

    // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — surface the periodic
    // worker + kill-switch state. `ready` stays coupled only to the
    // required checks; the workers are opt-in and their disabled
    // posture is expected — reporting on them does not gate readiness.
    let perps_workers = Some(PerpsWorkersReadiness {
        funding_worker_enabled: state.perps_funding_worker_config.worker_enabled,
        funding_tick_enabled: state.perps_funding_worker_config.tick_enabled,
        funding_interval_sec: state.perps_funding_worker_config.interval_sec,
        funding_last_tick: state
            .perp_funding_last_tick
            .lock()
            .ok()
            .and_then(|guard| *guard),
        liquidation_worker_enabled: state.perps_liquidation_worker_config.worker_enabled,
        liquidation_tick_enabled: state.perps_liquidation_worker_config.tick_enabled,
        liquidation_interval_sec: state.perps_liquidation_worker_config.interval_sec,
        liquidation_last_tick: state
            .perp_liquidation_last_tick
            .lock()
            .ok()
            .and_then(|guard| *guard),
        perps_public_trading_enabled: state.perps_public_trading_enabled,
        perps_closed_test_enabled: state.perps_closed_test_enabled,
    });

    ReadinessResponse {
        ready: checks.iter().all(|check| check.ok),
        service: "deopt-v2-backend",
        checks,
        perps_workers,
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
    // PERPS-FRONTEND-TICKET-ENABLEMENT-V1 — strict opt-in gate for
    // the /perps/orders + /perps/orders/:id mutation routes. Default
    // 0 (fail-closed). Legacy /orders, /rfqs* etc remain 0 always.
    metrics.gauge(
        "deopt_perps_public_routes_enabled",
        "Perps public mutation routes (POST /perps/orders, DELETE /perps/orders/:id) opt-in flag. Default 0 = fail-closed. Legacy /orders, /rfqs*, /execution-intents/*/signatures remain permanently fail-closed regardless.",
        bool_value(state.perps_public_trading_enabled),
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
    // PERPS-FUNDING-LIQUIDATION-WORKERS-V1 — periodic worker gauges.
    // All four are 0 by default (both workers disabled, both
    // kill-switches off) so Prometheus alerts of the shape
    // `deopt_perps_funding_worker_enabled == 1` fire only after the
    // operator has explicitly opted in.
    metrics.gauge(
        "deopt_perps_funding_worker_enabled",
        "Perps funding periodic worker enabled (0 = disabled / default).",
        bool_value(state.perps_funding_worker_config.worker_enabled),
    );
    metrics.gauge(
        "deopt_perps_funding_tick_enabled",
        "Perps funding tick kill-switch (0 = disabled). Consulted by both the periodic worker and the admin POST /admin/perps/funding/tick handler.",
        bool_value(state.perps_funding_worker_config.tick_enabled),
    );
    metrics.gauge(
        "deopt_perps_liquidation_worker_enabled",
        "Perps liquidation periodic worker enabled (0 = disabled / default).",
        bool_value(state.perps_liquidation_worker_config.worker_enabled),
    );
    metrics.gauge(
        "deopt_perps_liquidation_tick_enabled",
        "Perps liquidation tick kill-switch (0 = disabled). Consulted by both the periodic worker and the admin POST /admin/perps/liquidations/tick handler.",
        bool_value(state.perps_liquidation_worker_config.tick_enabled),
    );

    append_database_metrics(state, &mut metrics).await?;
    append_execution_metrics(state, &mut metrics).await?;
    append_rfq_metrics(state, &mut metrics).await?;
    append_option_metrics(state, &mut metrics).await?;
    append_fee_metrics(state, &mut metrics).await?;
    append_mm_metrics(state, &mut metrics)?;
    append_broadcast_observability_metrics(state, &mut metrics);

    Ok(metrics.finish())
}

/// Render the in-process [`BroadcastObservability`] snapshot into the
/// Prometheus text output. Counters are emitted as gauges (Prometheus
/// scraper computes rates downstream); last-seen live-read values are
/// emitted as gauges with the value bit-truncated to `u64` via
/// `u128_to_u64_gauge`. Labels are restricted to the
/// `BroadcastObservability` whitelist (`source_type`, `code`,
/// `signer_kind`); no intent_id / address / secret ever appears.
fn append_broadcast_observability_metrics(state: &AppState, metrics: &mut MetricsText) {
    let snap = state.broadcast_observability.snapshot();

    // ---- runtime LiveProvider config-state gauges (low cardinality,
    //      from typed config; complement the vault_observability layer's
    //      `deopt_protocol_fee_vault_configured` rendered above). ----
    metrics.gauge(
        "deopt_option_broadcast_protocol_fee_vault_configured",
        "1 if PROTOCOL_FEE_VAULT_ADDRESS is configured via OptionEventIndexerConfig and the runtime LiveProvider issues PFV reads. 0 if PFV reads are skipped silently.",
        bool_value(
            state
                .option_event_indexer_config
                .protocol_fee_vault_address
                .is_some(),
        ),
    );
    metrics.gauge(
        "deopt_option_broadcast_fees_manager_v2_configured",
        "1 if FEES_MANAGER_V2 address is configured and the runtime LiveProvider issues quoteFees + rebateBudget reads.",
        bool_value(
            state
                .option_event_indexer_config
                .fees_manager_v2_address
                .is_some(),
        ),
    );
    metrics.gauge(
        "deopt_option_broadcast_collateral_vault_configured",
        "1 if COLLATERAL_VAULT address is configured and the runtime LiveProvider issues CV(PFV,asset) reads (used by R5 precheck).",
        bool_value(
            !state
                .option_event_indexer_config
                .collateral_vault_address
                .0
                .is_empty(),
        ),
    );

    // ---- policy approve / reject counters (as gauges of cumulative count) ----
    metrics.append_labeled_gauge_2(
        "deopt_option_broadcast_policy_approved_total",
        "Cumulative count of option-execution broadcasts where should_broadcast approved, by source_type.",
        "source_type",
        &snap.policy_approved_total,
    );
    metrics.append_labeled_pair_gauge(
        "deopt_option_broadcast_policy_rejected_total",
        "Cumulative count of should_broadcast rejections, by reject code + source_type.",
        "code",
        "source_type",
        &snap.policy_rejected_total,
    );

    // ---- signer counters ----
    metrics.append_labeled_gauge_2(
        "deopt_option_broadcast_signer_attempted_total",
        "Cumulative count of signer-attempt invocations, by signer_kind.",
        "signer_kind",
        &snap.signer_attempted_total,
    );
    metrics.append_labeled_gauge_2(
        "deopt_option_broadcast_signer_success_total",
        "Cumulative count of signer-success returns, by signer_kind.",
        "signer_kind",
        &snap.signer_success_total,
    );
    metrics.append_labeled_pair_gauge(
        "deopt_option_broadcast_signer_denied_total",
        "Cumulative count of signer-denial returns, by signer-error code + signer_kind.",
        "code",
        "signer_kind",
        &snap.signer_denied_total,
    );

    // ---- policy data read failures ----
    metrics.append_labeled_gauge_2(
        "deopt_option_broadcast_policy_data_failures_total",
        "Cumulative count of policy-data provider read failures, by read-type tag.",
        "read_type",
        &snap.policy_data_failures_total,
    );

    metrics.gauge(
        "deopt_option_broadcast_econ_data_available_true_total",
        "Cumulative count of broadcast attempts where econ_data_available was true.",
        snap.econ_data_available_true_total,
    );
    metrics.gauge(
        "deopt_option_broadcast_econ_data_available_false_total",
        "Cumulative count of broadcast attempts where econ_data_available was false (boundary mode).",
        snap.econ_data_available_false_total,
    );
    metrics.gauge(
        "deopt_option_broadcast_fm_v2_decode_failures_total",
        "Cumulative count of FeesManagerV2 ABI decode failures during a gather_inputs call.",
        snap.fm_v2_decode_failures_total,
    );
    metrics.gauge(
        "deopt_option_broadcast_fm_v2_rpc_failures_total",
        "Cumulative count of FeesManagerV2 eth_call RPC failures during a gather_inputs call.",
        snap.fm_v2_rpc_failures_total,
    );
    metrics.gauge(
        "deopt_option_broadcast_r5_drift_observed_total",
        "Cumulative count of broadcast attempts where R5 drift (CV(PFV,asset) != feeBalance + rebateReserve) was observed.",
        snap.r5_drift_observed_total,
    );
    metrics.gauge(
        "deopt_option_broadcast_local_signer_on_mainnet_refused_total",
        "Cumulative count of mainnet runtime refusals of a LocalDev signer (defence-in-depth).",
        snap.local_signer_on_mainnet_refused_total,
    );

    // ---- last-seen live-read gauges (chain-state + economic) ----
    if let Some(value) = snap.last_be_balance_wei {
        metrics.gauge(
            "deopt_option_broadcast_last_be_balance_wei",
            "Most recent eth_getBalance(BACKEND_EXECUTOR) seen by a broadcast attempt (wei, truncated to u64).",
            u128_to_u64_gauge(value),
        );
    }
    if let Some(paused) = snap.last_ome_paused {
        metrics.gauge(
            "deopt_option_broadcast_last_ome_paused",
            "Most recent NEW_OME.paused() seen by a broadcast attempt (1 = paused, 0 = not paused).",
            bool_value(paused),
        );
    }
    if let Some(is_exec) = snap.last_ome_is_executor {
        metrics.gauge(
            "deopt_option_broadcast_last_ome_is_executor",
            "Most recent NEW_OME.isExecutor(BACKEND_EXECUTOR) seen by a broadcast attempt (1 = is executor, 0 = not).",
            bool_value(is_exec),
        );
    }
    if let Some(value) = snap.last_pfv_fee_balance {
        metrics.gauge(
            "deopt_option_broadcast_last_pfv_fee_balance",
            "Most recent PFV.feeBalance(asset) seen by a broadcast attempt (asset units, truncated to u64).",
            u128_to_u64_gauge(value),
        );
    }
    if let Some(value) = snap.last_pfv_rebate_reserve {
        metrics.gauge(
            "deopt_option_broadcast_last_pfv_rebate_reserve",
            "Most recent PFV.rebateReserve(asset) seen by a broadcast attempt (asset units, truncated to u64).",
            u128_to_u64_gauge(value),
        );
    }
    if let Some(value) = snap.last_cv_pfv_balance {
        metrics.gauge(
            "deopt_option_broadcast_last_cv_pfv_balance",
            "Most recent CV.balances(PFV,asset) seen by a broadcast attempt (asset units, truncated to u64).",
            u128_to_u64_gauge(value),
        );
    }
    if let Some(value) = snap.last_fm_v2_rebate_budget {
        metrics.gauge(
            "deopt_option_broadcast_last_fm_v2_rebate_budget",
            "Most recent FeesManagerV2.rebateBudget(asset) seen by a broadcast attempt (asset units, truncated to u64).",
            u128_to_u64_gauge(value),
        );
    }
    if let Some(r5) = snap.last_r5_drift_zero {
        metrics.gauge(
            "deopt_option_broadcast_last_r5_drift_zero",
            "Most recent R5 drift-zero result (1 = drift zero, 0 = drift observed).",
            bool_value(r5),
        );
    }
    if let Some(ms) = snap.last_broadcast_submitted_ms {
        metrics.gauge(
            "deopt_option_broadcast_last_submitted_ms",
            "UTC milliseconds of the most recent broadcast that the signer successfully signed.",
            ms.max(0) as u64,
        );
    }
}

fn u128_to_u64_gauge(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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
        state
            .option_event_indexer_config
            .protocol_fee_vault_address
            .clone(),
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

    /// Emit a single labelled gauge keyed by a string label.
    ///
    /// Used by [`append_broadcast_observability_metrics`] to render
    /// counter snapshots without growing the existing
    /// [`Self::labeled_gauges`] signature.
    fn append_labeled_gauge_2(
        &mut self,
        name: &'static str,
        help: &'static str,
        label_name: &'static str,
        counts: &BTreeMap<String, u64>,
    ) {
        self.describe(name, help, "gauge");
        let mut sanitized: BTreeMap<String, u64> = BTreeMap::new();
        for (label, value) in counts {
            let entry = sanitized.entry(safe_label_value(label)).or_insert(0);
            *entry = entry.saturating_add(*value);
        }
        for (label, value) in sanitized {
            self.output.push_str(name);
            self.output.push('{');
            self.output.push_str(label_name);
            self.output.push_str("=\"");
            self.output.push_str(&label);
            self.output.push_str("\"} ");
            self.output.push_str(&value.to_string());
            self.output.push('\n');
        }
    }

    /// Emit a gauge with two labels (e.g. reject code + source_type).
    fn append_labeled_pair_gauge(
        &mut self,
        name: &'static str,
        help: &'static str,
        label_a: &'static str,
        label_b: &'static str,
        counts: &BTreeMap<(String, String), u64>,
    ) {
        self.describe(name, help, "gauge");
        let mut sanitized: BTreeMap<(String, String), u64> = BTreeMap::new();
        for ((a, b), value) in counts {
            let key = (safe_label_value(a), safe_label_value(b));
            let entry = sanitized.entry(key).or_insert(0);
            *entry = entry.saturating_add(*value);
        }
        for ((a, b), value) in sanitized {
            self.output.push_str(name);
            self.output.push('{');
            self.output.push_str(label_a);
            self.output.push_str("=\"");
            self.output.push_str(&a);
            self.output.push_str("\",");
            self.output.push_str(label_b);
            self.output.push_str("=\"");
            self.output.push_str(&b);
            self.output.push_str("\"} ");
            self.output.push_str(&value.to_string());
            self.output.push('\n');
        }
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
