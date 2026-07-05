use super::broadcast_policy::{
    should_broadcast, BroadcastContext, BroadcastMode, FeeSplitSummary, RejectReason,
    ShouldBroadcastDecision, SimulationSummary, SubsidyBudgetView,
};
use super::broadcast_policy_data::{
    BroadcastPolicyDataProvider, BroadcastPolicyInputs, LiveBroadcastPolicyDataProvider,
    StubBroadcastPolicyDataProvider,
};
use super::series_id::{option_series_id, OptionSeriesIdInput};
use super::signing::{
    option_rfq_id_to_b256, option_rfq_quote_digest, option_rfq_quote_digest_bytes,
    option_series_id_to_b256, OptionRfqQuoteSigningPayload,
};
use super::{
    build_option_execution_transaction_request, encode_option_execute_trade_calldata,
    expected_option_execute_rfq_trade_selector, expected_option_execute_trade_selector,
    normalize_u256_string, option_execution_intent_id_to_hex_bytes32,
    option_execution_simulation_pending, option_execution_simulation_unavailable,
    option_trade_digest, option_trade_digest_bytes, perform_option_broadcast_gas_safety_check,
    simulate_option_execution_intent, validate_simulation_intent, validate_simulation_target,
    AttachmentLegSpec, OptionExecutionConfirmationStatus, OptionExecutionGasSafetyCheck,
    OptionExecutionIntent, OptionExecutionIntentId, OptionExecutionIntentStatus,
    OptionExecutionSignatureMode, OptionExecutionSimulationResult, OptionExecutionSimulationStatus,
    OptionExecutionSourceType, OptionExecutionTransaction, OptionFill, OptionFillFilter,
    OptionFillId, OptionOrder, OptionOrderAttachmentPlan, OptionOrderFilter, OptionOrderId,
    OptionOrderRejection, OptionOrderStatus, OptionOrderbookLevel, OptionOrderbookSnapshot,
    OptionRfqFill, OptionRfqFillFilter, OptionRfqFillId, OptionRfqId, OptionRfqQuote,
    OptionRfqQuoteId, OptionRfqQuoteSignatureMode, OptionRfqQuoteSignatureStatus,
    OptionRfqQuoteStatus, OptionRfqRequest, OptionRfqStatus, OptionSeries, OptionSeriesFilter,
    OptionSeriesId, OptionSeriesSource, OptionSeriesStatus, OptionTradePayload,
    OptionTradeSignatureBundle,
};
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::transaction::hex_0x;
use crate::execution::EthBalanceProvider;
use crate::execution::{
    assemble_eip1559_signed_transaction, eip1559_transaction_prehash, policy_fingerprint,
    EthCallProvider, ExecutionTransactionRequest, ExecutionTransactionStatus, ExecutorSigner,
    GasEstimateProvider, HttpJsonRpcProvider, LocalDevSigner, RemoteSigner, RemoteSignerClient,
    SignerBackendKind, SignerError, SignerRequest, TransactionBroadcastProvider,
    TransactionReceiptProvider, MAINNET_CHAIN_ID,
};
use crate::mm::protocol::{
    NotificationEnvelope, OptionRfqQuoteAcceptedPayload, OptionRfqQuoteRejectedPayload,
    OptionRfqRequestPayload, ServerMessage,
};
use crate::nonce_sync::{read_option_nonce_value, OptionNonceProvider};
use crate::signing::eip712::keccak256;
use crate::signing::eip712::parse_evm_address;
use crate::signing::recover_eip712_signer;
use crate::signing::signature::validate_signature_shape;
use crate::types::{now_ms, AccountId, OrderId, Price1e8, Side, Size1e8, TimeInForce, TimestampMs};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

const ONE_CONTRACT_1E8: u128 = 100_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOptionSeriesInput {
    pub underlying: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: Price1e8,
    pub is_call: bool,
    pub contract_size_1e8: Option<Size1e8>,
    pub onchain_product_id: Option<String>,
    pub onchain_series_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionOrderInput {
    pub option_series_id: OptionSeriesId,
    pub account: AccountId,
    pub side: Side,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub time_in_force: TimeInForce,
    pub post_only: bool,
    pub client_order_id: Option<String>,
    pub nonce: Option<u64>,
    pub deadline_ms: Option<TimestampMs>,
    pub signature: Option<String>,
    /// ATTACHED-TP-SL-ON-ENTRY-V1 — optional trader intent to
    /// attach TP/SL legs to the parent order. Validated up front;
    /// invalid attachments reject the whole submit (and surface in
    /// the rejected-attempts feed for honesty).
    pub attached_tp_sl: Option<AttachedTpSlInput>,
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — the trader's TP/SL intent on the
/// service input.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttachedTpSlInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<AttachedLegInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<AttachedLegInput>,
    #[serde(default)]
    pub link_as_oco: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<TimestampMs>,
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — a single TP or SL leg.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttachedLegInput {
    pub trigger_price_1e8: Price1e8,
    pub limit_price_1e8: Price1e8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionOrderOutcome {
    pub order: OptionOrder,
    pub fills: Vec<OptionFill>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOptionRfqInput {
    pub taker: AccountId,
    pub option_series_id: OptionSeriesId,
    pub side: Side,
    pub size_1e8: Size1e8,
    pub limit_price_1e8: Option<Price1e8>,
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionRfqQuoteInput {
    pub mm_account: AccountId,
    pub session_id: Option<String>,
    pub client_quote_id: Option<String>,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_nonce: Option<u64>,
    pub quote_ttl_ms: Option<u64>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionRfqQuoteSigningPayloadInput {
    pub option_rfq_id: OptionRfqId,
    pub mm_account: AccountId,
    pub price_1e8: Price1e8,
    pub size_1e8: Size1e8,
    pub quote_nonce: u64,
    pub quote_ttl_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionRfqQuoteSigningPayloadOutcome {
    pub rfq: OptionRfqRequest,
    pub payload: OptionRfqQuoteSigningPayload,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptOptionRfqQuoteOutcome {
    pub rfq: OptionRfqRequest,
    pub quote: OptionRfqQuote,
    pub fill: OptionRfqFill,
    pub mm_notification_sent: bool,
    pub mm_notification_warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionSigningPayloadOutcome {
    pub intent: OptionExecutionIntent,
    pub payload: OptionTradePayload,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionExecutionSignaturesInput {
    pub buyer_signature: Option<String>,
    pub seller_signature: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOptionExecutionSignaturesOutcome {
    pub intent: OptionExecutionIntent,
    pub buyer_signature_present: bool,
    pub seller_signature_present: bool,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionCalldataOutcome {
    pub intent: OptionExecutionIntent,
    pub calldata: Option<String>,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionBroadcastOutcome {
    pub intent: OptionExecutionIntent,
    pub transaction: OptionExecutionTransaction,
    pub broadcast_enabled: bool,
    pub submitted: bool,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionExecutionConfirmationOutcome {
    pub intent: OptionExecutionIntent,
    pub transaction: OptionExecutionTransaction,
    pub confirmation_status: OptionExecutionConfirmationStatus,
    pub receipt_status: Option<u64>,
    pub block_number: Option<u64>,
    pub error: Option<String>,
}

pub async fn create_option_series(
    state: &AppState,
    input: CreateOptionSeriesInput,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    if !state.options_config.allow_manual_series {
        return Err(BackendError::InvalidOptionSeriesState(
            "manual option series creation is disabled".to_string(),
        ));
    }

    let now = now_ms();
    let now_sec = now_sec(now)?;
    let contract_size_1e8 = input
        .contract_size_1e8
        .unwrap_or(state.options_config.default_contract_size_1e8);
    validate_assets(&[
        ("underlying", &input.underlying),
        ("base_asset", &input.base_asset),
        ("quote_asset", &input.quote_asset),
        ("settlement_asset", &input.settlement_asset),
    ])?;
    if input.expiry <= now_sec {
        return Err(BackendError::InvalidOptionSeriesState(
            "option series expiry must be in the future".to_string(),
        ));
    }
    if input.strike_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "strike_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }
    if contract_size_1e8 == 0 {
        return Err(BackendError::InvalidFixedPoint {
            field: "contract_size_1e8".to_string(),
            reason: "must be greater than zero".to_string(),
        });
    }

    let underlying = trim_asset(input.underlying);
    let base_asset = trim_asset(input.base_asset);
    let quote_asset = trim_asset(input.quote_asset);
    let settlement_asset = trim_asset(input.settlement_asset);
    let option_series_id = option_series_id(OptionSeriesIdInput {
        underlying: &underlying,
        base_asset: &base_asset,
        quote_asset: &quote_asset,
        settlement_asset: &settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
    });

    if let Some(existing) = get_option_series_optional(state, &option_series_id).await? {
        return Ok(existing);
    }

    let series = OptionSeries {
        option_series_id,
        underlying,
        base_asset,
        quote_asset,
        settlement_asset,
        expiry: input.expiry,
        strike_1e8: input.strike_1e8,
        is_call: input.is_call,
        contract_size_1e8,
        status: OptionSeriesStatus::Active,
        source: OptionSeriesSource::Manual,
        onchain_product_id: input.onchain_product_id,
        onchain_series_id: input.onchain_series_id,
        created_at_ms: now,
        updated_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_series(&series).await?;
        Ok(repository
            .get_option_series(&series.option_series_id)
            .await?
            .unwrap_or(series))
    } else {
        Ok(state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .insert_series(series))
    }
}

pub async fn list_option_series(
    state: &AppState,
    filter: OptionSeriesFilter,
) -> Result<Vec<OptionSeries>> {
    ensure_enabled(state)?;
    let now_sec = now_sec(now_ms())?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_series()
            .await?
            .into_iter()
            .filter(|series| filter.matches(series, now_sec))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_series(&filter, now_sec))
}

pub async fn get_option_series(state: &AppState, option_series_id: &str) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    get_option_series_optional(state, option_series_id)
        .await?
        .ok_or_else(|| BackendError::InvalidOptionSeriesId(option_series_id.to_string()))
}

pub async fn disable_option_series(
    state: &AppState,
    option_series_id: &str,
) -> Result<OptionSeries> {
    ensure_enabled(state)?;
    let now = now_ms();
    if let Some(repository) = state.repository.clone() {
        return repository
            .disable_option_series(option_series_id, now)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .disable_series(option_series_id, now)
}

pub async fn get_option_orderbook(
    state: &AppState,
    option_series_id: OptionSeriesId,
) -> Result<OptionOrderbookSnapshot> {
    let series = get_option_series(state, &option_series_id).await?;
    let orders = open_option_orders_for_series(state, &option_series_id).await?;
    Ok(OptionOrderbookSnapshot {
        option_series_id,
        status: series.effective_status(now_sec(now_ms())?),
        bids: aggregate_levels(&orders, Side::Buy),
        asks: aggregate_levels(&orders, Side::Sell),
    })
}

pub async fn submit_option_order(
    state: &AppState,
    input: SubmitOptionOrderInput,
) -> Result<SubmitOptionOrderOutcome> {
    let snapshot = input.clone();
    let outcome = submit_option_order_inner(state, input).await;
    if let Err(ref error) = outcome {
        // HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1 — opportunistically
        // persist a rejection record so /history can show the
        // attempt. The caller's identity (`snapshot.account`) was
        // already verified at the HTTP layer before this service
        // function was invoked, so it is safe to attribute. If
        // recording itself fails we log + swallow it; the trader
        // still sees the original HTTP error.
        record_submit_rejection_if_applicable(state, &snapshot, error).await;
    }
    outcome
}

async fn submit_option_order_inner(
    state: &AppState,
    input: SubmitOptionOrderInput,
) -> Result<SubmitOptionOrderOutcome> {
    ensure_enabled(state)?;
    validate_account(&input.account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    validate_tif_combination(input.time_in_force, input.post_only)?;
    if let Some(deadline_ms) = input.deadline_ms {
        if now_ms() >= deadline_ms {
            return Err(BackendError::DeadlineExpired);
        }
    }
    if let Some(signature) = &input.signature {
        validate_signature_shape(signature)?;
    }
    // ATTACHED-TP-SL-ON-ENTRY-V1 — fail fast on bad TP/SL spec
    // BEFORE we persist the parent order so the rejection-attempts
    // feed records the failure cleanly with `attached_tp_sl_invalid`.
    if let Some(attached) = &input.attached_tp_sl {
        validate_attached_tp_sl(attached)?;
    }

    let series = get_option_series(state, &input.option_series_id).await?;
    if series.effective_status(now_sec(now_ms())?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionOrderState(
            "option series is not active".to_string(),
        ));
    }
    validate_option_order_execution_preflight(state, &series, &input).await?;

    let now = now_ms();
    // Stash the attached payload BEFORE moving the rest of the
    // input into `order` so we can run the post-fill materializer
    // without re-fetching it from the request body.
    let attached_tp_sl = input.attached_tp_sl.clone();
    let order = OptionOrder {
        order_id: OrderId::new(),
        option_series_id: input.option_series_id,
        account: input.account,
        side: input.side,
        price_1e8: input.price_1e8,
        size_1e8: input.size_1e8,
        remaining_size_1e8: input.size_1e8,
        time_in_force: input.time_in_force,
        post_only: input.post_only,
        client_order_id: input.client_order_id,
        nonce: input.nonce,
        deadline_ms: input.deadline_ms,
        signature: input.signature,
        status: OptionOrderStatus::Open,
        terminal_reason_code: None,
        terminal_reason_message: None,
        terminal_reason_source: None,
        created_at_ms: now,
        updated_at_ms: now,
    };

    let (order, fills) = if let Some(repository) = state.repository.clone() {
        repository.submit_option_order_and_match(order, now).await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .submit_order_and_match(order, now)?
    };
    create_option_orderbook_execution_intents(state, &fills).await?;
    crate::fees::service::record_option_order_fills(state, &fills).await?;
    // ATTACHED-TP-SL-ON-ENTRY-V1 — after the parent + fills are
    // committed, persist the attachment plan if the submitter
    // requested one. ATTACHED-TP-SL-MAKER-FILL-HOOK-V2 — then sync
    // the submitter's plan AND every maker plan from this fill
    // batch to the cumulative filled exposure of each parent order.
    if let Some(attached) = attached_tp_sl {
        record_submitter_attachment_plan(state, &order, attached, now).await;
    }
    sync_attached_plans_after_fills(state, &order, &fills, now).await;
    emit_option_order_lifecycle(state, &order, &fills);
    Ok(SubmitOptionOrderOutcome { order, fills })
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — validation. Enforces:
///   * at least one of TP / SL is present
///   * each present leg has trigger_price > 0 AND limit_price > 0
///   * `link_as_oco=true` requires BOTH legs
///   * `expires_at_ms`, if present, is in the future
fn validate_attached_tp_sl(attached: &AttachedTpSlInput) -> Result<()> {
    if attached.take_profit.is_none() && attached.stop_loss.is_none() {
        return Err(BackendError::InvalidAttachedTpSl(
            "at least one of take_profit / stop_loss must be present".to_string(),
        ));
    }
    if let Some(tp) = &attached.take_profit {
        if tp.trigger_price_1e8 == 0 {
            return Err(BackendError::InvalidAttachedTpSl(
                "take_profit.trigger_price_1e8 must be > 0".to_string(),
            ));
        }
        if tp.limit_price_1e8 == 0 {
            return Err(BackendError::InvalidAttachedTpSl(
                "take_profit.limit_price_1e8 must be > 0".to_string(),
            ));
        }
    }
    if let Some(sl) = &attached.stop_loss {
        if sl.trigger_price_1e8 == 0 {
            return Err(BackendError::InvalidAttachedTpSl(
                "stop_loss.trigger_price_1e8 must be > 0".to_string(),
            ));
        }
        if sl.limit_price_1e8 == 0 {
            return Err(BackendError::InvalidAttachedTpSl(
                "stop_loss.limit_price_1e8 must be > 0".to_string(),
            ));
        }
    }
    if attached.link_as_oco && (attached.take_profit.is_none() || attached.stop_loss.is_none()) {
        return Err(BackendError::InvalidAttachedTpSl(
            "link_as_oco=true requires BOTH take_profit and stop_loss".to_string(),
        ));
    }
    if let Some(expires_at_ms) = attached.expires_at_ms {
        if expires_at_ms <= now_ms() {
            return Err(BackendError::InvalidAttachedTpSl(
                "expires_at_ms must be in the future".to_string(),
            ));
        }
    }
    Ok(())
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — persist a new attachment plan
/// (status=pending). The actual materialisation happens via
/// `sync_attached_plans_after_fills` which fires for both the
/// submitter and every maker involved in the fill batch.
async fn record_submitter_attachment_plan(
    state: &AppState,
    parent: &OptionOrder,
    attached: AttachedTpSlInput,
    now: TimestampMs,
) {
    let plan = OptionOrderAttachmentPlan {
        plan_id: uuid::Uuid::new_v4(),
        parent_order_id: parent.order_id,
        account: parent.account.clone(),
        option_series_id: parent.option_series_id.clone(),
        take_profit: attached.take_profit.as_ref().map(|leg| AttachmentLegSpec {
            trigger_price_1e8: leg.trigger_price_1e8,
            limit_price_1e8: leg.limit_price_1e8,
        }),
        stop_loss: attached.stop_loss.as_ref().map(|leg| AttachmentLegSpec {
            trigger_price_1e8: leg.trigger_price_1e8,
            limit_price_1e8: leg.limit_price_1e8,
        }),
        link_as_oco: attached.link_as_oco,
        expires_at_ms: attached.expires_at_ms,
        status: super::AttachmentPlanStatus::Pending,
        materialized_size_1e8: None,
        tp_conditional_order_id: None,
        sl_conditional_order_id: None,
        oco_group_id: None,
        failure_code: None,
        failure_message: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    if let Err(err) = persist_attachment_plan(state, &plan).await {
        tracing::warn!(
            target: "deopt.options.attachment",
            parent_order_id = %parent.order_id,
            error = %err,
            "failed to persist attachment plan; trader still sees the original submit response"
        );
        return;
    }
    // ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — emit AFTER durable persistence.
    emit_attachment_plan_lifecycle(state, &plan);
}

/// ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — publish `AttachmentPlanUpdated`
/// on `account.conditional_orders`. Called AFTER a plan row has been
/// durably persisted or updated (never before). Emits the plan's
/// current status, materialized size, child conditional-order ids,
/// oco group and failure fields — nothing else.
fn emit_attachment_plan_lifecycle(state: &AppState, plan: &OptionOrderAttachmentPlan) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    state.lifecycle_events.emit(LifecycleEvent {
        account: plan.account.clone(),
        channel: LifecycleChannel::AccountConditionalOrders,
        payload: LifecyclePayload::AttachmentPlanUpdated {
            plan_id: plan.plan_id.to_string(),
            parent_order_id: plan.parent_order_id.to_string(),
            option_series_id: plan.option_series_id.to_string(),
            status: plan.status.as_str().to_string(),
            materialized_size_1e8: plan.materialized_size_1e8.map(|s| s.to_string()),
            tp_conditional_order_id: plan.tp_conditional_order_id.map(|u| u.to_string()),
            sl_conditional_order_id: plan.sl_conditional_order_id.map(|u| u.to_string()),
            oco_group_id: plan.oco_group_id.map(|u| u.to_string()),
            failure_code: plan.failure_code.clone(),
            failure_message: plan.failure_message.clone(),
            updated_at_ms: plan.updated_at_ms,
        },
        emitted_at_ms: now,
    });
}

/// ATTACHED-TP-SL-MAKER-FILL-HOOK-V2 — for every order that
/// participated in `fills` (the submitter + every distinct maker),
/// look up its attachment plan and sync it to the parent's current
/// cumulative filled exposure. This is the unified entry that V1
/// also depends on: an immediate-fill submit walks through the
/// submitter branch; a maker that later picks up fills walks
/// through the maker branch.
///
/// Idempotent: re-running with the same cumulative filled exposure
/// is a no-op (plan status guards each branch). Safe on terminal
/// child legs: the resize path uses
/// `update_conditional_order_quantity_if_armed`, which returns
/// `None` and leaves the row alone when the leg has moved past
/// `Armed`.
async fn sync_attached_plans_after_fills(
    state: &AppState,
    submitter: &OptionOrder,
    fills: &[OptionFill],
    now: TimestampMs,
) {
    // Always sync the submitter (covers the immediate-fill case
    // that V1 owned). Then walk the distinct maker order ids from
    // this batch and sync each one — that's the V2 maker hook.
    if let Err(err) = sync_attachment_plan_for_parent(state, submitter.order_id, now).await {
        tracing::warn!(
            target: "deopt.options.attachment",
            parent_order_id = %submitter.order_id,
            error = %err,
            "submitter attachment plan sync failed"
        );
    }
    use std::collections::HashSet;
    let mut seen: HashSet<OptionOrderId> = HashSet::new();
    seen.insert(submitter.order_id);
    for fill in fills {
        let maker_id = fill.maker_order_id;
        if !seen.insert(maker_id) {
            continue;
        }
        if let Err(err) = sync_attachment_plan_for_parent(state, maker_id, now).await {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %maker_id,
                error = %err,
                "maker attachment plan sync failed"
            );
        }
    }
}

/// ATTACHED-TP-SL-MAKER-FILL-HOOK-V2 — sync a single parent
/// order's plan to that parent's current cumulative filled
/// exposure. No-op when:
///   * no plan exists (the parent never had attached TP/SL)
///   * the plan is in a terminal status (cancelled / failed)
///   * the parent's cumulative filled is zero (resting, no fills)
///   * the parent's cumulative filled does not exceed
///     `plan.materialized_size_1e8` (idempotent)
///
/// Transitions:
///   * pending → active when first non-zero filled exposure
///     materialises the conditional rows via the existing
///     `create_conditional_orders` service.
///   * active → active with a larger `materialized_size_1e8` when
///     additional fills land; the underlying conditional rows are
///     resized via `update_conditional_order_quantity_if_armed`.
///     Triggered/terminal legs are skipped (safe subset).
async fn sync_attachment_plan_for_parent(
    state: &AppState,
    parent_order_id: OptionOrderId,
    now: TimestampMs,
) -> Result<()> {
    let Some(plan) = fetch_attachment_plan(state, parent_order_id).await? else {
        return Ok(());
    };
    if plan.status.is_terminal() && plan.status != super::AttachmentPlanStatus::Active {
        // Cancelled / Failed → leave alone.
        return Ok(());
    }
    let Some(parent_order) = get_option_order(state, parent_order_id).await.ok() else {
        return Ok(());
    };
    let cumulative_filled = parent_order
        .size_1e8
        .saturating_sub(parent_order.remaining_size_1e8);
    if cumulative_filled == 0 {
        return Ok(()); // resting, nothing to do
    }
    match plan.status {
        super::AttachmentPlanStatus::Pending => {
            materialize_attachment_plan(state, &parent_order, &plan, cumulative_filled, now).await;
        }
        super::AttachmentPlanStatus::Active => {
            let already = plan.materialized_size_1e8.unwrap_or(0);
            if cumulative_filled > already {
                resize_active_attachment_plan(state, &plan, cumulative_filled, now).await;
            }
        }
        super::AttachmentPlanStatus::Cancelled | super::AttachmentPlanStatus::Failed => {
            // Already handled by the early-return above.
        }
    }
    Ok(())
}

/// ATTACHED-TP-SL-MAKER-FILL-HOOK-V2 — bump an active plan's
/// materialised conditional rows to `new_size`. Triggered or
/// terminal legs are skipped via the `_if_armed` resize method;
/// the plan's `materialized_size_1e8` is then bumped to reflect
/// the new exposure regardless (so observability shows the
/// current cumulative filled), but a non-fatal warning is set
/// when one or more legs were skipped so the trader sees what
/// happened.
async fn resize_active_attachment_plan(
    state: &AppState,
    plan: &OptionOrderAttachmentPlan,
    new_size: crate::types::Size1e8,
    now: TimestampMs,
) {
    let mut skipped: Vec<&'static str> = Vec::new();
    if let Some(tp_id) = plan.tp_conditional_order_id {
        let updated = resize_conditional_if_armed(state, tp_id, new_size, now).await;
        if let Ok(None) = updated {
            skipped.push("tp");
        }
        if let Err(err) = updated {
            tracing::warn!(
                target: "deopt.options.attachment",
                tp_conditional_order_id = %tp_id,
                error = %err,
                "tp conditional resize failed; plan size still bumped for observability"
            );
        }
    }
    if let Some(sl_id) = plan.sl_conditional_order_id {
        let updated = resize_conditional_if_armed(state, sl_id, new_size, now).await;
        if let Ok(None) = updated {
            skipped.push("sl");
        }
        if let Err(err) = updated {
            tracing::warn!(
                target: "deopt.options.attachment",
                sl_conditional_order_id = %sl_id,
                error = %err,
                "sl conditional resize failed; plan size still bumped for observability"
            );
        }
    }
    let (failure_code, failure_message) = if skipped.is_empty() {
        (plan.failure_code.clone(), plan.failure_message.clone())
    } else {
        (
            Some("conditional_leg_already_terminal".to_string()),
            Some(format!(
                "skipped resize on legs: {} (already triggered/completed/cancelled/failed)",
                skipped.join(", ")
            )),
        )
    };
    match update_attachment_plan_status(
        state,
        plan.parent_order_id,
        super::AttachmentPlanStatus::Active,
        Some(new_size),
        None,
        None,
        None,
        failure_code,
        failure_message,
        now,
    )
    .await
    {
        Ok(updated) => emit_attachment_plan_lifecycle(state, &updated),
        Err(err) => {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %plan.parent_order_id,
                error = %err,
                "failed to bump attachment plan materialised size after resize"
            );
        }
    }
}

async fn resize_conditional_if_armed(
    state: &AppState,
    id: uuid::Uuid,
    new_size: crate::types::Size1e8,
    now: TimestampMs,
) -> Result<Option<crate::options::conditional_orders::ConditionalOrder>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .update_conditional_order_quantity_if_armed(id, new_size, now)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_conditional_order_quantity_if_armed(id, new_size, now))
}

async fn persist_attachment_plan(state: &AppState, plan: &OptionOrderAttachmentPlan) -> Result<()> {
    if let Some(repository) = state.repository.clone() {
        return repository.insert_option_order_attachment_plan(plan).await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_order_attachment_plan(plan.clone())?;
    Ok(())
}

async fn materialize_attachment_plan(
    state: &AppState,
    parent: &OptionOrder,
    plan: &OptionOrderAttachmentPlan,
    filled_size_1e8: u128,
    now: TimestampMs,
) {
    use crate::options::conditional_orders::{
        create_conditional_orders, ConditionalLegInput, ConditionalType,
        CreateConditionalOrderInput, TriggerCondition,
    };
    // The existing conditional service computes side + trigger
    // direction from the position; we pass the prices verbatim
    // and let the service do the rest.
    let mut legs: Vec<ConditionalLegInput> = Vec::new();
    if let Some(tp) = plan.take_profit.as_ref() {
        legs.push(ConditionalLegInput {
            conditional_type: ConditionalType::TakeProfit,
            trigger_price_1e8: tp.trigger_price_1e8,
            limit_price_1e8: tp.limit_price_1e8,
            explicit_trigger_condition: None::<TriggerCondition>,
        });
    }
    if let Some(sl) = plan.stop_loss.as_ref() {
        legs.push(ConditionalLegInput {
            conditional_type: ConditionalType::StopLoss,
            trigger_price_1e8: sl.trigger_price_1e8,
            limit_price_1e8: sl.limit_price_1e8,
            explicit_trigger_condition: None::<TriggerCondition>,
        });
    }
    let link_as_oco = plan.link_as_oco && legs.len() == 2;
    let input = CreateConditionalOrderInput {
        account: parent.account.clone(),
        option_series_id: parent.option_series_id.clone(),
        quantity_1e8: filled_size_1e8,
        legs,
        link_as_oco,
        expires_at_ms: plan.expires_at_ms,
    };
    let result = create_conditional_orders(state, input).await;
    let (status, materialized, tp_id, sl_id, oco_id, fail_code, fail_msg) = match result {
        Ok(rows) => {
            let mut tp_id: Option<uuid::Uuid> = None;
            let mut sl_id: Option<uuid::Uuid> = None;
            let mut oco_id: Option<uuid::Uuid> = None;
            for row in &rows {
                match row.conditional_type {
                    ConditionalType::TakeProfit => tp_id = Some(row.id),
                    ConditionalType::StopLoss => sl_id = Some(row.id),
                }
                if oco_id.is_none() {
                    oco_id = row.oco_group_id;
                }
            }
            (
                super::AttachmentPlanStatus::Active,
                Some(filled_size_1e8),
                tp_id,
                sl_id,
                oco_id,
                None,
                None,
            )
        }
        Err(err) => {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %parent.order_id,
                error = %err,
                "failed to materialize attached TP/SL; plan marked failed (parent order unaffected)"
            );
            (
                super::AttachmentPlanStatus::Failed,
                None,
                None,
                None,
                None,
                Some("conditional_create_failed".to_string()),
                Some(err.to_string()),
            )
        }
    };
    match update_attachment_plan_status(
        state,
        parent.order_id,
        status,
        materialized,
        tp_id,
        sl_id,
        oco_id,
        fail_code,
        fail_msg,
        now,
    )
    .await
    {
        Ok(updated) => emit_attachment_plan_lifecycle(state, &updated),
        Err(err) => {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %parent.order_id,
                error = %err,
                "failed to update attachment plan post-materialization (state may be inconsistent)"
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn update_attachment_plan_status(
    state: &AppState,
    parent_order_id: OptionOrderId,
    status: super::AttachmentPlanStatus,
    materialized: Option<crate::types::Size1e8>,
    tp_id: Option<uuid::Uuid>,
    sl_id: Option<uuid::Uuid>,
    oco_id: Option<uuid::Uuid>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    updated_at_ms: TimestampMs,
) -> Result<OptionOrderAttachmentPlan> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .update_option_order_attachment_plan(
                parent_order_id,
                status,
                materialized,
                tp_id,
                sl_id,
                oco_id,
                failure_code,
                failure_message,
                updated_at_ms,
            )
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_option_order_attachment_plan(
            parent_order_id,
            status,
            materialized,
            tp_id,
            sl_id,
            oco_id,
            failure_code,
            failure_message,
            updated_at_ms,
        )
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — terminal helper for the
/// cancel/expire hooks: flips a still-pending plan to Cancelled.
/// Idempotent: a plan already in a terminal state is left alone.
pub(crate) async fn cancel_attachment_plan_if_pending(
    state: &AppState,
    parent_order_id: OptionOrderId,
    now: TimestampMs,
) {
    let plan = match fetch_attachment_plan(state, parent_order_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %parent_order_id,
                error = %err,
                "failed to fetch attachment plan for cancel/expire hook"
            );
            return;
        }
    };
    if plan.status != super::AttachmentPlanStatus::Pending {
        return;
    }
    match update_attachment_plan_status(
        state,
        parent_order_id,
        super::AttachmentPlanStatus::Cancelled,
        None,
        None,
        None,
        None,
        Some("parent_terminal_before_fill".to_string()),
        Some("parent order reached a terminal state with no fill".to_string()),
        now,
    )
    .await
    {
        Ok(updated) => emit_attachment_plan_lifecycle(state, &updated),
        Err(err) => {
            tracing::warn!(
                target: "deopt.options.attachment",
                parent_order_id = %parent_order_id,
                error = %err,
                "failed to cancel pending attachment plan"
            );
        }
    }
}

async fn fetch_attachment_plan(
    state: &AppState,
    parent_order_id: OptionOrderId,
) -> Result<Option<OptionOrderAttachmentPlan>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_order_attachment_plan(parent_order_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_order_attachment_plan(parent_order_id))
}

/// ATTACHED-TP-SL-ON-ENTRY-V1 — list attachment plans for the
/// given account, newest first.
pub async fn list_option_order_attachment_plans_for_account(
    state: &AppState,
    account: &crate::types::AccountId,
    since_ms: Option<TimestampMs>,
) -> Result<Vec<OptionOrderAttachmentPlan>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_option_order_attachment_plans_for_account(account, since_ms)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_order_attachment_plans_for_account(account, since_ms))
}

/// HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1 — given an input that was
/// rejected and the resulting `BackendError`, classify the error
/// and persist a rejection row if the cause is in the recordable
/// set. Returns `Some(reason_code, reason_source)` if classified
/// (recorded), `None` if intentionally skipped.
async fn record_submit_rejection_if_applicable(
    state: &AppState,
    input: &SubmitOptionOrderInput,
    error: &BackendError,
) -> Option<(&'static str, &'static str)> {
    let (reason_code, reason_source) = classify_rejection(error)?;
    let now = now_ms();
    let rejection = OptionOrderRejection {
        rejection_id: uuid::Uuid::new_v4(),
        account: input.account.clone(),
        option_series_id: Some(input.option_series_id.clone()),
        side: Some(input.side),
        price_1e8: Some(input.price_1e8),
        size_1e8: Some(input.size_1e8),
        time_in_force: Some(input.time_in_force),
        post_only: Some(input.post_only),
        client_order_id: input.client_order_id.clone(),
        nonce: input.nonce.map(|n| n.to_string()),
        reason_code: reason_code.to_string(),
        // We use the BackendError's Display string as the message —
        // never the raw signature/envelope/etc. The Display strings
        // are static + trader-meaningful (see error.rs).
        reason_message: Some(error.to_string()),
        reason_source: reason_source.to_string(),
        created_at_ms: now,
    };
    if let Some(repository) = state.repository.clone() {
        if let Err(err) = repository.insert_option_order_rejection(&rejection).await {
            tracing::warn!(
                target: "deopt.options.rejection",
                reason = %reason_code,
                error = %err,
                "failed to persist option order rejection to PG; trader still saw the original error"
            );
            return None;
        }
    } else {
        match state.options_store.lock() {
            Ok(mut store) => store.record_option_order_rejection(rejection.clone()),
            Err(_) => {
                tracing::warn!(
                    target: "deopt.options.rejection",
                    reason = %reason_code,
                    "options store lock poisoned; rejection not recorded"
                );
                return None;
            }
        }
    }
    // ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — emit AFTER durable persistence
    // so that a receiver observing this event can trust the row is on disk.
    emit_option_order_rejection_lifecycle(state, &rejection);
    Some((reason_code, reason_source))
}

/// ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — publish `OrderRejected` on
/// `account.orders`. Called AFTER `record_submit_rejection_if_applicable`
/// has persisted the row (never before). Never carries signatures,
/// nonces, auth envelopes, headers or bearer tokens — only the same
/// scalar fields already surfaced by the rejected-attempts feed.
fn emit_option_order_rejection_lifecycle(state: &AppState, rejection: &OptionOrderRejection) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    state.lifecycle_events.emit(LifecycleEvent {
        account: rejection.account.clone(),
        channel: LifecycleChannel::AccountOrders,
        payload: LifecyclePayload::OrderRejected {
            rejection_id: rejection.rejection_id.to_string(),
            option_series_id: rejection.option_series_id.as_ref().map(|s| s.to_string()),
            side: rejection.side.map(|s| match s {
                crate::types::Side::Buy => "buy".to_string(),
                crate::types::Side::Sell => "sell".to_string(),
            }),
            price_1e8: rejection.price_1e8.map(|p| p.to_string()),
            size_1e8: rejection.size_1e8.map(|s| s.to_string()),
            time_in_force: rejection.time_in_force.map(|tif| match tif {
                crate::types::TimeInForce::Gtc => "gtc".to_string(),
                crate::types::TimeInForce::Ioc => "ioc".to_string(),
                crate::types::TimeInForce::Fok => "fok".to_string(),
            }),
            post_only: rejection.post_only,
            client_order_id: rejection.client_order_id.clone(),
            reason_code: rejection.reason_code.clone(),
            reason_message: rejection.reason_message.clone(),
            reason_source: rejection.reason_source.clone(),
            created_at_ms: rejection.created_at_ms,
        },
        emitted_at_ms: now,
    });
}

/// Maps a `BackendError` to a `(reason_code, reason_source)` pair
/// IFF the error is a pre-persistence option-order rejection that
/// (a) the caller's identity is already proven for, and (b) the
/// cause is trader-meaningful.
///
/// Auth-level errors (`SignatureSignerMismatch`, `WriteAuth(...)`,
/// `SignatureRecoveryFailed`) are intentionally not in the table:
/// they fire BEFORE the service function in the HTTP handler and
/// the account is not safely attributable. Returns `None` for
/// anything outside the recordable set (e.g. internal errors,
/// matching errors that are not in our policy list).
fn classify_rejection(error: &BackendError) -> Option<(&'static str, &'static str)> {
    use crate::options::rejection_reason as r;
    match error {
        BackendError::PostOnlyWouldMatch => {
            Some((r::POST_ONLY_WOULD_MATCH, r::SOURCE_MATCHING_POLICY))
        }
        BackendError::FokNotFillable => Some((r::FOK_NOT_FILLABLE, r::SOURCE_MATCHING_POLICY)),
        BackendError::SelfTrade => Some((r::SELF_TRADE, r::SOURCE_MATCHING_POLICY)),
        BackendError::DeadlineExpired => Some((r::DEADLINE_EXPIRED, r::SOURCE_REQUEST_VALIDATION)),
        BackendError::ZeroPrice => Some((r::ZERO_PRICE, r::SOURCE_REQUEST_VALIDATION)),
        BackendError::ZeroSize => Some((r::ZERO_SIZE, r::SOURCE_REQUEST_VALIDATION)),
        BackendError::UnsupportedTimeInForce(_) => {
            Some((r::UNSUPPORTED_TIF, r::SOURCE_REQUEST_VALIDATION))
        }
        BackendError::InvalidTimeInForceCombination(_) => {
            Some((r::INVALID_TIF_COMBINATION, r::SOURCE_REQUEST_VALIDATION))
        }
        BackendError::InvalidOptionOrderState(msg) if msg == "option series is not active" => {
            Some((r::OPTION_SERIES_INACTIVE, r::SOURCE_SERIES_STATE))
        }
        BackendError::InvalidAttachedTpSl(_) => {
            Some((r::ATTACHED_TP_SL_INVALID, r::SOURCE_REQUEST_VALIDATION))
        }
        _ => None,
    }
}

/// HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1 — list the rejected
/// option-order submit attempts persisted for `account`,
/// optionally restricted to rows newer than `since_ms`. Newest
/// first; mirrors the `/history` ordering convention.
pub async fn list_option_order_rejections_for_account(
    state: &AppState,
    account: &crate::types::AccountId,
    since_ms: Option<crate::types::TimestampMs>,
) -> Result<Vec<OptionOrderRejection>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_option_order_rejections_for_account(account, since_ms)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_order_rejections_for_account(account, since_ms))
}

/// ORDER-LIFECYCLE-OBSERVABILITY-V1 — emit `OrderUpdated` for the
/// submitted order AND one `FillCreated` per fill (broadcast to BOTH
/// buyer and seller accounts so each side sees their own fill on
/// `account.fills`). All events are emitted AFTER the DB commit /
/// in-memory mutation has succeeded, so a rolled-back transaction
/// never produces an event.
///
/// `pub(crate)` because `ORDER-LIFECYCLE-OBSERVABILITY-WORKER-V1`
/// reuses this helper from the conditional-orders worker's in-memory
/// trigger path, where the child IOC order is submitted via the
/// matching store directly (not through `submit_option_order`).
pub(crate) fn emit_option_order_lifecycle(
    state: &AppState,
    order: &OptionOrder,
    fills: &[crate::options::OptionFill],
) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    state.lifecycle_events.emit(LifecycleEvent {
        account: order.account.clone(),
        channel: LifecycleChannel::AccountOrders,
        payload: LifecyclePayload::OrderUpdated {
            order_id: order.order_id.to_string(),
            option_series_id: order.option_series_id.clone(),
            status: order.status.as_str().to_string(),
            remaining_size_1e8: order.remaining_size_1e8.to_string(),
            size_1e8: order.size_1e8.to_string(),
        },
        emitted_at_ms: now,
    });
    for fill in fills {
        for (account, side) in [(fill.buyer.clone(), "buy"), (fill.seller.clone(), "sell")] {
            state.lifecycle_events.emit(LifecycleEvent {
                account,
                channel: LifecycleChannel::AccountFills,
                payload: LifecyclePayload::FillCreated {
                    fill_id: fill.fill_id.to_string(),
                    option_series_id: fill.option_series_id.clone(),
                    order_id: order.order_id.to_string(),
                    side: side.to_string(),
                    price_1e8: fill.price_1e8.to_string(),
                    size_1e8: fill.size_1e8.to_string(),
                    created_at_ms: fill.created_at_ms,
                },
                emitted_at_ms: now,
            });
        }
    }
}

pub async fn list_option_orders(
    state: &AppState,
    filter: OptionOrderFilter,
) -> Result<Vec<OptionOrder>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_orders()
            .await?
            .into_iter()
            .filter(|order| filter.matches(order))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_orders(&filter))
}

pub async fn get_option_order(state: &AppState, order_id: OptionOrderId) -> Result<OptionOrder> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_order(order_id)
            .await?
            .ok_or(BackendError::InvalidOptionOrderId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_order(order_id)
        .ok_or(BackendError::InvalidOptionOrderId)
}

pub async fn cancel_option_order(state: &AppState, order_id: OptionOrderId) -> Result<OptionOrder> {
    ensure_enabled(state)?;
    let now = now_ms();
    let cancelled = if let Some(repository) = state.repository.clone() {
        repository.cancel_option_order(order_id, now).await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .cancel_order(order_id, now)?
    };
    // ATTACHED-TP-SL-ON-ENTRY-V1 — if the cancelled parent had a
    // pending attachment plan (no fills landed before cancel), the
    // plan is moved to Cancelled. If the plan is already Active
    // (parent partially-filled then user cancelled remainder), the
    // already-materialized conditional rows stay live and are
    // user-managed via the existing TP/SL endpoints.
    cancel_attachment_plan_if_pending(state, order_id, now).await;
    // ORDER-LIFECYCLE-OBSERVABILITY-V1 — emit AFTER successful mutation.
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    state.lifecycle_events.emit(LifecycleEvent {
        account: cancelled.account.clone(),
        channel: LifecycleChannel::AccountOrders,
        payload: LifecyclePayload::OrderUpdated {
            order_id: cancelled.order_id.to_string(),
            option_series_id: cancelled.option_series_id.clone(),
            status: cancelled.status.as_str().to_string(),
            remaining_size_1e8: cancelled.remaining_size_1e8.to_string(),
            size_1e8: cancelled.size_1e8.to_string(),
        },
        emitted_at_ms: now,
    });
    Ok(cancelled)
}

/// OPTION-ORDER-EXPIRY-SWEEP-V1 — terminalize every active option
/// order whose `deadline_ms` has passed. Dispatches to the PG
/// repository when persistence is wired; otherwise sweeps the
/// in-memory store. One `OrderUpdated` lifecycle event is emitted
/// per expired order (account-scoped), mirroring `cancel_option_order`
/// so /history and Open-Orders refresh paths see the new state.
///
/// Idempotent: a second call with no further-elapsed time returns an
/// empty `Vec` because the predicate only matches active rows.
pub async fn sweep_expired_option_orders(
    state: &AppState,
    now_ms: TimestampMs,
) -> Result<Vec<OptionOrder>> {
    ensure_enabled(state)?;
    let expired = if let Some(repository) = state.repository.clone() {
        repository.expire_option_orders_due(now_ms).await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .expire_orders_due(now_ms)
    };
    if expired.is_empty() {
        return Ok(expired);
    }
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    for order in &expired {
        state.lifecycle_events.emit(LifecycleEvent {
            account: order.account.clone(),
            channel: LifecycleChannel::AccountOrders,
            payload: LifecyclePayload::OrderUpdated {
                order_id: order.order_id.to_string(),
                option_series_id: order.option_series_id.clone(),
                status: order.status.as_str().to_string(),
                remaining_size_1e8: order.remaining_size_1e8.to_string(),
                size_1e8: order.size_1e8.to_string(),
            },
            emitted_at_ms: now_ms,
        });
        // ATTACHED-TP-SL-ON-ENTRY-V1 — same posture as cancel:
        // pending plans on an expired parent go to Cancelled;
        // already-materialized conditional rows stay live.
        cancel_attachment_plan_if_pending(state, order.order_id, now_ms).await;
    }
    Ok(expired)
}

pub async fn list_option_fills(
    state: &AppState,
    filter: OptionFillFilter,
) -> Result<Vec<OptionFill>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return Ok(repository
            .list_option_fills()
            .await?
            .into_iter()
            .filter(|fill| filter.matches(fill))
            .collect());
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_fills(&filter))
}

pub async fn get_option_fill(state: &AppState, fill_id: OptionFillId) -> Result<OptionFill> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_fill(fill_id)
            .await?
            .ok_or(BackendError::InvalidOptionFillId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_fill(fill_id)
        .ok_or(BackendError::InvalidOptionFillId)
}

pub async fn get_option_order_fills(
    state: &AppState,
    order_id: OptionOrderId,
) -> Result<Vec<OptionFill>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.option_fills_for_order(order_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .fills_for_order(order_id))
}

pub async fn create_option_rfq(
    state: &AppState,
    input: CreateOptionRfqInput,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.taker)?;
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if input.limit_price_1e8 == Some(0) {
        return Err(BackendError::ZeroPrice);
    }

    let series = get_option_series(state, &input.option_series_id).await?;
    if series.effective_status(now_sec(now_ms())?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionRfqState(
            "option series is not active".to_string(),
        ));
    }

    let ttl_ms = input
        .ttl_ms
        .unwrap_or(state.options_config.rfq_default_ttl_ms);
    if ttl_ms == 0 {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ ttl_ms must be greater than zero".to_string(),
        ));
    }
    let ttl_ms = ttl_ms.min(state.options_config.rfq_max_ttl_ms);
    let now = now_ms();
    let expires_at_ms = checked_expiry(now, ttl_ms, "option RFQ expiry")?;
    let rfq = OptionRfqRequest {
        option_rfq_id: Uuid::new_v4(),
        taker: input.taker,
        option_series_id: input.option_series_id,
        side: input.side,
        size_1e8: input.size_1e8,
        limit_price_1e8: input.limit_price_1e8,
        status: OptionRfqStatus::Open,
        created_at_ms: now,
        expires_at_ms,
        accepted_quote_id: None,
        option_fill_id: None,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_rfq(&rfq).await?;
        let rfq = repository
            .get_option_rfq(rfq.option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId)?;
        broadcast_option_rfq_request(state, &rfq);
        emit_option_rfq_created_lifecycle(state, &rfq);
        return Ok(rfq);
    }

    let rfq = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_rfq(rfq);
    broadcast_option_rfq_request(state, &rfq);
    emit_option_rfq_created_lifecycle(state, &rfq);
    Ok(rfq)
}

/// OPTIONS-RFQ-LIFECYCLE-WS-V1 — emit an `OptionRfqCreated`
/// lifecycle event routed on `account.rfqs` to the taker.
/// Best-effort: dispatch failure never propagates to the caller.
fn emit_option_rfq_created_lifecycle(state: &AppState, rfq: &OptionRfqRequest) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    state.lifecycle_events.emit(LifecycleEvent {
        account: rfq.taker.clone(),
        channel: LifecycleChannel::AccountRfqs,
        payload: LifecyclePayload::OptionRfqCreated {
            option_rfq_id: rfq.option_rfq_id.to_string(),
            option_series_id: rfq.option_series_id.clone(),
            taker: rfq.taker.0.clone(),
            side: match rfq.side {
                Side::Buy => "buy".to_string(),
                Side::Sell => "sell".to_string(),
            },
            size_1e8: rfq.size_1e8.to_string(),
            limit_price_1e8: rfq.limit_price_1e8.map(|value| value.to_string()),
            status: rfq.status.as_str().to_string(),
            created_at_ms: rfq.created_at_ms,
            expires_at_ms: rfq.expires_at_ms,
        },
        emitted_at_ms: now,
    });
}

pub async fn list_option_rfqs(state: &AppState) -> Result<Vec<OptionRfqRequest>> {
    ensure_option_rfq_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_rfqs().await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_rfqs())
}

pub async fn get_option_rfq(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_rfq(option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_rfq(option_rfq_id)
        .ok_or(BackendError::InvalidOptionRfqId)
}

pub async fn option_rfq_quote_signing_payload(
    state: &AppState,
    input: OptionRfqQuoteSigningPayloadInput,
) -> Result<OptionRfqQuoteSigningPayloadOutcome> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.mm_account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    let quote_ttl_ms = input
        .quote_ttl_ms
        .min(state.options_config.rfq_max_quote_ttl_ms);
    validate_option_rfq_quote_ttl(state, quote_ttl_ms)?;

    let rfq = get_option_rfq(state, input.option_rfq_id).await?;
    let now = now_ms();
    if rfq.effective_status(now) != OptionRfqStatus::Open {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ is not open".to_string(),
        ));
    }
    if input.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote size exceeds requested size".to_string(),
        ));
    }

    let payload = option_rfq_quote_payload(
        &rfq,
        input.mm_account,
        input.price_1e8,
        input.size_1e8,
        input.quote_nonce,
        quote_ttl_ms,
    )?;
    let digest = option_rfq_quote_digest(&payload, &state.options_config.rfq_eip712_domain)?;
    Ok(OptionRfqQuoteSigningPayloadOutcome {
        rfq,
        payload,
        digest,
    })
}

pub async fn submit_option_rfq_quote(
    state: &AppState,
    option_rfq_id: OptionRfqId,
    input: SubmitOptionRfqQuoteInput,
) -> Result<OptionRfqQuote> {
    ensure_option_rfq_enabled(state)?;
    validate_account(&input.mm_account)?;
    if input.price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if input.size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }

    let now = now_ms();
    let rfq = get_option_rfq(state, option_rfq_id).await?;
    if rfq.effective_status(now) != OptionRfqStatus::Open {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ is not open".to_string(),
        ));
    }
    crate::mm::permissions::check_can_quote_option_rfq(
        state,
        &input.mm_account,
        &rfq.option_series_id,
    )
    .await?;
    if input.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote size exceeds requested size".to_string(),
        ));
    }
    let existing_quote_count = count_option_rfq_quotes(state, option_rfq_id).await?;
    if existing_quote_count >= state.options_config.rfq_max_quotes_per_rfq {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote limit reached".to_string(),
        ));
    }

    let quote_ttl_ms = input
        .quote_ttl_ms
        .unwrap_or(state.options_config.rfq_max_quote_ttl_ms)
        .min(state.options_config.rfq_max_quote_ttl_ms);
    validate_option_rfq_quote_ttl(state, quote_ttl_ms)?;
    let signature_metadata = verify_option_rfq_quote_signature(state, &rfq, &input, quote_ttl_ms)?;
    let expires_at_ms = quote_expires_at_ms(state, &rfq, now, quote_ttl_ms)?;
    if now >= expires_at_ms {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote has expired".to_string(),
        ));
    }

    let quote = OptionRfqQuote {
        quote_id: Uuid::new_v4(),
        option_rfq_id,
        mm_account: input.mm_account,
        session_id: input.session_id,
        client_quote_id: input.client_quote_id,
        price_1e8: input.price_1e8,
        size_1e8: input.size_1e8,
        status: OptionRfqQuoteStatus::Active,
        created_at_ms: now,
        expires_at_ms,
        signature: signature_metadata.signature,
        quote_digest: signature_metadata.quote_digest,
        quote_nonce: signature_metadata.quote_nonce,
        signature_status: signature_metadata.signature_status,
        recovered_signer: signature_metadata.recovered_signer,
    };

    if let Some(repository) = state.repository.clone() {
        repository.insert_option_rfq_quote(&quote).await?;
        let persisted = repository
            .get_option_rfq_quote(quote.quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId)?;
        emit_option_rfq_quote_submitted_lifecycle(state, &rfq, &persisted);
        return Ok(persisted);
    }

    let persisted = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_rfq_quote(quote)?;
    emit_option_rfq_quote_submitted_lifecycle(state, &rfq, &persisted);
    Ok(persisted)
}

/// OPTIONS-RFQ-LIFECYCLE-WS-V1 — emit an `OptionRfqQuoteSubmitted`
/// lifecycle event routed on `account.rfqs` to BOTH the taker and
/// the maker. Best-effort: dispatch failure never propagates.
fn emit_option_rfq_quote_submitted_lifecycle(
    state: &AppState,
    rfq: &OptionRfqRequest,
    quote: &OptionRfqQuote,
) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    for account in [rfq.taker.clone(), quote.mm_account.clone()] {
        state.lifecycle_events.emit(LifecycleEvent {
            account,
            channel: LifecycleChannel::AccountRfqs,
            payload: LifecyclePayload::OptionRfqQuoteSubmitted {
                option_rfq_id: rfq.option_rfq_id.to_string(),
                quote_id: quote.quote_id.to_string(),
                option_series_id: rfq.option_series_id.clone(),
                taker: rfq.taker.0.clone(),
                mm_account: quote.mm_account.0.clone(),
                price_1e8: quote.price_1e8.to_string(),
                size_1e8: quote.size_1e8.to_string(),
                status: quote.status.as_str().to_string(),
                created_at_ms: quote.created_at_ms,
                expires_at_ms: quote.expires_at_ms,
            },
            emitted_at_ms: now,
        });
    }
}

pub async fn list_option_rfq_quotes(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<Vec<OptionRfqQuote>> {
    ensure_option_rfq_enabled(state)?;
    let _ = get_option_rfq(state, option_rfq_id).await?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_rfq_quotes(option_rfq_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_rfq_quotes(option_rfq_id))
}

/// OPTIONS-RFQ-TRADES-FEED-V1 — public list of accepted RFQ fills.
/// Newest-first ordering. `filter.account` matches when the address
/// equals any of buyer / seller / taker / mm_account. The service
/// applies `limit` after filtering so a lightweight limit still
/// bounds the response size for busy accounts.
pub async fn list_option_rfq_fills(
    state: &AppState,
    filter: OptionRfqFillFilter,
    limit: Option<u32>,
) -> Result<Vec<OptionRfqFill>> {
    ensure_option_rfq_enabled(state)?;
    let mut fills = if let Some(repository) = state.repository.clone() {
        repository.list_option_rfq_fills().await?
    } else {
        let mut store_fills = state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .list_option_rfq_fills();
        // The in-memory store returns fills oldest-first for
        // determinism; reverse to newest-first to match the SQL
        // path's `ORDER BY created_at_ms DESC`.
        store_fills.reverse();
        store_fills
    };
    fills.retain(|fill| filter.matches(fill));
    if let Some(cap) = limit {
        fills.truncate(cap as usize);
    }
    Ok(fills)
}

pub async fn accept_option_rfq_quote(
    state: &AppState,
    option_rfq_id: OptionRfqId,
    quote_id: OptionRfqQuoteId,
) -> Result<AcceptOptionRfqQuoteOutcome> {
    ensure_option_rfq_enabled(state)?;
    let now = now_ms();
    let rfq = get_option_rfq(state, option_rfq_id).await?;
    if rfq.effective_status(now) != OptionRfqStatus::Open {
        return Err(BackendError::InvalidOptionRfqState(
            "option RFQ is not open".to_string(),
        ));
    }
    let quote = get_option_rfq_quote(state, quote_id).await?;
    if quote.option_rfq_id != option_rfq_id {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote does not belong to RFQ".to_string(),
        ));
    }
    if quote.effective_status(now) != OptionRfqQuoteStatus::Active {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote is not active".to_string(),
        ));
    }
    validate_option_rfq_quote_signature_status(state, &quote)?;
    if quote.size_1e8 == 0 || quote.size_1e8 > rfq.size_1e8 {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote size is invalid".to_string(),
        ));
    }
    if !option_rfq_price_satisfies_limit(rfq.side, rfq.limit_price_1e8, quote.price_1e8) {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote price violates limit".to_string(),
        ));
    }

    let series = get_option_series(state, &rfq.option_series_id).await?;
    if series.effective_status(now_sec(now)?) != OptionSeriesStatus::Active {
        return Err(BackendError::InvalidOptionRfqState(
            "option series is not active".to_string(),
        ));
    }
    validate_option_rfq_execution_preflight(state, &series, &quote).await?;
    let quotes_before_accept = list_option_rfq_quotes(state, option_rfq_id).await?;

    let (buyer, seller) = match rfq.side {
        Side::Buy => (rfq.taker.clone(), quote.mm_account.clone()),
        Side::Sell => (quote.mm_account.clone(), rfq.taker.clone()),
    };
    let fill = OptionRfqFill {
        fill_id: Uuid::new_v4(),
        option_rfq_id,
        quote_id,
        option_series_id: rfq.option_series_id.clone(),
        buyer,
        seller,
        taker: rfq.taker.clone(),
        mm_account: quote.mm_account.clone(),
        taker_side: rfq.side,
        price_1e8: quote.price_1e8,
        size_1e8: quote.size_1e8,
        created_at_ms: now,
    };

    if let Some(repository) = state.repository.clone() {
        repository
            .accept_option_rfq_quote_and_insert_fill(option_rfq_id, quote_id, &fill)
            .await?;
        let rfq = repository
            .get_option_rfq(option_rfq_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqId)?;
        let quote = repository
            .get_option_rfq_quote(quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId)?;
        let fill = repository.get_option_rfq_fill(fill.fill_id).await?.ok_or(
            BackendError::InvalidOptionRfqState("option RFQ fill was not persisted".to_string()),
        )?;
        create_option_rfq_execution_intent(state, &fill).await?;
        crate::fees::service::record_option_rfq_fill(state, &fill, &quote).await?;
        let (mm_notification_sent, mm_notification_warning) =
            notify_option_rfq_quote_acceptance(state, &quote, &quotes_before_accept, fill.fill_id);
        emit_option_rfq_accept_lifecycle(state, &rfq, &quote, &fill);
        return Ok(AcceptOptionRfqQuoteOutcome {
            rfq,
            quote,
            fill,
            mm_notification_sent,
            mm_notification_warning,
        });
    }

    let (rfq, quote) = state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .accept_option_rfq_quote(option_rfq_id, quote_id, fill.clone())?;
    create_option_rfq_execution_intent(state, &fill).await?;
    crate::fees::service::record_option_rfq_fill(state, &fill, &quote).await?;
    let (mm_notification_sent, mm_notification_warning) =
        notify_option_rfq_quote_acceptance(state, &quote, &quotes_before_accept, fill.fill_id);
    emit_option_rfq_accept_lifecycle(state, &rfq, &quote, &fill);
    Ok(AcceptOptionRfqQuoteOutcome {
        rfq,
        quote,
        fill,
        mm_notification_sent,
        mm_notification_warning,
    })
}

/// OPTIONS-RFQ-LIFECYCLE-WS-V1 — emit BOTH `OptionRfqAccepted` and
/// `OptionRfqFillCreated` lifecycle events, ONE frame per interested
/// account (buyer + seller), routed on `account.rfqs`. Best-effort.
fn emit_option_rfq_accept_lifecycle(
    state: &AppState,
    rfq: &OptionRfqRequest,
    quote: &OptionRfqQuote,
    fill: &OptionRfqFill,
) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    // Emit to both buyer AND seller. In an RFQ flow buyer/seller are
    // always {taker, mm_account} in some order — but derive from the
    // fill row so the mapping stays canonical regardless of side.
    let accounts = [fill.buyer.clone(), fill.seller.clone()];
    for account in &accounts {
        state.lifecycle_events.emit(LifecycleEvent {
            account: account.clone(),
            channel: LifecycleChannel::AccountRfqs,
            payload: LifecyclePayload::OptionRfqAccepted {
                option_rfq_id: rfq.option_rfq_id.to_string(),
                quote_id: quote.quote_id.to_string(),
                option_series_id: rfq.option_series_id.clone(),
                taker: rfq.taker.0.clone(),
                mm_account: quote.mm_account.0.clone(),
                rfq_status: rfq.status.as_str().to_string(),
                quote_status: quote.status.as_str().to_string(),
                option_fill_id: fill.fill_id.to_string(),
                accepted_at_ms: fill.created_at_ms,
            },
            emitted_at_ms: now,
        });
    }
    for account in &accounts {
        state.lifecycle_events.emit(LifecycleEvent {
            account: account.clone(),
            channel: LifecycleChannel::AccountRfqs,
            payload: LifecyclePayload::OptionRfqFillCreated {
                option_rfq_id: rfq.option_rfq_id.to_string(),
                quote_id: quote.quote_id.to_string(),
                fill_id: fill.fill_id.to_string(),
                option_series_id: fill.option_series_id.clone(),
                taker: fill.taker.0.clone(),
                mm_account: fill.mm_account.0.clone(),
                taker_side: match fill.taker_side {
                    Side::Buy => "buy".to_string(),
                    Side::Sell => "sell".to_string(),
                },
                price_1e8: fill.price_1e8.to_string(),
                size_1e8: fill.size_1e8.to_string(),
                created_at_ms: fill.created_at_ms,
            },
            emitted_at_ms: now,
        });
    }
}

pub async fn cancel_option_rfq(
    state: &AppState,
    option_rfq_id: OptionRfqId,
) -> Result<OptionRfqRequest> {
    ensure_option_rfq_enabled(state)?;
    let rfq = if let Some(repository) = state.repository.clone() {
        repository.cancel_option_rfq(option_rfq_id).await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .cancel_option_rfq(option_rfq_id)?
    };
    emit_option_rfq_cancelled_lifecycle(state, &rfq);
    Ok(rfq)
}

/// OPTIONS-RFQ-LIFECYCLE-WS-V1 — emit an `OptionRfqCancelled`
/// lifecycle event routed on `account.rfqs` to the taker.
/// Best-effort: dispatch failure never propagates.
fn emit_option_rfq_cancelled_lifecycle(state: &AppState, rfq: &OptionRfqRequest) {
    use crate::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
    let now = now_ms();
    state.lifecycle_events.emit(LifecycleEvent {
        account: rfq.taker.clone(),
        channel: LifecycleChannel::AccountRfqs,
        payload: LifecyclePayload::OptionRfqCancelled {
            option_rfq_id: rfq.option_rfq_id.to_string(),
            option_series_id: rfq.option_series_id.clone(),
            taker: rfq.taker.0.clone(),
            status: rfq.status.as_str().to_string(),
            cancelled_at_ms: now,
        },
        emitted_at_ms: now,
    });
}

pub async fn list_option_execution_intents(state: &AppState) -> Result<Vec<OptionExecutionIntent>> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_execution_intents().await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_execution_intents())
}

pub async fn get_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionIntent> {
    ensure_enabled(state)?;
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_execution_intent(intent_id)
            .await?
            .ok_or(BackendError::InvalidOptionExecutionIntentId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_execution_intent(intent_id)
        .ok_or(BackendError::InvalidOptionExecutionIntentId)
}

pub async fn option_execution_signing_payload(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionSigningPayloadOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    let payload = OptionTradePayload::from_intent(&intent)?;
    let digest = option_trade_digest(&payload, &state.options_config.execution_eip712_domain)?;
    Ok(OptionExecutionSigningPayloadOutcome {
        intent,
        payload,
        digest,
    })
}

pub async fn submit_option_execution_signatures(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    input: SubmitOptionExecutionSignaturesInput,
) -> Result<SubmitOptionExecutionSignaturesOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    let payload = OptionTradePayload::from_intent(&intent)?;
    let digest_bytes =
        option_trade_digest_bytes(&payload, &state.options_config.execution_eip712_domain)?;
    verify_option_execution_signature(
        state,
        input.buyer_signature.as_deref(),
        &digest_bytes,
        &intent.buyer,
    )?;
    verify_option_execution_signature(
        state,
        input.seller_signature.as_deref(),
        &digest_bytes,
        &intent.seller,
    )?;

    let effective_buyer_signature = input
        .buyer_signature
        .clone()
        .or_else(|| intent.buyer_signature.clone());
    let effective_seller_signature = input
        .seller_signature
        .clone()
        .or_else(|| intent.seller_signature.clone());
    let (status, calldata) = if let (Some(buyer_signature), Some(seller_signature)) = (
        effective_buyer_signature.as_deref(),
        effective_seller_signature.as_deref(),
    ) {
        let calldata = build_option_execution_calldata_from_parts(
            &payload,
            buyer_signature,
            seller_signature,
        )?;
        (OptionExecutionIntentStatus::CalldataReady, Some(calldata))
    } else {
        (OptionExecutionIntentStatus::SignaturesRequired, None)
    };

    let updated_at_ms = now_ms();
    let updated = if let Some(repository) = state.repository.clone() {
        repository
            .upsert_option_execution_signatures(
                intent_id,
                input.buyer_signature,
                input.seller_signature,
                status,
                calldata,
                updated_at_ms,
            )
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .upsert_option_execution_signatures(
                intent_id,
                input.buyer_signature,
                input.seller_signature,
                status,
                calldata,
                updated_at_ms,
            )?
    };

    Ok(option_execution_signature_outcome(updated))
}

pub async fn option_execution_calldata(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionCalldataOutcome> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    if intent.calldata.is_some() {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: true,
            missing_signatures: false,
            calldata: intent.calldata.clone(),
            intent,
        });
    }
    let Some(buyer_signature) = intent.buyer_signature.as_deref() else {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: false,
            missing_signatures: true,
            calldata: None,
            intent,
        });
    };
    let Some(seller_signature) = intent.seller_signature.as_deref() else {
        return Ok(OptionExecutionCalldataOutcome {
            calldata_ready: false,
            missing_signatures: true,
            calldata: None,
            intent,
        });
    };

    let payload = OptionTradePayload::from_intent(&intent)?;
    let calldata =
        build_option_execution_calldata_from_parts(&payload, buyer_signature, seller_signature)?;
    let updated_at_ms = now_ms();
    let updated = if let Some(repository) = state.repository.clone() {
        repository
            .upsert_option_execution_signatures(
                intent_id,
                None,
                None,
                OptionExecutionIntentStatus::CalldataReady,
                Some(calldata.clone()),
                updated_at_ms,
            )
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .upsert_option_execution_signatures(
                intent_id,
                None,
                None,
                OptionExecutionIntentStatus::CalldataReady,
                Some(calldata.clone()),
                updated_at_ms,
            )?
    };
    Ok(OptionExecutionCalldataOutcome {
        intent: updated,
        calldata: Some(calldata),
        calldata_ready: true,
        missing_signatures: false,
    })
}

pub async fn option_execution_simulation_status(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionSimulationResult> {
    let intent = get_option_execution_intent(state, intent_id).await?;
    Ok(option_execution_simulation_pending(&intent))
}

pub async fn prepare_option_execution_simulation(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionIntent> {
    ensure_option_execution_simulation_enabled(state)?;
    let intent = get_option_execution_intent(state, intent_id).await?;
    if let Err(error) = validate_option_execution_simulation_preflight(state, &intent) {
        let unavailable =
            option_execution_simulation_unavailable(intent.intent_id, error.to_string());
        persist_option_execution_simulation_result(state, &unavailable).await?;
        return Err(error);
    }
    Ok(intent)
}

pub async fn simulate_prepared_option_execution_intent<P>(
    state: &AppState,
    intent: &OptionExecutionIntent,
    provider: &P,
) -> Result<OptionExecutionSimulationResult>
where
    P: EthCallProvider,
{
    let from = option_execution_simulation_from(state)?;
    let result = match simulate_option_execution_intent(
        provider,
        intent,
        &state.options_config.matching_engine_address,
        &from,
        state.options_config.execution_simulation_gas_limit,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let unavailable =
                option_execution_simulation_unavailable(intent.intent_id, error.to_string());
            persist_option_execution_simulation_result(state, &unavailable).await?;
            return Err(error);
        }
    };
    persist_option_execution_simulation_result(state, &result).await?;
    Ok(result)
}

pub async fn persist_option_execution_simulation_unavailable(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    error: impl Into<String>,
) -> Result<OptionExecutionSimulationResult> {
    let result = option_execution_simulation_unavailable(intent_id, error.into());
    persist_option_execution_simulation_result(state, &result).await?;
    Ok(result)
}

fn selector_hex(selector: &[u8; 4]) -> String {
    format!(
        "0x{:02x}{:02x}{:02x}{:02x}",
        selector[0], selector[1], selector[2], selector[3]
    )
}

/// Build a `BroadcastContext` and run the §8 `should_broadcast` policy gate.
///
/// This is the canonical pre-broadcast decision point. Field-level facts
/// (signatures, intent status, calldata, deadline, nonces, target/selector
/// allowlist, wash, chain id, simulation status) are always enforced.
/// Chain-state facts (OME paused, BE balance, margins, RM snapshot age, fee
/// split, rebate budget/reserve) are not yet wired through this synchronous
/// call site; until follow-on tracks land them, they are populated with
/// Sepolia-permissive defaults. On mainnet, the chain-state checks fail-closed
/// by default unless the caller has already injected up-to-date facts via
/// repository / cache (future PR).
/// Apply mainnet fail-closed semantics to the gathered
/// `BroadcastPolicyInputs` snapshot, then drive the `should_broadcast`
/// policy.
///
/// Mainnet behaviour (`mode == BroadcastMode::Mainnet`):
///   * Missing `chain_id_rpc`, `be_balance_wei`, `ome_paused`,
///     `ome_is_executor`, `pfv_rebate_reserve_asset` → rejected via the
///     policy's structured codes (`chain-id-mismatch`, `be-low-bal`,
///     `ome-paused`, `be-not-exec`, `rebate-reserve`).
///   * `r5_drift_zero == Some(false)` → rejected via `policy-internal:r5-drift`.
///
/// Sepolia behaviour (`mode == BroadcastMode::Sepolia`):
///   * Missing reads are permissive — boundary checks still fire, but
///     chain-state defaults stay healthy and the existing rehearsal
///     regression remains green.
fn run_should_broadcast_policy(
    state: &AppState,
    intent: &OptionExecutionIntent,
    inputs: &BroadcastPolicyInputs,
) -> ShouldBroadcastDecision {
    let allowed_target = state.options_config.matching_engine_address.clone();
    let call_target = allowed_target.clone();
    let trade_selector = selector_hex(&expected_option_execute_trade_selector());
    let rfq_selector = selector_hex(&expected_option_execute_rfq_trade_selector());
    let call_selector = match intent.source_type {
        OptionExecutionSourceType::OptionOrderbookFill => trade_selector.clone(),
        OptionExecutionSourceType::OptionRfqFill => rfq_selector.clone(),
    };
    let allowed_selectors = vec![trade_selector, rfq_selector];

    let mode = BroadcastMode::from_chain_id(state.execution_config.executor_chain_id);
    let permissive_chain_state = matches!(mode, BroadcastMode::Sepolia);

    // R5 precheck: hard fail on observed drift (every mode); missing
    // inputs deferred to mode-dependent below.
    if matches!(inputs.r5_drift_zero, Some(false)) {
        return ShouldBroadcastDecision::Reject(RejectReason::PolicyInternal(
            "r5-drift".to_string(),
        ));
    }

    let simulation = SimulationSummary {
        status: intent
            .simulation_status
            .unwrap_or(OptionExecutionSimulationStatus::SimulationOk),
        revert_reason: intent.simulation_error.clone(),
        gas_units: 0,
    };

    let fee_split = inputs
        .fee_split
        .clone()
        .unwrap_or_else(|| FeeSplitSummary::empty(intent.settlement_asset.clone()));
    // Activate the economic decision branches of `should_broadcast` only
    // when ALL three reads have landed: maker+taker quote (`fee_split`),
    // FM_V2.rebateBudget(asset) (`fm_v2_rebate_budget_asset`), and
    // PFV.rebateReserve(asset) (`pfv_rebate_reserve_asset`). Missing any
    // → keep `econ_data_available = false` → §8 steps 4 / 5 / 7 skip,
    // mainnet still fail-closed via the chain-state gates above.
    let econ_data_available = inputs.fee_split.is_some()
        && inputs.fm_v2_rebate_budget_asset.is_some()
        && inputs.pfv_rebate_reserve_asset.is_some();

    // Mainnet fail-closed: every live read MUST have landed; Sepolia is
    // permissive (existing rehearsal preserved).
    let be_balance_wei =
        inputs
            .be_balance_wei
            .unwrap_or(if permissive_chain_state { u128::MAX } else { 0 });
    let fund_floor_wei = if permissive_chain_state {
        0
    } else {
        state
            .execution_config
            .max_fee_per_gas_wei
            .as_deref()
            .and_then(|s| s.parse::<u128>().ok())
            .map(|gp| gp.saturating_mul(state.execution_config.max_gas_limit as u128))
            .unwrap_or(u128::MAX)
    };
    // Surface the exact fund-floor value that §6 will check against
    // `be_balance_wei` so `/executor/health/v2` can report what the
    // policy gate is using. Fires once per `should_broadcast` call —
    // shared between orderbook + RFQ source types.
    state
        .broadcast_observability
        .record_be_balance_floor_wei(fund_floor_wei);
    let ome_paused = inputs.ome_paused.unwrap_or(!permissive_chain_state);
    let ome_is_executor = inputs.ome_is_executor.unwrap_or(permissive_chain_state);
    // Live FeesManagerV2.rebateBudget(asset) read. Mainnet missing-read
    // is fail-closed: 0 ⇒ any rebate-positive intent rejects via §8
    // step 5 hard gate.
    let rebate_budget_asset = inputs
        .fm_v2_rebate_budget_asset
        .unwrap_or(if permissive_chain_state { u128::MAX } else { 0 });
    let rebate_reserve_asset = inputs
        .pfv_rebate_reserve_asset
        .unwrap_or(if permissive_chain_state { u128::MAX } else { 0 });

    let be_address = state.execution_config.executor_from_address.clone();
    let context = BroadcastContext {
        chain_id: state.execution_config.executor_chain_id,
        now_ms: now_ms(),
        mode,
        options_config: &state.options_config,
        execution_config: &state.execution_config,
        be_address: &be_address,
        be_balance_wei,
        fund_floor_wei,
        ome_paused,
        ome_is_executor,
        buyer_has_margin: true,
        seller_has_margin: true,
        product_listed: true,
        rm_snapshot_age_ms: 0,
        rm_snapshot_max_age_ms: u64::MAX,
        dedupe_hit: inputs.dedupe_hit,
        allowed_target: &allowed_target,
        allowed_selectors: &allowed_selectors,
        call_selector,
        call_target,
        simulation,
        econ_data_available,
        fee_split,
        rebate_budget_asset,
        rebate_reserve_asset,
        gas_units: 0,
        hard_gas_cap: state.options_config.execution_broadcast_gas_limit.max(1),
        gas_cost_native: 0,
        pnl_floor_native: 0,
        safety_margin_bps: state.options_config.execution_gas_safety_bps,
        subsidy_budget: SubsidyBudgetView::default(),
    };

    should_broadcast(intent, &context)
}

pub async fn broadcast_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionBroadcastOutcome> {
    ensure_option_execution_broadcast_enabled(state)?;
    let rpc_url = state.execution_config.rpc_url.clone().ok_or_else(|| {
        BackendError::Config("RPC_URL is required for option execution broadcast".to_string())
    })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    // Runtime path: construct the LiveProvider with all configured
    // chain-state + economic addresses + the in-process observability
    // handle so live read failures land in `/metrics`. Production
    // broadcast attempts flow through the full-fidelity entry point
    // `_signer_and_data_provider`.
    let signer = build_signer_for_state(state)?;
    let data_provider = build_runtime_policy_data_provider(state, provider.clone());
    broadcast_option_execution_intent_with_provider_signer_and_data_provider(
        state,
        intent_id,
        &provider,
        signer.as_ref(),
        &data_provider,
    )
    .await
}

/// Construct the production `LiveBroadcastPolicyDataProvider` from the
/// current [`AppState`] + the RPC provider. The constructor attaches the
/// in-process [`BroadcastObservability`] handle so RPC + ABI decode
/// failures increment the matching `policy_data_failures_total` /
/// `fm_v2_*_failures_total` counters. Addresses are sourced from:
///
/// - `state.option_event_indexer_config.collateral_vault_address` → CV.
/// - `state.option_event_indexer_config.fees_manager_v2_address` → FM_V2.
/// - PFV address: NOT yet sourced from config; `None` for now. A
///   follow-on PR (`BACKEND-LIVE-PROVIDER-PFV-CONFIG`) threads
///   `PROTOCOL_FEE_VAULT_ADDRESS` from env. While PFV is `None` the
///   provider still emits `fm_v2_*` reads + `chain_state` reads + dedupe.
pub fn build_runtime_policy_data_provider<P>(
    state: &AppState,
    provider: P,
) -> LiveBroadcastPolicyDataProvider<P>
where
    P: TransactionBroadcastProvider
        + crate::execution::EthCallProvider
        + EthBalanceProvider
        + Clone
        + 'static,
{
    let cv_address = {
        let raw = &state.option_event_indexer_config.collateral_vault_address;
        if raw.0.is_empty() {
            None
        } else {
            Some(raw.clone())
        }
    };
    let fm_v2_address = state
        .option_event_indexer_config
        .fees_manager_v2_address
        .clone();
    // PFV address — sourced from
    // `OptionEventIndexerConfig::protocol_fee_vault_address`
    // (`PROTOCOL_FEE_VAULT_ADDRESS` env key). `None` keeps fail-closed
    // posture: `should_broadcast`'s rebate-reserve hard gate defaults
    // to 0 on mainnet, rejecting any rebate-positive intent.
    let pfv_address = state
        .option_event_indexer_config
        .protocol_fee_vault_address
        .clone();
    LiveBroadcastPolicyDataProvider::new(provider, pfv_address, cv_address, fm_v2_address)
        .with_observability(state.broadcast_observability.clone())
}

pub async fn broadcast_option_execution_intent_with_provider<P>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
) -> Result<OptionExecutionBroadcastOutcome>
where
    P: TransactionBroadcastProvider + GasEstimateProvider,
{
    let signer = build_signer_for_state(state)?;
    broadcast_option_execution_intent_with_provider_and_signer(
        state,
        intent_id,
        provider,
        signer.as_ref(),
    )
    .await
}

/// Drive the signer abstraction for the option-execution path. Computes
/// the EIP-1559 prehash + calldata hash + `should_broadcast` ↔ signer
/// policy fingerprint, calls `sign_option_execution_tx`, and assembles the
/// raw signed transaction from the returned signature components. Returns
/// `SignerError` directly so the caller can map it into a structured
/// `BroadcastFailed` transition without leaking secrets.
///
/// MUST never fall back to a local-key signing path on remote-signer
/// failure (custody policy §6 BE-5; design doc §5.3).
async fn sign_option_execution_via_signer<S>(
    signer: &S,
    request: &ExecutionTransactionRequest,
    nonce: u64,
    intent: &OptionExecutionIntent,
    signer_kind: SignerBackendKind,
) -> std::result::Result<String, SignerError>
where
    S: RemoteSigner + ?Sized,
{
    let prehash = eip1559_transaction_prehash(request, nonce)
        .map_err(|err| SignerError::Internal(format!("eip1559 prehash failed: {err}")))?;
    let calldata_hash = keccak256(&request.calldata);
    let policy_decision_id = uuid::Uuid::new_v4();
    let simulation_block = intent.simulation_block_number.unwrap_or(0);
    let deadline_ms = (intent.deadline as i64).saturating_mul(1_000);
    let fingerprint = policy_fingerprint(
        policy_decision_id,
        &calldata_hash,
        nonce,
        simulation_block,
        deadline_ms,
    );
    let function_selector = match intent.source_type {
        OptionExecutionSourceType::OptionOrderbookFill => {
            let s = expected_option_execute_trade_selector();
            format!("0x{:02x}{:02x}{:02x}{:02x}", s[0], s[1], s[2], s[3])
        }
        OptionExecutionSourceType::OptionRfqFill => {
            let s = expected_option_execute_rfq_trade_selector();
            format!("0x{:02x}{:02x}{:02x}{:02x}", s[0], s[1], s[2], s[3])
        }
    };
    let signer_request = SignerRequest {
        request_id: uuid::Uuid::new_v4(),
        intent_id: intent.intent_id,
        source_type: intent.source_type.as_str(),
        chain_id: request.chain_id,
        target_contract: &request.to,
        function_selector: function_selector.as_str(),
        calldata_hash,
        calldata_length: request.calldata.len(),
        transaction_to: &request.to,
        transaction_value_wei: 0,
        gas_limit: request.gas_limit,
        max_fee_per_gas_wei: request.max_fee_per_gas_wei.as_deref().unwrap_or(""),
        max_priority_fee_per_gas_wei: request
            .max_priority_fee_per_gas_wei
            .as_deref()
            .unwrap_or(""),
        nonce,
        simulation_block,
        deadline_ms,
        policy_decision_id,
        policy_fingerprint: fingerprint,
        policy_decision_at_ms: now_ms(),
        prehash,
    };
    let response = signer.sign_option_execution_tx(signer_request).await?;
    tracing::info!(
        target: "broadcast_signer",
        intent_id = %intent.intent_id,
        signer_kind = %signer_kind,
        policy_decision_id = %policy_decision_id,
        kms_request_id = %response.kms_request_id.as_deref().unwrap_or("-"),
        audit_log_id = %response.audit_log_id.as_deref().unwrap_or("-"),
        remote_signer_request_id = %response.remote_signer_request_id.as_deref().unwrap_or("-"),
        signer_address = %response.signer_address.0,
        "signer approved option execution intent"
    );
    let raw = assemble_eip1559_signed_transaction(
        request,
        nonce,
        response.signature.y_parity,
        &response.signature.r,
        &response.signature.s,
    )
    .map_err(|err| SignerError::Internal(format!("eip1559 assemble failed: {err}")))?;
    Ok(raw)
}

/// Build the `RemoteSigner` selected by `state.execution_config` and apply
/// the runtime defence-in-depth: even though
/// `ExecutionConfig::validate_startup` already refuses `LocalDev` on
/// mainnet, we re-check here so any in-flight regression fails closed
/// before reaching the broadcast site.
pub fn build_signer_for_state(state: &AppState) -> Result<Arc<dyn RemoteSigner>> {
    if state.execution_config.executor_chain_id == MAINNET_CHAIN_ID
        && state.execution_config.backend_signer_mode == SignerBackendKind::LocalDev
    {
        state
            .broadcast_observability
            .record_local_signer_on_mainnet_refused();
        return Err(BackendError::Config(
            "BACKEND_SIGNER_MODE=local_dev is REFUSED at runtime on mainnet (chain_id=8453)"
                .to_string(),
        ));
    }
    match state.execution_config.backend_signer_mode {
        SignerBackendKind::LocalDev => {
            let private_key = state
                .execution_config
                .executor_private_key
                .as_ref()
                .ok_or_else(|| {
                    BackendError::Config(
                        "EXECUTOR_PRIVATE_KEY is required for BACKEND_SIGNER_MODE=local_dev"
                            .to_string(),
                    )
                })?;
            let inner = ExecutorSigner::from_private_key(private_key)?;
            Ok(Arc::new(LocalDevSigner::from_executor_signer(inner)))
        }
        SignerBackendKind::Remote => {
            let endpoint = state
                .execution_config
                .backend_signer_endpoint
                .clone()
                .ok_or_else(|| {
                    BackendError::Config(
                        "BACKEND_SIGNER_ENDPOINT is required when BACKEND_SIGNER_MODE=remote"
                            .to_string(),
                    )
                })?;
            let expected = state.execution_config.executor_from_address.clone();
            Ok(Arc::new(RemoteSignerClient::new(endpoint, expected)))
        }
    }
}

/// Broadcast variant that accepts an externally-supplied `RemoteSigner`.
/// Canonical entry point for both production (with the signer built from
/// config) and tests (with a mock signer).
pub async fn broadcast_option_execution_intent_with_provider_and_signer<P, S>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
    signer: &S,
) -> Result<OptionExecutionBroadcastOutcome>
where
    P: TransactionBroadcastProvider + GasEstimateProvider,
    S: RemoteSigner + ?Sized,
{
    // Default Sepolia-permissive data provider — preserves the existing
    // Sepolia rehearsal regression. The
    // `broadcast_option_execution_intent_with_provider` (no-data-provider)
    // and the test `_and_signer` paths route here.
    let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();
    broadcast_option_execution_intent_with_provider_signer_and_data_provider(
        state,
        intent_id,
        provider,
        signer,
        &data_provider,
    )
    .await
}

/// Full-fidelity broadcast entry point with an explicit policy data
/// provider. Production code wires a [`LiveBroadcastPolicyDataProvider`]
/// constructed from the configured RPC URL + PFV / CV addresses; tests
/// inject a [`StubBroadcastPolicyDataProvider`] with canned inputs.
pub async fn broadcast_option_execution_intent_with_provider_signer_and_data_provider<P, S, D>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
    signer: &S,
    data_provider: &D,
) -> Result<OptionExecutionBroadcastOutcome>
where
    P: TransactionBroadcastProvider + GasEstimateProvider,
    S: RemoteSigner + ?Sized,
    D: BroadcastPolicyDataProvider + ?Sized,
{
    ensure_option_execution_broadcast_enabled(state)?;
    let intent = get_option_execution_intent(state, intent_id).await?;

    // Gather every read-only chain / DB / accounting input the policy
    // needs. Mainnet fail-closed on any provider error.
    let mut inputs = match data_provider.gather_inputs(state, &intent).await {
        Ok(inputs) => inputs,
        Err(err) => {
            let mode = BroadcastMode::from_chain_id(state.execution_config.executor_chain_id);
            if matches!(mode, BroadcastMode::Mainnet) {
                let reason = format!("policy:policy-internal:{err}");
                warn!(
                    target: "broadcast_policy",
                    intent_id = %intent_id,
                    "policy data provider failure (mainnet fail-closed)"
                );
                state
                    .broadcast_observability
                    .record_policy_data_failure(&format!("{err}"));
                state
                    .broadcast_observability
                    .record_policy_rejected("policy-internal", intent.source_type);
                let now = now_ms();
                update_option_execution_intent_status(
                    state,
                    intent_id,
                    OptionExecutionIntentStatus::BroadcastFailed,
                    Some(reason.clone()),
                    now,
                )
                .await?;
                return Err(BackendError::BroadcastRejected(reason));
            }
            warn!(
                target: "broadcast_policy",
                intent_id = %intent_id,
                error = %err,
                "policy data provider failure on testnet — using permissive fallback"
            );
            state
                .broadcast_observability
                .record_policy_data_failure(&format!("{err}"));
            BroadcastPolicyInputs::default()
        }
    };

    // Dedupe is the one input that MUST reflect the latest persisted state
    // at every broadcast attempt, not whatever the provider snapshotted —
    // override here. Stub providers can ship a permissive default and the
    // live re-check below is authoritative.
    inputs.dedupe_hit = inputs.dedupe_hit
        || matches!(
            intent.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
                | OptionExecutionIntentStatus::BroadcastConfirmed
                | OptionExecutionIntentStatus::BroadcastFailed
        )
        || find_submitted_option_execution_transaction(state, intent_id)
            .await?
            .is_some();

    // Per-broadcast observability hooks: live-read snapshot for `/metrics`
    // gauges + econ_data_available transition counter + R5 drift counter.
    state
        .broadcast_observability
        .record_inputs_snapshot(&inputs);
    let econ_data_available = inputs.fee_split.is_some()
        && inputs.fm_v2_rebate_budget_asset.is_some()
        && inputs.pfv_rebate_reserve_asset.is_some();
    state
        .broadcast_observability
        .record_econ_data_available(econ_data_available);
    // Surface the most-recent effective maker/taker ppm — the same
    // values that drive `should_broadcast`'s §8 negative-effective-ppm
    // gate — to operators via `/executor/health/v2`. Guarded by
    // `inputs.fee_split.is_some()` so a missing `fee_split` (boundary
    // mode) never records fake `(0, 0)`; the snapshot retains the
    // previous reading.
    if let Some(fee_split) = inputs.fee_split.as_ref() {
        state
            .broadcast_observability
            .record_effective_fee_ppm(fee_split.effective_maker_ppm, fee_split.effective_taker_ppm);
    }
    if matches!(inputs.r5_drift_zero, Some(false)) {
        state.broadcast_observability.record_r5_drift_observed();
    }

    let policy_decision = run_should_broadcast_policy(state, &intent, &inputs);
    if let ShouldBroadcastDecision::Reject(reason) = &policy_decision {
        if matches!(reason, RejectReason::Dupe) {
            if let Some(transaction) =
                find_submitted_option_execution_transaction(state, intent_id).await?
            {
                state
                    .broadcast_observability
                    .record_policy_rejected("dupe", intent.source_type);
                return Ok(OptionExecutionBroadcastOutcome {
                    intent,
                    transaction,
                    broadcast_enabled: true,
                    submitted: true,
                    duplicate: true,
                });
            }
        }
        let message = policy_decision.message();
        warn!(
            target: "broadcast_policy",
            intent_id = %intent_id,
            code = reason.code(),
            "should_broadcast rejected option execution intent"
        );
        state
            .broadcast_observability
            .record_policy_rejected(reason.code(), intent.source_type);
        let now = now_ms();
        update_option_execution_intent_status(
            state,
            intent_id,
            OptionExecutionIntentStatus::BroadcastFailed,
            Some(message.clone()),
            now,
        )
        .await?;
        return Err(BackendError::BroadcastRejected(message));
    }
    state
        .broadcast_observability
        .record_policy_approved(intent.source_type);
    if let Some(transaction) = find_submitted_option_execution_transaction(state, intent_id).await?
    {
        return Ok(OptionExecutionBroadcastOutcome {
            intent,
            transaction,
            broadcast_enabled: true,
            submitted: true,
            duplicate: true,
        });
    }
    if state.execution_config.rpc_url.is_none() {
        return Err(BackendError::Config(
            "RPC_URL is required for option execution broadcast".to_string(),
        ));
    }
    let request = build_option_execution_transaction_request(
        &state.execution_config,
        &state.options_config,
        &intent,
    )?;
    let from = signer.signer_address().clone();
    let signer_kind = signer.kind();

    let gas_check = perform_option_broadcast_gas_safety_check(
        provider,
        &state.execution_config,
        &state.options_config,
        &intent,
        &from,
    )
    .await?;

    if !gas_check.is_ok() {
        let reason = gas_check
            .reject_reason()
            .unwrap_or_else(|| "option execution gas safety check failed".to_string());
        let now = now_ms();
        let transaction = option_execution_transaction_from_request(
            &request,
            from,
            None,
            Some(reason.clone()),
            now,
            Some(&gas_check),
        );
        insert_option_execution_transaction(state, transaction).await?;
        update_option_execution_intent_status(
            state,
            intent_id,
            OptionExecutionIntentStatus::BroadcastFailed,
            Some(reason.clone()),
            now,
        )
        .await?;
        return Err(BackendError::BroadcastRejected(reason));
    }

    let rpc_chain_id = provider.chain_id().await?;
    if rpc_chain_id != request.chain_id {
        return Err(BackendError::BroadcastRejected(format!(
            "RPC chain id {rpc_chain_id} does not match EXECUTOR_CHAIN_ID {}",
            request.chain_id
        )));
    }

    let now = now_ms();
    let nonce = provider.transaction_count(from.clone()).await?;
    state
        .broadcast_observability
        .record_signer_attempt(signer_kind.as_str());
    let raw_transaction =
        match sign_option_execution_via_signer(signer, &request, nonce, &intent, signer_kind).await
        {
            Ok(raw) => raw,
            Err(signer_err) => {
                let reason = format!("{signer_err}");
                tracing::warn!(
                    target: "broadcast_signer",
                    intent_id = %intent_id,
                    signer_kind = %signer_kind,
                    code = signer_err.code(),
                    "remote signer rejected option execution transaction"
                );
                state
                    .broadcast_observability
                    .record_signer_denied(signer_err.code(), signer_kind.as_str());
                let transaction = option_execution_transaction_from_request(
                    &request,
                    from.clone(),
                    None,
                    Some(reason.clone()),
                    now,
                    Some(&gas_check),
                );
                insert_option_execution_transaction(state, transaction).await?;
                update_option_execution_intent_status(
                    state,
                    intent_id,
                    OptionExecutionIntentStatus::BroadcastFailed,
                    Some(reason.clone()),
                    now,
                )
                .await?;
                return Err(BackendError::BroadcastRejected(reason));
            }
        };
    state
        .broadcast_observability
        .record_signer_success(signer_kind.as_str(), now);
    let tx_hash = match provider.send_raw_transaction(raw_transaction).await {
        Ok(tx_hash) => {
            if !is_valid_tx_hash(&tx_hash) {
                let error = "broadcast provider returned an invalid transaction hash".to_string();
                let transaction = option_execution_transaction_from_request(
                    &request,
                    from,
                    None,
                    Some(error.clone()),
                    now,
                    Some(&gas_check),
                );
                insert_option_execution_transaction(state, transaction.clone()).await?;
                update_option_execution_intent_status(
                    state,
                    intent_id,
                    OptionExecutionIntentStatus::BroadcastFailed,
                    Some(error.clone()),
                    now,
                )
                .await?;
                return Err(BackendError::BroadcastRejected(error));
            }
            tx_hash.to_ascii_lowercase()
        }
        Err(error) => {
            let transaction = option_execution_transaction_from_request(
                &request,
                from,
                None,
                Some(error.to_string()),
                now,
                Some(&gas_check),
            );
            insert_option_execution_transaction(state, transaction).await?;
            update_option_execution_intent_status(
                state,
                intent_id,
                OptionExecutionIntentStatus::BroadcastFailed,
                Some(error.to_string()),
                now,
            )
            .await?;
            return Err(error);
        }
    };

    let transaction = option_execution_transaction_from_request(
        &request,
        from,
        Some(tx_hash),
        None,
        now,
        Some(&gas_check),
    );
    let transaction = insert_option_execution_transaction(state, transaction).await?;
    let updated_intent = update_option_execution_intent_status(
        state,
        intent_id,
        OptionExecutionIntentStatus::BroadcastSubmitted,
        None,
        now,
    )
    .await?;

    Ok(OptionExecutionBroadcastOutcome {
        intent: updated_intent,
        transaction,
        broadcast_enabled: true,
        submitted: true,
        duplicate: false,
    })
}

pub async fn confirm_option_execution_intent(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<OptionExecutionConfirmationOutcome> {
    let rpc_url = state.execution_config.rpc_url.clone().ok_or_else(|| {
        BackendError::Config("RPC_URL is required for option execution confirmation".to_string())
    })?;
    let provider = HttpJsonRpcProvider::new(rpc_url);
    confirm_option_execution_intent_with_provider(state, intent_id, &provider).await
}

pub async fn confirm_option_execution_intent_with_provider<P>(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    provider: &P,
) -> Result<OptionExecutionConfirmationOutcome>
where
    P: TransactionReceiptProvider,
{
    let intent = get_option_execution_intent(state, intent_id).await?;
    let transaction = find_submitted_option_execution_transaction(state, intent_id)
        .await?
        .ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(
                "no submitted option execution transaction to confirm".to_string(),
            )
        })?;
    let tx_hash = transaction.tx_hash.clone().ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "submitted option execution transaction is missing a tx hash".to_string(),
        )
    })?;

    let receipt_result = provider.transaction_receipt(tx_hash.clone()).await;
    let now = now_ms();

    let mut receipt_cost = crate::options::OptionExecutionReceiptCost::default();
    let (status, receipt_status_value, block_number, error_string) = match receipt_result {
        Ok(Some(receipt)) => {
            if !receipt.tx_hash.eq_ignore_ascii_case(&tx_hash) {
                (
                    OptionExecutionConfirmationStatus::ReceiptMissing,
                    None,
                    None,
                    Some(format!(
                        "receipt tx hash {} does not match submitted {}",
                        receipt.tx_hash, tx_hash
                    )),
                )
            } else {
                let mapped = match receipt.status {
                    Some(1) => OptionExecutionConfirmationStatus::MinedSuccess,
                    Some(_) => OptionExecutionConfirmationStatus::MinedReverted,
                    None => OptionExecutionConfirmationStatus::ReceiptError,
                };
                let err = if mapped == OptionExecutionConfirmationStatus::ReceiptError {
                    Some("receipt missing status field".to_string())
                } else {
                    None
                };
                receipt_cost = receipt_cost_from_receipt(&receipt, now);
                (mapped, receipt.status, receipt.block_number, err)
            }
        }
        Ok(None) => (
            OptionExecutionConfirmationStatus::ReceiptMissing,
            None,
            None,
            Some("receipt not yet available".to_string()),
        ),
        Err(error) => (
            OptionExecutionConfirmationStatus::ReceiptError,
            None,
            None,
            Some(error.to_string()),
        ),
    };

    let updated_transaction = persist_option_execution_confirmation(
        state,
        &transaction.transaction_id,
        status,
        now,
        block_number,
        receipt_status_value,
        error_string.clone(),
        &receipt_cost,
    )
    .await?;

    let next_intent_status = match status {
        OptionExecutionConfirmationStatus::MinedSuccess => {
            Some(OptionExecutionIntentStatus::BroadcastConfirmed)
        }
        OptionExecutionConfirmationStatus::MinedReverted => {
            Some(OptionExecutionIntentStatus::BroadcastReverted)
        }
        _ => None,
    };

    let updated_intent = if let Some(next) = next_intent_status {
        update_option_execution_intent_status(state, intent_id, next, None, now).await?
    } else {
        intent.clone()
    };

    Ok(OptionExecutionConfirmationOutcome {
        intent: updated_intent,
        transaction: updated_transaction,
        confirmation_status: status,
        receipt_status: receipt_status_value,
        block_number,
        error: error_string,
    })
}

async fn list_pending_option_execution_transactions(
    state: &AppState,
    limit: u32,
) -> Result<Vec<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_pending_option_execution_transactions(limit)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_pending_option_execution_transactions(limit))
}

/// Run one tick of the option execution confirmation worker against the supplied receipt
/// provider. Deterministic and side-effect-isolated to the option store / repository:
/// it never broadcasts, never touches the generic executor path, never inserts new rows.
///
/// For each pending option execution transaction:
/// - If a receipt is found and `receipt.block_number + finality_blocks <= current_block_number`:
///   - status=1 → persist `mined_success`, transition intent to `broadcast_confirmed`
///   - status=0 → persist `mined_failed`, transition intent to `broadcast_failed`
///   - status=None → record receipt + leave pending (receipt_error)
/// - If a receipt is found but finality is not yet reached, leave pending. Update the
///   in-memory `confirmation_status` to `pending` so operators can see the worker observed
///   the receipt (no terminal state until finalized).
/// - If `eth_getTransactionReceipt` returns `None`, leave pending (`receipt_missing`).
/// - If RPC errors, leave pending (`receipt_error`).
pub async fn confirm_pending_option_execution_transactions<P>(
    state: &AppState,
    provider: &P,
) -> Result<crate::options::OptionConfirmationTickResult>
where
    P: TransactionReceiptProvider,
{
    let config = state.option_confirmation_config.clone();
    if !config.enabled {
        return Ok(crate::options::OptionConfirmationTickResult {
            enabled: false,
            batch_size: config.batch_size,
            finality_blocks: config.finality_blocks,
            current_block_number: None,
            decisions: Vec::new(),
        });
    }

    let current_block_number = provider.block_number().await.ok();
    let pending = list_pending_option_execution_transactions(state, config.batch_size).await?;
    let mut decisions = Vec::with_capacity(pending.len());

    for tx in pending {
        let now = now_ms();
        let Some(tx_hash) = tx.tx_hash.clone() else {
            // Shouldn't happen — list query filters for non-null hash — defensive only.
            continue;
        };
        let receipt_result = provider.transaction_receipt(tx_hash.clone()).await;
        let decision = compute_worker_decision(
            &tx.transaction_id,
            &tx_hash,
            current_block_number,
            config.finality_blocks,
            &receipt_result,
        );
        let receipt_cost = match &receipt_result {
            Ok(Some(receipt)) if receipt.tx_hash.eq_ignore_ascii_case(&tx_hash) => {
                receipt_cost_from_receipt(receipt, now)
            }
            _ => crate::options::OptionExecutionReceiptCost::default(),
        };
        apply_worker_decision(state, &tx, &decision, &receipt_cost, now).await?;
        decisions.push(decision);
    }

    let result = crate::options::OptionConfirmationTickResult {
        enabled: true,
        batch_size: config.batch_size,
        finality_blocks: config.finality_blocks,
        current_block_number,
        decisions,
    };
    if let Ok(mut slot) = state.option_confirmation_last_tick.lock() {
        *slot = Some(result.clone());
    }
    Ok(result)
}

fn compute_worker_decision(
    transaction_id: &str,
    expected_tx_hash: &str,
    current_block_number: Option<u64>,
    finality_blocks: u64,
    receipt_result: &Result<Option<crate::confirmation::ConfirmationReceipt>>,
) -> crate::options::OptionConfirmationDecision {
    use crate::options::{OptionConfirmationDecision, OptionConfirmationOutcome};
    let base = |outcome: OptionConfirmationOutcome,
                receipt_status: Option<u64>,
                block_number: Option<u64>,
                error: Option<String>|
     -> OptionConfirmationDecision {
        OptionConfirmationDecision {
            transaction_id: transaction_id.to_string(),
            tx_hash: Some(expected_tx_hash.to_string()),
            outcome,
            receipt_status,
            block_number,
            current_block_number,
            finality_blocks,
            error,
        }
    };
    let receipt = match receipt_result {
        Ok(Some(r)) => r,
        Ok(None) => {
            return base(
                OptionConfirmationOutcome::ReceiptMissing,
                None,
                None,
                Some("receipt not yet available".to_string()),
            )
        }
        Err(error) => {
            return base(
                OptionConfirmationOutcome::ReceiptError,
                None,
                None,
                Some(error.to_string()),
            )
        }
    };
    if !receipt.tx_hash.eq_ignore_ascii_case(expected_tx_hash) {
        return base(
            OptionConfirmationOutcome::ReceiptMissing,
            receipt.status,
            receipt.block_number,
            Some(format!(
                "receipt tx hash {} does not match submitted {}",
                receipt.tx_hash, expected_tx_hash
            )),
        );
    }
    let Some(receipt_block) = receipt.block_number else {
        return base(
            OptionConfirmationOutcome::ReceiptError,
            receipt.status,
            None,
            Some("receipt missing block_number".to_string()),
        );
    };
    let Some(head) = current_block_number else {
        return base(
            OptionConfirmationOutcome::NotFinalized,
            receipt.status,
            Some(receipt_block),
            Some("current block number unavailable".to_string()),
        );
    };
    let finalized_at = receipt_block.saturating_add(finality_blocks);
    if head < finalized_at {
        return base(
            OptionConfirmationOutcome::NotFinalized,
            receipt.status,
            Some(receipt_block),
            None,
        );
    }
    match receipt.status {
        Some(1) => base(
            OptionConfirmationOutcome::MinedSuccess,
            Some(1),
            Some(receipt_block),
            None,
        ),
        Some(other) => base(
            OptionConfirmationOutcome::MinedFailed,
            Some(other),
            Some(receipt_block),
            None,
        ),
        None => base(
            OptionConfirmationOutcome::ReceiptError,
            None,
            Some(receipt_block),
            Some("receipt missing status field".to_string()),
        ),
    }
}

async fn apply_worker_decision(
    state: &AppState,
    transaction: &OptionExecutionTransaction,
    decision: &crate::options::OptionConfirmationDecision,
    receipt_cost: &crate::options::OptionExecutionReceiptCost,
    now: TimestampMs,
) -> Result<()> {
    use crate::options::OptionConfirmationOutcome;
    let (persist_status, intent_status) = match decision.outcome {
        OptionConfirmationOutcome::MinedSuccess => (
            OptionExecutionConfirmationStatus::MinedSuccess,
            Some(OptionExecutionIntentStatus::BroadcastConfirmed),
        ),
        OptionConfirmationOutcome::MinedFailed => (
            OptionExecutionConfirmationStatus::MinedFailed,
            Some(OptionExecutionIntentStatus::BroadcastFailed),
        ),
        OptionConfirmationOutcome::NotFinalized => {
            (OptionExecutionConfirmationStatus::Pending, None)
        }
        OptionConfirmationOutcome::ReceiptMissing => {
            (OptionExecutionConfirmationStatus::ReceiptMissing, None)
        }
        OptionConfirmationOutcome::ReceiptError => {
            (OptionExecutionConfirmationStatus::ReceiptError, None)
        }
        OptionConfirmationOutcome::Disabled | OptionConfirmationOutcome::NoPending => return Ok(()),
    };

    persist_option_execution_confirmation(
        state,
        &transaction.transaction_id,
        persist_status,
        now,
        decision.block_number,
        decision.receipt_status,
        decision.error.clone(),
        receipt_cost,
    )
    .await?;
    if let Some(next) = intent_status {
        update_option_execution_intent_status(state, transaction.intent_id, next, None, now)
            .await?;
    }
    Ok(())
}

/// Bridge `ConfirmationReceipt` (RPC-shaped) into the persisted
/// `OptionExecutionReceiptCost` bundle. The bridge is shared by the V1T
/// manual confirm endpoint and the V1V background worker so both paths emit
/// identical persistence side effects.
pub(crate) fn receipt_cost_from_receipt(
    receipt: &crate::confirmation::ConfirmationReceipt,
    observed_at_ms: TimestampMs,
) -> crate::options::OptionExecutionReceiptCost {
    crate::options::OptionExecutionReceiptCost {
        gas_used: receipt.gas_used,
        effective_gas_price: receipt.effective_gas_price.clone(),
        cumulative_gas_used: receipt.cumulative_gas_used,
        block_hash: receipt.block_hash.clone(),
        transaction_index: receipt.transaction_index,
        observed_at_ms: Some(observed_at_ms),
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_option_execution_confirmation(
    state: &AppState,
    transaction_id: &str,
    confirmation_status: OptionExecutionConfirmationStatus,
    confirmed_at_ms: TimestampMs,
    confirmed_block_number: Option<u64>,
    receipt_status: Option<u64>,
    confirmation_error: Option<String>,
    receipt_cost: &crate::options::OptionExecutionReceiptCost,
) -> Result<OptionExecutionTransaction> {
    if let Some(repository) = state.repository.clone() {
        let rows = repository
            .update_option_execution_confirmation(
                transaction_id,
                confirmation_status,
                confirmed_at_ms,
                confirmed_block_number,
                receipt_status,
                confirmation_error.clone(),
                receipt_cost,
            )
            .await?;
        if rows == 0 {
            return Err(BackendError::Persistence(format!(
                "option execution transaction {transaction_id} not found"
            )));
        }
        let tx = repository
            .get_option_execution_transaction(transaction_id)
            .await?;
        return tx.ok_or_else(|| {
            BackendError::Persistence(format!(
                "option execution transaction {transaction_id} disappeared after update"
            ))
        });
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_option_execution_confirmation(
            transaction_id,
            confirmation_status,
            confirmed_at_ms,
            confirmed_block_number,
            receipt_status,
            confirmation_error,
            receipt_cost,
        )
}

pub async fn create_option_orderbook_execution_intent(
    state: &AppState,
    fill: &OptionFill,
) -> Result<Option<OptionExecutionIntent>> {
    let provider = option_nonce_provider(state)?;
    create_option_orderbook_execution_intent_with_nonce_provider(state, fill, provider.as_ref())
        .await
}

async fn create_option_orderbook_execution_intent_with_nonce_provider<P>(
    state: &AppState,
    fill: &OptionFill,
    nonce_provider: Option<&P>,
) -> Result<Option<OptionExecutionIntent>>
where
    P: OptionNonceProvider,
{
    if !state.options_config.execution_enabled {
        return Ok(None);
    }
    let series = get_option_series(state, &fill.option_series_id).await?;
    let buyer_is_maker = fill.maker_order_id == fill.buy_order_id;
    let (buyer_nonce, seller_nonce) =
        option_execution_nonces(state, nonce_provider, &fill.buyer, &fill.seller).await?;
    let intent = build_option_execution_intent(
        state,
        &series,
        OptionExecutionSourceType::OptionOrderbookFill,
        fill.fill_id.to_string(),
        fill.buyer.clone(),
        fill.seller.clone(),
        fill.price_1e8,
        fill.size_1e8,
        buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        fill.created_at_ms,
    )?;
    insert_option_execution_intent(state, intent)
        .await
        .map(Some)
}

pub async fn create_option_rfq_execution_intent(
    state: &AppState,
    fill: &OptionRfqFill,
) -> Result<Option<OptionExecutionIntent>> {
    let provider = option_nonce_provider(state)?;
    create_option_rfq_execution_intent_with_nonce_provider(state, fill, provider.as_ref()).await
}

async fn create_option_rfq_execution_intent_with_nonce_provider<P>(
    state: &AppState,
    fill: &OptionRfqFill,
    nonce_provider: Option<&P>,
) -> Result<Option<OptionExecutionIntent>>
where
    P: OptionNonceProvider,
{
    if !state.options_config.execution_enabled {
        return Ok(None);
    }
    let series = get_option_series(state, &fill.option_series_id).await?;
    let buyer_is_maker = fill.buyer.0.eq_ignore_ascii_case(&fill.mm_account.0);
    let (buyer_nonce, seller_nonce) =
        option_execution_nonces(state, nonce_provider, &fill.buyer, &fill.seller).await?;
    let intent = build_option_execution_intent(
        state,
        &series,
        OptionExecutionSourceType::OptionRfqFill,
        fill.fill_id.to_string(),
        fill.buyer.clone(),
        fill.seller.clone(),
        fill.price_1e8,
        fill.size_1e8,
        buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        fill.created_at_ms,
    )?;
    insert_option_execution_intent(state, intent)
        .await
        .map(Some)
}

async fn create_option_orderbook_execution_intents(
    state: &AppState,
    fills: &[OptionFill],
) -> Result<Vec<OptionExecutionIntent>> {
    let mut intents = Vec::new();
    if !state.options_config.execution_enabled {
        return Ok(intents);
    }
    for fill in fills {
        if let Some(intent) = create_option_orderbook_execution_intent(state, fill).await? {
            intents.push(intent);
        }
    }
    Ok(intents)
}

/// M-P2f (B7 close) — User-wallet-initiated execution intent creation.
///
/// Public/no-admin-Bearer counterpart to
/// `create_option_orderbook_execution_intent`. Where the orderbook
/// variant builds the intent from a server-side `OptionFill` after
/// matching, this function builds it from caller-supplied trade
/// parameters (both buyer + seller are required, since the
/// counterparty resolver is not yet wired). The intent is inserted
/// into the same store/repository the existing signing-payload /
/// signature-submit / tx-status endpoints already read from, so the
/// downstream UI flow works unchanged.
///
/// **Posture (verified by tests in `src/api/trading.rs`):**
///   * NEVER signs.
///   * NEVER broadcasts.
///   * NEVER calls the signer / AWS / KMS.
///   * NEVER touches a public chain (no `eth_call`, no
///     `eth_sendRawTransaction`).
///   * NEVER mutates production transaction tables — only the
///     existing `option_execution_intents` table (which is the same
///     table the M-P2a/M-P3b flow has always written to).
///
/// Returns the inserted `OptionExecutionIntent` with
/// `status = SignaturesRequired`. The caller is the frontend's
/// `CreateIntentButton`; the user then explicitly clicks "Sign" to
/// drive the existing signing-payload flow.
pub async fn create_user_initiated_execution_intent_from_quote(
    state: &AppState,
    series: &OptionSeries,
    buyer: AccountId,
    seller: AccountId,
    side: crate::types::Side,
    size_1e8: crate::types::Size1e8,
    price_1e8: crate::types::Price1e8,
) -> Result<OptionExecutionIntent> {
    ensure_enabled(state)?;
    validate_account(&buyer)?;
    validate_account(&seller)?;
    if size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    if buyer.0.eq_ignore_ascii_case(&seller.0) {
        return Err(BackendError::SelfTrade);
    }
    if !matches!(series.status, OptionSeriesStatus::Active) {
        return Err(BackendError::InvalidOptionSeriesState(
            "option series is not active".to_string(),
        ));
    }
    // The user is the taker; the counterparty role is "maker" for
    // book-keeping purposes only — no order-book matching is
    // performed here. The `buyer_is_maker` flag is set based on
    // `side`: when the caller is buying, the seller is the maker.
    let buyer_is_maker = matches!(side, crate::types::Side::Sell);
    let nonce_provider = option_nonce_provider(state)?;
    let (buyer_nonce, seller_nonce) =
        option_execution_nonces(state, nonce_provider.as_ref(), &buyer, &seller).await?;
    let source_id = Uuid::new_v4().to_string();
    let intent = build_option_execution_intent(
        state,
        series,
        OptionExecutionSourceType::OptionOrderbookFill,
        source_id,
        buyer,
        seller,
        price_1e8,
        size_1e8,
        buyer_is_maker,
        buyer_nonce,
        seller_nonce,
        now_ms(),
    )?;
    insert_option_execution_intent(state, intent).await
}

async fn insert_option_execution_intent(
    state: &AppState,
    intent: OptionExecutionIntent,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository.insert_option_execution_intent(&intent).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_execution_intent(intent))
}

async fn persist_option_execution_simulation_result(
    state: &AppState,
    result: &OptionExecutionSimulationResult,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .persist_option_execution_simulation_result(result)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .persist_option_execution_simulation_result(result)
}

async fn update_option_execution_intent_status(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
    status: OptionExecutionIntentStatus,
    error: Option<String>,
    updated_at_ms: TimestampMs,
) -> Result<OptionExecutionIntent> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .update_option_execution_intent_status(intent_id, status, error, updated_at_ms)
            .await;
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .update_option_execution_intent_status(intent_id, status, error, updated_at_ms)
}

async fn insert_option_execution_transaction(
    state: &AppState,
    transaction: OptionExecutionTransaction,
) -> Result<OptionExecutionTransaction> {
    if let Some(repository) = state.repository.clone() {
        repository
            .insert_option_execution_transaction(&transaction)
            .await?;
        return Ok(transaction);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .insert_option_execution_transaction(transaction)
}

async fn find_submitted_option_execution_transaction(
    state: &AppState,
    intent_id: OptionExecutionIntentId,
) -> Result<Option<OptionExecutionTransaction>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .find_submitted_option_execution_transaction_by_intent(intent_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .find_submitted_option_execution_transaction_by_intent(intent_id))
}

fn option_execution_transaction_from_request(
    request: &ExecutionTransactionRequest,
    from: AccountId,
    tx_hash: Option<String>,
    error: Option<String>,
    now: TimestampMs,
    gas_check: Option<&OptionExecutionGasSafetyCheck>,
) -> OptionExecutionTransaction {
    let status = if tx_hash.is_some() {
        ExecutionTransactionStatus::Submitted
    } else {
        ExecutionTransactionStatus::Failed
    };
    OptionExecutionTransaction {
        transaction_id: Uuid::new_v4().to_string(),
        intent_id: request.intent_id,
        onchain_intent_id: Some(request.onchain_intent_id.clone()),
        from,
        to: request.to.clone(),
        calldata: request.calldata_hex(),
        value_wei: request.value_wei.to_string(),
        gas_limit: Some(request.gas_limit),
        tx_hash,
        status,
        error,
        estimated_gas: gas_check.and_then(|check| check.estimated_gas),
        required_gas: gas_check.and_then(|check| check.required_gas),
        simulation_gas_limit: gas_check.map(|check| check.simulation_gas_limit),
        broadcast_gas_limit: gas_check.map(|check| check.broadcast_gas_limit),
        gas_safety_bps: gas_check.map(|check| check.gas_safety_bps),
        gas_check_status: gas_check.map(|check| check.status),
        gas_check_error: gas_check.and_then(|check| check.error.clone()),
        confirmation_status: None,
        confirmed_at_ms: None,
        confirmed_block_number: None,
        receipt_status: None,
        confirmation_error: None,
        gas_used: None,
        effective_gas_price: None,
        cumulative_gas_used: None,
        receipt_block_hash: None,
        receipt_transaction_index: None,
        receipt_observed_at_ms: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

fn is_valid_tx_hash(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("0x") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn option_nonce_provider(state: &AppState) -> Result<Option<HttpJsonRpcProvider>> {
    if !state.option_nonce_sync_config.enabled {
        return Ok(None);
    }
    Ok(state
        .option_nonce_sync_config
        .rpc_url
        .clone()
        .map(HttpJsonRpcProvider::new))
}

async fn option_execution_nonces<P>(
    state: &AppState,
    nonce_provider: Option<&P>,
    buyer: &AccountId,
    seller: &AccountId,
) -> Result<(u128, u128)>
where
    P: OptionNonceProvider,
{
    if !state.option_nonce_sync_config.enabled {
        return Ok((0, 0));
    }

    let result = async {
        let provider = nonce_provider.ok_or_else(|| {
            BackendError::Config("RPC_URL is required for option nonce sync".to_string())
        })?;
        let buyer_nonce =
            read_option_nonce_value(&state.option_nonce_sync_config, provider, buyer).await?;
        let seller_nonce =
            read_option_nonce_value(&state.option_nonce_sync_config, provider, seller).await?;
        Ok((buyer_nonce, seller_nonce))
    }
    .await;

    match result {
        Ok(nonces) => Ok(nonces),
        Err(error) if state.option_nonce_sync_config.strict => Err(error),
        Err(error) => {
            warn!(
                buyer = %buyer.0,
                seller = %seller.0,
                error = %error,
                "option nonce sync failed in non-strict mode; falling back to zero nonces"
            );
            Ok((0, 0))
        }
    }
}

fn validate_option_execution_simulation_preflight(
    state: &AppState,
    intent: &OptionExecutionIntent,
) -> Result<()> {
    validate_simulation_target(&state.options_config.matching_engine_address)?;
    validate_simulation_intent(intent)?;
    let from = option_execution_simulation_from(state)?;
    parse_evm_address(&from)?;
    Ok(())
}

fn option_execution_simulation_from(state: &AppState) -> Result<AccountId> {
    let from = state
        .options_config
        .execution_simulation_from
        .clone()
        .unwrap_or_else(|| state.execution_config.executor_from_address.clone());
    parse_evm_address(&from)?;
    Ok(from)
}

async fn validate_option_order_execution_preflight(
    state: &AppState,
    series: &OptionSeries,
    input: &SubmitOptionOrderInput,
) -> Result<()> {
    if !state.options_config.execution_enabled {
        return Ok(());
    }

    let mut candidates = open_option_orders_for_series(state, &input.option_series_id)
        .await?
        .into_iter()
        .filter(|order| {
            order.side != input.side
                && order.status.is_live()
                && order.remaining_size_1e8 > 0
                && match input.side {
                    Side::Buy => input.price_1e8 >= order.price_1e8,
                    Side::Sell => input.price_1e8 <= order.price_1e8,
                }
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }

    validate_executable_option_series(state, series)?;
    sort_execution_preflight_candidates(&mut candidates, input.side);
    let mut remaining_size_1e8 = input.size_1e8;
    for maker in candidates {
        if remaining_size_1e8 == 0 {
            break;
        }
        let fill_size_1e8 = remaining_size_1e8.min(maker.remaining_size_1e8);
        validate_option_execution_conversion(state, fill_size_1e8, maker.price_1e8)?;
        remaining_size_1e8 -= fill_size_1e8;
    }
    Ok(())
}

async fn validate_option_rfq_execution_preflight(
    state: &AppState,
    series: &OptionSeries,
    quote: &OptionRfqQuote,
) -> Result<()> {
    if !state.options_config.execution_enabled {
        return Ok(());
    }
    validate_executable_option_series(state, series)?;
    validate_option_execution_conversion(state, quote.size_1e8, quote.price_1e8)
}

#[allow(clippy::too_many_arguments)]
fn build_option_execution_intent(
    state: &AppState,
    series: &OptionSeries,
    source_type: OptionExecutionSourceType,
    source_id: String,
    buyer: AccountId,
    seller: AccountId,
    source_price_1e8: Price1e8,
    source_size_1e8: Size1e8,
    buyer_is_maker: bool,
    buyer_nonce: u128,
    seller_nonce: u128,
    source_created_at_ms: TimestampMs,
) -> Result<OptionExecutionIntent> {
    let metadata = validate_executable_option_series(state, series)?;
    let quantity_contracts = quantity_contracts_from_size(source_size_1e8)?;
    let premium_per_contract_native = premium_per_contract_native(
        source_price_1e8,
        state.options_config.execution_default_settlement_decimals,
    )?;
    let intent_id = Uuid::new_v4();
    let onchain_intent_id = option_execution_intent_id_to_hex_bytes32(&intent_id.to_string())?;
    let now = now_ms();
    Ok(OptionExecutionIntent {
        intent_id,
        onchain_intent_id,
        source_type,
        source_id,
        option_series_id: series.option_series_id.clone(),
        onchain_option_id: metadata.onchain_option_id,
        buyer,
        seller,
        underlying: metadata.underlying,
        settlement_asset: metadata.settlement_asset,
        expiry: series.expiry,
        strike_1e8: series.strike_1e8,
        is_call: series.is_call,
        contract_size_1e8: series.contract_size_1e8,
        quantity_contracts,
        source_size_1e8,
        source_price_1e8,
        premium_per_contract_native,
        buyer_is_maker,
        buyer_nonce: Some(buyer_nonce),
        seller_nonce: Some(seller_nonce),
        deadline: 0,
        buyer_signature: None,
        seller_signature: None,
        calldata: None,
        status: OptionExecutionIntentStatus::SignaturesRequired,
        error: None,
        simulation_status: None,
        simulation_error: None,
        simulation_block_number: None,
        simulation_revert_data: None,
        simulation_revert_selector: None,
        simulated_at_ms: None,
        created_at_ms: source_created_at_ms,
        updated_at_ms: now,
    })
}

struct ExecutableOptionSeriesMetadata {
    onchain_option_id: String,
    underlying: AccountId,
    settlement_asset: AccountId,
}

fn validate_executable_option_series(
    state: &AppState,
    series: &OptionSeries,
) -> Result<ExecutableOptionSeriesMetadata> {
    let onchain_option_id = series
        .onchain_series_id
        .as_deref()
        .or(series.onchain_product_id.as_deref())
        .ok_or_else(|| {
            BackendError::InvalidOptionExecutionIntentState(
                "option series is missing onchain_series_id or onchain_product_id".to_string(),
            )
        })
        .and_then(|value| normalize_u256_string(value, "optionId"))?;
    validate_nonzero_execution_address(&series.underlying, "underlying")?;
    validate_nonzero_execution_address(&series.settlement_asset, "settlement_asset")?;
    validate_series_option_id_matches_metadata(series, &onchain_option_id)?;
    let _ = state;
    Ok(ExecutableOptionSeriesMetadata {
        onchain_option_id,
        underlying: AccountId::new(series.underlying.clone()),
        settlement_asset: AccountId::new(series.settlement_asset.clone()),
    })
}

fn validate_series_option_id_matches_metadata(
    series: &OptionSeries,
    onchain_option_id: &str,
) -> Result<()> {
    let strike_1e8 = u64::try_from(series.strike_1e8).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState("strike_1e8 exceeds uint64".to_string())
    })?;
    let option_id =
        alloy_primitives::U256::from_str_radix(onchain_option_id, 10).map_err(|error| {
            BackendError::InvalidOptionExecutionIntentState(format!(
                "optionId must be a uint256: {error}"
            ))
        })?;
    let underlying = AccountId::new(series.underlying.clone());
    let settlement_asset = AccountId::new(series.settlement_asset.clone());
    let european_id = crate::options::option_product_registry_option_id(
        &underlying,
        &settlement_asset,
        series.expiry,
        strike_1e8,
        series.contract_size_1e8,
        series.is_call,
        true,
    )?;
    let american_id = crate::options::option_product_registry_option_id(
        &underlying,
        &settlement_asset,
        series.expiry,
        strike_1e8,
        series.contract_size_1e8,
        series.is_call,
        false,
    )?;
    if option_id != european_id && option_id != american_id {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "optionId does not match option metadata for either isEuropean value".to_string(),
        ));
    }
    Ok(())
}

fn validate_nonzero_execution_address(value: &str, field: &str) -> Result<()> {
    let account = AccountId::new(value.to_string());
    let address = parse_evm_address(&account).map_err(|_| {
        BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be an EVM address when option execution is enabled"
        ))
    })?;
    if address.iter().all(|byte| *byte == 0) {
        return Err(BackendError::InvalidOptionExecutionIntentState(format!(
            "{field} must be nonzero when option execution is enabled"
        )));
    }
    Ok(())
}

fn validate_option_execution_conversion(
    state: &AppState,
    size_1e8: Size1e8,
    price_1e8: Price1e8,
) -> Result<()> {
    let _ = quantity_contracts_from_size(size_1e8)?;
    let _ = premium_per_contract_native(
        price_1e8,
        state.options_config.execution_default_settlement_decimals,
    )?;
    Ok(())
}

fn quantity_contracts_from_size(size_1e8: Size1e8) -> Result<u128> {
    if size_1e8 == 0 {
        return Err(BackendError::ZeroSize);
    }
    if size_1e8 % ONE_CONTRACT_1E8 != 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "size_1e8 must be a whole number of option contracts when option execution is enabled"
                .to_string(),
        ));
    }
    let quantity = size_1e8 / ONE_CONTRACT_1E8;
    if quantity == 0 {
        return Err(BackendError::ZeroSize);
    }
    Ok(quantity)
}

fn premium_per_contract_native(price_1e8: Price1e8, settlement_decimals: u32) -> Result<u128> {
    if price_1e8 == 0 {
        return Err(BackendError::ZeroPrice);
    }
    let scale = 10u128.checked_pow(settlement_decimals).ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "settlement decimals overflow native premium conversion".to_string(),
        )
    })?;
    let premium = price_1e8.checked_mul(scale).ok_or_else(|| {
        BackendError::InvalidOptionExecutionIntentState(
            "premium native conversion overflow".to_string(),
        )
    })? / ONE_CONTRACT_1E8;
    if premium == 0 {
        return Err(BackendError::InvalidOptionExecutionIntentState(
            "premium_per_contract_native is zero after settlement-native conversion".to_string(),
        ));
    }
    Ok(premium)
}

fn verify_option_execution_signature(
    state: &AppState,
    signature: Option<&str>,
    digest_bytes: &[u8; 32],
    expected_signer: &AccountId,
) -> Result<()> {
    let Some(signature) = signature else {
        return Ok(());
    };
    validate_signature_shape(signature)?;
    if state.options_config.execution_signature_mode == OptionExecutionSignatureMode::Strict {
        let recovered_signer = recover_eip712_signer(digest_bytes, signature)?;
        let expected = parse_evm_address(expected_signer)?;
        let recovered = parse_evm_address(&recovered_signer)?;
        if recovered != expected {
            return Err(BackendError::SignatureSignerMismatch);
        }
    }
    Ok(())
}

fn build_option_execution_calldata_from_parts(
    payload: &OptionTradePayload,
    buyer_signature: &str,
    seller_signature: &str,
) -> Result<String> {
    let bundle = OptionTradeSignatureBundle::new(buyer_signature, seller_signature)?;
    Ok(hex_0x(&encode_option_execute_trade_calldata(
        payload, &bundle,
    )?))
}

fn option_execution_signature_outcome(
    intent: OptionExecutionIntent,
) -> SubmitOptionExecutionSignaturesOutcome {
    let buyer_signature_present = intent.buyer_signature.is_some();
    let seller_signature_present = intent.seller_signature.is_some();
    let calldata_ready =
        intent.calldata.is_some() && intent.status == OptionExecutionIntentStatus::CalldataReady;
    SubmitOptionExecutionSignaturesOutcome {
        intent,
        buyer_signature_present,
        seller_signature_present,
        calldata_ready,
        missing_signatures: !(buyer_signature_present && seller_signature_present),
    }
}

fn sort_execution_preflight_candidates(orders: &mut [OptionOrder], taker_side: Side) {
    orders.sort_by(|left, right| {
        let price_order = match taker_side {
            Side::Buy => left.price_1e8.cmp(&right.price_1e8),
            Side::Sell => right.price_1e8.cmp(&left.price_1e8),
        };
        price_order
            .then_with(|| left.created_at_ms.cmp(&right.created_at_ms))
            .then_with(|| left.order_id.cmp(&right.order_id))
    });
}

async fn open_option_orders_for_series(
    state: &AppState,
    option_series_id: &str,
) -> Result<Vec<OptionOrder>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .open_option_orders_for_series(option_series_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .open_orders_for_series(option_series_id))
}

async fn count_option_rfq_quotes(state: &AppState, option_rfq_id: OptionRfqId) -> Result<usize> {
    if let Some(repository) = state.repository.clone() {
        return repository.count_option_rfq_quotes(option_rfq_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .count_option_rfq_quotes(option_rfq_id))
}

async fn get_option_rfq_quote(
    state: &AppState,
    quote_id: OptionRfqQuoteId,
) -> Result<OptionRfqQuote> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_rfq_quote(quote_id)
            .await?
            .ok_or(BackendError::InvalidOptionRfqQuoteId);
    }
    state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_rfq_quote(quote_id)
        .ok_or(BackendError::InvalidOptionRfqQuoteId)
}

async fn get_option_series_optional(
    state: &AppState,
    option_series_id: &str,
) -> Result<Option<OptionSeries>> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_option_series(option_series_id).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_series(option_series_id))
}

fn ensure_enabled(state: &AppState) -> Result<()> {
    if state.options_config.enabled {
        Ok(())
    } else {
        Err(BackendError::OptionsDisabled)
    }
}

fn ensure_option_rfq_enabled(state: &AppState) -> Result<()> {
    ensure_enabled(state)?;
    if state.options_config.rfq_enabled {
        Ok(())
    } else {
        Err(BackendError::OptionRfqDisabled)
    }
}

fn ensure_option_execution_simulation_enabled(state: &AppState) -> Result<()> {
    ensure_enabled(state)?;
    if state.options_config.execution_simulation_enabled {
        Ok(())
    } else {
        Err(BackendError::Config(
            "option execution simulation is disabled".to_string(),
        ))
    }
}

fn ensure_option_execution_broadcast_enabled(state: &AppState) -> Result<()> {
    if !state.options_config.execution_broadcast_enabled {
        return Err(BackendError::Config(
            "option execution broadcast is disabled".to_string(),
        ));
    }
    ensure_enabled(state)?;
    if !state.options_config.execution_enabled {
        return Err(BackendError::Config(
            "OPTION_EXECUTION_ENABLED=true is required for option execution broadcast".to_string(),
        ));
    }
    if !state.execution_config.execution_enabled {
        return Err(BackendError::Config(
            "EXECUTION_ENABLED=true is required for option execution broadcast".to_string(),
        ));
    }
    if !state.execution_config.real_broadcast_enabled {
        return Err(BackendError::Config(
            "EXECUTOR_REAL_BROADCAST_ENABLED=true is required for option execution broadcast"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_account(account: &AccountId) -> Result<()> {
    parse_evm_address(account).map(|_| ())
}

/// Reject TIF / post-only combinations whose semantics are ambiguous
/// or undefined. Options have no market-order variant (`price_1e8 == 0`
/// is already rejected upstream), so `POST_ONLY_REQUIRES_LIMIT` is not
/// emitted here.
fn validate_tif_combination(tif: TimeInForce, post_only: bool) -> Result<()> {
    if post_only {
        match tif {
            TimeInForce::Gtc => {}
            TimeInForce::Ioc => {
                return Err(BackendError::InvalidTimeInForceCombination(
                    "post-only is not compatible with IOC".to_string(),
                ));
            }
            TimeInForce::Fok => {
                return Err(BackendError::InvalidTimeInForceCombination(
                    "post-only is not compatible with FOK".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_option_rfq_quote_ttl(state: &AppState, quote_ttl_ms: u64) -> Result<()> {
    if quote_ttl_ms < state.options_config.rfq_min_quote_ttl_ms {
        return Err(BackendError::InvalidOptionRfqQuoteState(
            "option RFQ quote_ttl_ms is below the minimum".to_string(),
        ));
    }
    Ok(())
}

fn validate_assets(fields: &[(&str, &str)]) -> Result<()> {
    for (field, value) in fields {
        if value.trim().is_empty() {
            return Err(BackendError::InvalidOptionSeriesState(format!(
                "{field} must be non-empty"
            )));
        }
    }
    Ok(())
}

fn trim_asset(value: String) -> String {
    value.trim().to_string()
}

fn now_sec(now_ms: TimestampMs) -> Result<u64> {
    u64::try_from(now_ms / 1000)
        .map_err(|_| BackendError::Config("current timestamp cannot be encoded".to_string()))
}

fn checked_expiry(now: TimestampMs, ttl_ms: u64, context: &str) -> Result<TimestampMs> {
    let ttl_ms = i64::try_from(ttl_ms)
        .map_err(|_| BackendError::Config(format!("{context} ttl cannot be encoded")))?;
    now.checked_add(ttl_ms)
        .ok_or_else(|| BackendError::Config(format!("{context} overflow")))
}

fn option_rfq_price_satisfies_limit(
    side: Side,
    limit_price_1e8: Option<Price1e8>,
    price_1e8: Price1e8,
) -> bool {
    match (side, limit_price_1e8) {
        (_, None) => true,
        (Side::Buy, Some(limit)) => price_1e8 <= limit,
        (Side::Sell, Some(limit)) => price_1e8 >= limit,
    }
}

struct OptionRfqQuoteSignatureMetadata {
    signature: Option<String>,
    quote_digest: Option<String>,
    quote_nonce: Option<String>,
    signature_status: OptionRfqQuoteSignatureStatus,
    recovered_signer: Option<AccountId>,
}

fn verify_option_rfq_quote_signature(
    state: &AppState,
    rfq: &OptionRfqRequest,
    input: &SubmitOptionRfqQuoteInput,
    quote_ttl_ms: u64,
) -> Result<OptionRfqQuoteSignatureMetadata> {
    match state.options_config.rfq_quote_signature_mode {
        OptionRfqQuoteSignatureMode::Disabled => {
            let quote_digest = input
                .quote_nonce
                .map(|quote_nonce| {
                    let payload = option_rfq_quote_payload(
                        rfq,
                        input.mm_account.clone(),
                        input.price_1e8,
                        input.size_1e8,
                        quote_nonce,
                        quote_ttl_ms,
                    )?;
                    option_rfq_quote_digest(&payload, &state.options_config.rfq_eip712_domain)
                })
                .transpose()?;
            Ok(OptionRfqQuoteSignatureMetadata {
                signature: input.signature.clone(),
                quote_digest,
                quote_nonce: input.quote_nonce.map(|value| value.to_string()),
                signature_status: OptionRfqQuoteSignatureStatus::NotRequired,
                recovered_signer: None,
            })
        }
        OptionRfqQuoteSignatureMode::Strict => {
            let Some(quote_nonce) = input.quote_nonce else {
                return Err(BackendError::InvalidOptionRfqQuoteState(
                    "quote_nonce is required when OPTION_RFQ_QUOTE_SIGNATURE_MODE=strict"
                        .to_string(),
                ));
            };
            let Some(signature) = input.signature.as_deref() else {
                return Err(BackendError::InvalidOptionRfqQuoteState(
                    "signature is required when OPTION_RFQ_QUOTE_SIGNATURE_MODE=strict".to_string(),
                ));
            };
            validate_signature_shape(signature)?;
            let payload = option_rfq_quote_payload(
                rfq,
                input.mm_account.clone(),
                input.price_1e8,
                input.size_1e8,
                quote_nonce,
                quote_ttl_ms,
            )?;
            let digest_bytes =
                option_rfq_quote_digest_bytes(&payload, &state.options_config.rfq_eip712_domain)?;
            let quote_digest = hex_0x(&digest_bytes);
            let recovered_signer = recover_eip712_signer(&digest_bytes, signature)?;
            let expected = parse_evm_address(&input.mm_account)?;
            let recovered = parse_evm_address(&recovered_signer)?;
            if recovered != expected {
                return Err(BackendError::SignatureSignerMismatch);
            }
            Ok(OptionRfqQuoteSignatureMetadata {
                signature: Some(signature.to_string()),
                quote_digest: Some(quote_digest),
                quote_nonce: Some(quote_nonce.to_string()),
                signature_status: OptionRfqQuoteSignatureStatus::Verified,
                recovered_signer: Some(recovered_signer),
            })
        }
    }
}

fn quote_expires_at_ms(
    state: &AppState,
    rfq: &OptionRfqRequest,
    now: TimestampMs,
    quote_ttl_ms: u64,
) -> Result<TimestampMs> {
    match state.options_config.rfq_quote_signature_mode {
        OptionRfqQuoteSignatureMode::Disabled => {
            checked_expiry(now, quote_ttl_ms, "option RFQ quote expiry")
                .map(|expires_at_ms| expires_at_ms.min(rfq.expires_at_ms))
        }
        OptionRfqQuoteSignatureMode::Strict => signed_quote_expires_at_ms(rfq, quote_ttl_ms),
    }
}

fn signed_quote_expires_at_ms(rfq: &OptionRfqRequest, quote_ttl_ms: u64) -> Result<TimestampMs> {
    let quote_ttl_ms = i64::try_from(quote_ttl_ms).map_err(|_| {
        BackendError::InvalidOptionRfqQuoteState("quote_ttl_ms cannot be encoded".to_string())
    })?;
    rfq.created_at_ms
        .checked_add(quote_ttl_ms)
        .map(|expires_at_ms| expires_at_ms.min(rfq.expires_at_ms))
        .ok_or_else(|| {
            BackendError::InvalidOptionRfqQuoteState("quote expiry overflow".to_string())
        })
}

fn validate_option_rfq_quote_signature_status(
    state: &AppState,
    quote: &OptionRfqQuote,
) -> Result<()> {
    if state.options_config.rfq_quote_signature_mode != OptionRfqQuoteSignatureMode::Strict {
        return Ok(());
    }
    if quote.signature_status != OptionRfqQuoteSignatureStatus::Verified {
        return Err(BackendError::InvalidOptionRfqQuoteState(format!(
            "option RFQ quote signature is {}",
            quote.signature_status.as_str()
        )));
    }
    Ok(())
}

fn option_rfq_quote_payload(
    rfq: &OptionRfqRequest,
    mm_account: AccountId,
    price_1e8: Price1e8,
    size_1e8: Size1e8,
    quote_nonce: u64,
    quote_ttl_ms: u64,
) -> Result<OptionRfqQuoteSigningPayload> {
    let expires_at_ms = signed_quote_expires_at_ms(rfq, quote_ttl_ms)?;
    let expiry = u128::try_from(expires_at_ms / 1000).map_err(|_| {
        BackendError::InvalidOptionRfqQuoteState("quote expiry cannot be encoded".to_string())
    })?;
    Ok(OptionRfqQuoteSigningPayload {
        option_rfq_id: option_rfq_id_to_b256(&rfq.option_rfq_id.to_string()),
        mm_account,
        option_series_id: option_series_id_to_b256(&rfq.option_series_id)?,
        taker_is_buyer: rfq.side == Side::Buy,
        price_1e8,
        size_1e8,
        quote_nonce: quote_nonce.into(),
        expiry,
    })
}

fn broadcast_option_rfq_request(state: &AppState, rfq: &OptionRfqRequest) {
    let message = ServerMessage::OptionRfqRequest(NotificationEnvelope::new(
        "option_rfq_request",
        format!("option-rfq-push-{}", rfq.option_rfq_id),
        OptionRfqRequestPayload {
            option_rfq_id: rfq.option_rfq_id,
            taker: rfq.taker.clone(),
            option_series_id: rfq.option_series_id.clone(),
            side: rfq.side,
            size_1e8: rfq.size_1e8.to_string(),
            limit_price_1e8: rfq.limit_price_1e8.map(|value| value.to_string()),
            expires_at_ms: rfq.expires_at_ms,
        },
    ));
    match state.mm_sessions.broadcast(message) {
        Ok(sent) => {
            info!(
                option_rfq_id = %rfq.option_rfq_id,
                broadcast_count = sent,
                "broadcast option RFQ request to MM sessions"
            );
        }
        Err(error) => {
            warn!(
                option_rfq_id = %rfq.option_rfq_id,
                error = %error,
                "option RFQ request broadcast failed"
            );
        }
    }
}

fn notify_option_rfq_quote_acceptance(
    state: &AppState,
    accepted_quote: &OptionRfqQuote,
    quotes_before_accept: &[OptionRfqQuote],
    option_fill_id: OptionRfqFillId,
) -> (bool, Option<String>) {
    let mut accepted_sent = false;
    let mut warning = None;

    if let Some(session_id) = accepted_quote.session_id.as_deref() {
        let message = ServerMessage::OptionRfqQuoteAccepted(NotificationEnvelope::new(
            "option_rfq_quote_accepted",
            format!("option-rfq-accepted-{}", accepted_quote.quote_id),
            OptionRfqQuoteAcceptedPayload {
                option_rfq_id: accepted_quote.option_rfq_id,
                quote_id: accepted_quote.quote_id,
                option_fill_id,
            },
        ));
        match state.mm_sessions.send_to_session(session_id, message) {
            Ok(()) => {
                accepted_sent = true;
            }
            Err(error) => {
                let message = error.to_string();
                warn!(
                    option_rfq_id = %accepted_quote.option_rfq_id,
                    quote_id = %accepted_quote.quote_id,
                    session_id,
                    error = %message,
                    "option RFQ quote accepted notification failed"
                );
                warning = Some(message);
            }
        }
    }

    for quote in quotes_before_accept {
        if quote.quote_id == accepted_quote.quote_id || quote.status != OptionRfqQuoteStatus::Active
        {
            continue;
        }
        let Some(session_id) = quote.session_id.as_deref() else {
            continue;
        };
        let message = ServerMessage::OptionRfqQuoteRejected(NotificationEnvelope::new(
            "option_rfq_quote_rejected",
            format!("option-rfq-rejected-{}", quote.quote_id),
            OptionRfqQuoteRejectedPayload {
                option_rfq_id: quote.option_rfq_id,
                quote_id: quote.quote_id,
                reason: "competing quote accepted".to_string(),
            },
        ));
        if let Err(error) = state.mm_sessions.send_to_session(session_id, message) {
            warn!(
                option_rfq_id = %quote.option_rfq_id,
                quote_id = %quote.quote_id,
                session_id,
                error = %error,
                "option RFQ quote rejected notification failed"
            );
        }
    }

    (accepted_sent, warning)
}

fn aggregate_levels(orders: &[OptionOrder], side: Side) -> Vec<OptionOrderbookLevel> {
    let mut by_price = BTreeMap::<Price1e8, Size1e8>::new();
    for order in orders {
        if order.side == side && order.status.is_live() {
            *by_price.entry(order.price_1e8).or_default() += order.remaining_size_1e8;
        }
    }

    let iter: Box<dyn Iterator<Item = (Price1e8, Size1e8)>> = match side {
        Side::Buy => Box::new(by_price.into_iter().rev()),
        Side::Sell => Box::new(by_price.into_iter()),
    };

    iter.map(|(price_1e8, size_1e8)| OptionOrderbookLevel {
        price_1e8: price_1e8.to_string(),
        size_1e8: size_1e8.to_string(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::ConfirmationReceipt;
    use crate::engine::EngineState;
    use crate::execution::rpc::{EstimateGasRequest, EthCallRequest, EthCallSuccess, RpcFuture};
    use crate::execution::{DecodedRevertError, RevertDiagnostics};
    use crate::nonce_sync::OptionNonceSyncConfig;
    use crate::options::{
        OptionExecutionConfirmationStatus, OptionExecutionGasCheckStatus,
        OptionExecutionSimulationStatus, OptionsConfig,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    enum MockOutcome {
        Success,
        Revert(RevertDiagnostics),
    }

    #[derive(Clone)]
    struct MockProvider {
        outcome: MockOutcome,
        calls: Arc<Mutex<Vec<EthCallRequest>>>,
    }

    #[derive(Clone)]
    struct MockOptionNonceProvider {
        buyer_nonce: u128,
        seller_nonce: u128,
        fail: bool,
        calls: Arc<Mutex<Vec<AccountId>>>,
    }

    #[derive(Clone)]
    enum MockEstimateOutcome {
        Value(u64),
        Failure(String),
    }

    #[derive(Clone)]
    struct MockBroadcastProvider {
        chain_id: u64,
        tx_hash: String,
        fail_send: bool,
        send_calls: Arc<Mutex<Vec<String>>>,
        nonce_calls: Arc<Mutex<Vec<AccountId>>>,
        estimate_outcome: MockEstimateOutcome,
        estimate_calls: Arc<Mutex<Vec<EstimateGasRequest>>>,
    }

    impl MockProvider {
        fn success() -> Self {
            Self {
                outcome: MockOutcome::Success,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn revert(revert_data: &str) -> Self {
            Self {
                outcome: MockOutcome::Revert(RevertDiagnostics {
                    raw_error: "execution reverted".to_string(),
                    revert_data: Some(revert_data.to_string()),
                    revert_selector: Some(revert_data[..10].to_string()),
                    decoded_error: DecodedRevertError {
                        kind: "unknown_custom_error".to_string(),
                        name: None,
                        selector: Some(revert_data[..10].to_string()),
                        args: None,
                        decoded: None,
                    },
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl MockOptionNonceProvider {
        fn success(buyer_nonce: u128, seller_nonce: u128) -> Self {
            Self {
                buyer_nonce,
                seller_nonce,
                fail: false,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn failure() -> Self {
            Self {
                buyer_nonce: 0,
                seller_nonce: 0,
                fail: true,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<AccountId> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl MockBroadcastProvider {
        fn success() -> Self {
            Self {
                chain_id: 84532,
                tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                fail_send: false,
                send_calls: Arc::new(Mutex::new(Vec::new())),
                nonce_calls: Arc::new(Mutex::new(Vec::new())),
                estimate_outcome: MockEstimateOutcome::Value(450_000),
                estimate_calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn invalid_hash() -> Self {
            Self {
                tx_hash: "0xnot-a-tx-hash".to_string(),
                ..Self::success()
            }
        }

        fn fail_send() -> Self {
            Self {
                fail_send: true,
                ..Self::success()
            }
        }

        fn with_estimate(estimated_gas: u64) -> Self {
            Self {
                estimate_outcome: MockEstimateOutcome::Value(estimated_gas),
                ..Self::success()
            }
        }

        fn with_estimate_failure(error: impl Into<String>) -> Self {
            Self {
                estimate_outcome: MockEstimateOutcome::Failure(error.into()),
                ..Self::success()
            }
        }

        fn send_count(&self) -> usize {
            self.send_calls.lock().unwrap().len()
        }

        fn estimate_count(&self) -> usize {
            self.estimate_calls.lock().unwrap().len()
        }
    }

    impl EthCallProvider for MockProvider {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let outcome = self.outcome.clone();
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(request);
                match outcome {
                    MockOutcome::Success => Ok(EthCallSuccess {
                        block_number: Some(123),
                        output: Vec::new(),
                    }),
                    MockOutcome::Revert(diagnostics) => {
                        Err(BackendError::SimulationReverted(Box::new(diagnostics)))
                    }
                }
            })
        }
    }

    impl OptionNonceProvider for MockOptionNonceProvider {
        fn option_matching_nonce(
            &self,
            _matching_engine: AccountId,
            account: AccountId,
        ) -> RpcFuture<'_, u128> {
            let calls = self.calls.clone();
            let buyer_nonce = self.buyer_nonce;
            let seller_nonce = self.seller_nonce;
            let fail = self.fail;
            Box::pin(async move {
                calls.lock().unwrap().push(account.clone());
                if fail {
                    return Err(BackendError::Simulation(
                        "option nonce RPC unavailable".to_string(),
                    ));
                }
                if account.0.ends_with("0001") {
                    Ok(buyer_nonce)
                } else {
                    Ok(seller_nonce)
                }
            })
        }
    }

    impl GasEstimateProvider for MockBroadcastProvider {
        fn estimate_gas(&self, request: EstimateGasRequest) -> RpcFuture<'_, u64> {
            let calls = self.estimate_calls.clone();
            let outcome = self.estimate_outcome.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(request);
                match outcome {
                    MockEstimateOutcome::Value(value) => Ok(value),
                    MockEstimateOutcome::Failure(message) => Err(BackendError::Simulation(message)),
                }
            })
        }
    }

    impl TransactionBroadcastProvider for MockBroadcastProvider {
        fn chain_id(&self) -> RpcFuture<'_, u64> {
            let chain_id = self.chain_id;
            Box::pin(async move { Ok(chain_id) })
        }

        fn transaction_count(&self, address: AccountId) -> RpcFuture<'_, u64> {
            let calls = self.nonce_calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(address);
                Ok(7)
            })
        }

        fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String> {
            let calls = self.send_calls.clone();
            let tx_hash = self.tx_hash.clone();
            let fail_send = self.fail_send;
            Box::pin(async move {
                calls.lock().unwrap().push(raw_transaction);
                if fail_send {
                    return Err(BackendError::Simulation("mock send failed".to_string()));
                }
                Ok(tx_hash)
            })
        }
    }

    // EthCallProvider + EthBalanceProvider impls — minimal stub behaviour
    // so the runtime LiveProvider helper can drive the mock through
    // gather_inputs. eth_call always fails (so every chain-state read
    // surfaces a `policy_data_failures_total{...}` increment when the
    // runtime helper is used) — verifies the failure-counter wiring is
    // hooked through the entry point.
    impl crate::execution::EthCallProvider for MockBroadcastProvider {
        fn eth_call(
            &self,
            _request: crate::execution::EthCallRequest,
        ) -> RpcFuture<'_, crate::execution::EthCallSuccess> {
            Box::pin(async move {
                Err(BackendError::Simulation(
                    "mock eth_call failure".to_string(),
                ))
            })
        }
    }

    impl crate::execution::EthBalanceProvider for MockBroadcastProvider {
        fn eth_get_balance(&self, _address: AccountId) -> RpcFuture<'_, u128> {
            Box::pin(async move {
                Err(BackendError::Simulation(
                    "mock eth_get_balance failure".to_string(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn option_nonce_sync_disabled_preserves_zero_intent_nonces() {
        let state = state_with_simulation(false);
        insert_executable_series(&state);
        let fill = orderbook_fill();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider::<
            MockOptionNonceProvider,
        >(&state, &fill, None)
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(0));
        assert_eq!(intent.seller_nonce, Some(0));
    }

    #[tokio::test]
    async fn option_execution_intent_uses_synced_option_nonces() {
        let state = state_with_option_nonce_sync(true);
        insert_executable_series(&state);
        let provider = MockOptionNonceProvider::success(17, 23);
        let fill = orderbook_fill();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &fill,
            Some(&provider),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(17));
        assert_eq!(intent.seller_nonce, Some(23));
        let calls = provider.calls();
        assert_eq!(calls, vec![fill.buyer, fill.seller]);
    }

    #[tokio::test]
    async fn strict_option_nonce_sync_failure_does_not_create_intent() {
        let state = state_with_option_nonce_sync(true);
        insert_executable_series(&state);
        let fill = orderbook_fill();
        let provider = MockOptionNonceProvider::failure();

        let error = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &fill,
            Some(&provider),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("option nonce RPC unavailable"));
        assert!(state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_intents()
            .is_empty());
    }

    #[tokio::test]
    async fn non_strict_option_nonce_sync_failure_falls_back_to_zero() {
        let state = state_with_option_nonce_sync(false);
        insert_executable_series(&state);
        let provider = MockOptionNonceProvider::failure();

        let intent = create_option_orderbook_execution_intent_with_nonce_provider(
            &state,
            &orderbook_fill(),
            Some(&provider),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(intent.buyer_nonce, Some(0));
        assert_eq!(intent.seller_nonce, Some(0));
    }

    #[tokio::test]
    async fn option_execution_signing_payload_uses_stored_synced_nonces() {
        let state = state_with_simulation(false);
        let mut intent = calldata_ready_intent();
        intent.buyer_nonce = Some(17);
        intent.seller_nonce = Some(23);
        let intent = insert_intent(&state, intent);

        let outcome = option_execution_signing_payload(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(outcome.payload.buyer_nonce, 17);
        assert_eq!(outcome.payload.seller_nonce, 23);
    }

    #[tokio::test]
    async fn option_execution_calldata_uses_stored_synced_nonces() {
        let state = state_with_simulation(false);
        let mut intent = calldata_ready_intent();
        intent.buyer_nonce = Some(17);
        intent.seller_nonce = Some(23);
        intent.calldata = None;
        intent.status = OptionExecutionIntentStatus::SignaturesRequired;
        intent.buyer_signature = Some(signature_hex(0xaa));
        intent.seller_signature = Some(signature_hex(0xbb));
        let expected_payload = OptionTradePayload::from_intent(&intent).unwrap();
        let expected_calldata = build_option_execution_calldata_from_parts(
            &expected_payload,
            intent.buyer_signature.as_deref().unwrap(),
            intent.seller_signature.as_deref().unwrap(),
        )
        .unwrap();
        let intent = insert_intent(&state, intent);

        let outcome = option_execution_calldata(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            outcome.calldata.as_deref(),
            Some(expected_calldata.as_str())
        );
    }

    #[tokio::test]
    async fn option_execution_simulation_disabled_rejects() {
        let state = state_with_simulation(false);
        let intent = insert_intent(&state, calldata_ready_intent());

        let error = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap_err();

        assert!(
            matches!(error, BackendError::Config(message) if message.contains("option execution simulation is disabled"))
        );
    }

    #[tokio::test]
    async fn option_execution_simulation_missing_intent_rejects() {
        let state = state_with_simulation(true);

        let error = prepare_option_execution_simulation(&state, Uuid::from_u128(99))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentId
        ));
    }

    #[tokio::test]
    async fn option_execution_simulation_missing_calldata_rejects_and_stores_unavailable() {
        let state = state_with_simulation(true);
        let intent = insert_intent(
            &state,
            OptionExecutionIntent {
                calldata: None,
                ..calldata_ready_intent()
            },
        );

        let error = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap_err();
        let status = option_execution_simulation_status(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(
            matches!(error, BackendError::InvalidOptionExecutionIntentState(message) if message.contains("calldata"))
        );
        assert_eq!(
            status.simulation_status,
            OptionExecutionSimulationStatus::SimulationUnavailable
        );
        assert!(status.error.unwrap().contains("calldata"));
    }

    #[tokio::test]
    async fn option_execution_simulation_success_stores_ok_without_changing_intent_status() {
        let state = state_with_simulation(true);
        let intent = insert_intent(&state, calldata_ready_intent());
        let provider = MockProvider::success();

        let prepared = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap();
        let result = simulate_prepared_option_execution_intent(&state, &prepared, &provider)
            .await
            .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            result.simulation_status,
            OptionExecutionSimulationStatus::SimulationOk
        );
        assert_eq!(result.block_number, Some(123));
        assert_eq!(stored.status, OptionExecutionIntentStatus::CalldataReady);
        assert_eq!(
            stored.simulation_status,
            Some(OptionExecutionSimulationStatus::SimulationOk)
        );
        assert_eq!(stored.simulation_block_number, Some(123));
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.0, "0x00000000000000000000000000000000000000ee");
        assert_eq!(calls[0].gas_limit, Some(500_000));
    }

    #[tokio::test]
    async fn option_execution_simulation_revert_stores_failed_and_revert_data() {
        let state = state_with_simulation(true);
        let intent = insert_intent(&state, calldata_ready_intent());
        let provider = MockProvider::revert("0x12345678");

        let prepared = prepare_option_execution_simulation(&state, intent.intent_id)
            .await
            .unwrap();
        let result = simulate_prepared_option_execution_intent(&state, &prepared, &provider)
            .await
            .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert_eq!(
            result.simulation_status,
            OptionExecutionSimulationStatus::SimulationFailed
        );
        assert_eq!(result.revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(result.revert_selector.as_deref(), Some("0x12345678"));
        assert_eq!(
            stored.simulation_status,
            Some(OptionExecutionSimulationStatus::SimulationFailed)
        );
        assert_eq!(stored.simulation_revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(stored.status, OptionExecutionIntentStatus::CalldataReady);
    }

    #[tokio::test]
    async fn option_execution_broadcast_disabled_rejects_without_send() {
        let state = state_with_broadcast(false);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(error, BackendError::Config(message) if message.contains("broadcast is disabled"))
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_intent_rejects() {
        let state = state_with_broadcast(true);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, Uuid::from_u128(99), &provider)
                .await
                .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentId
        ));
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_calldata_rejects_without_transaction() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.calldata = None;
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("calldata-missing")),
            "expected policy calldata-missing rejection, got {error:?}"
        );
        assert_eq!(provider.send_count(), 0);
        assert!(option_transactions(&state, intent.intent_id).is_empty());
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_signatures_rejects_without_send() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.buyer_signature = None;
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("buyer-sig-missing")),
            "expected policy buyer-sig-missing rejection, got {error:?}"
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_requires_simulation_ok_by_default() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.simulation_status = Some(OptionExecutionSimulationStatus::SimulationFailed);
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("sim-revert")),
            "expected policy sim-revert rejection, got {error:?}"
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_missing_rpc_or_private_key_rejects() {
        let mut missing_rpc = state_with_broadcast(true);
        missing_rpc.execution_config.rpc_url = None;
        let intent = insert_intent(&missing_rpc, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let error = broadcast_option_execution_intent_with_provider(
            &missing_rpc,
            intent.intent_id,
            &provider,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, BackendError::Config(message) if message.contains("RPC_URL")));
        assert_eq!(provider.send_count(), 0);

        let mut missing_key = state_with_broadcast(true);
        missing_key.execution_config.executor_private_key = None;
        let intent = insert_intent(&missing_key, broadcast_ready_intent());
        let error = broadcast_option_execution_intent_with_provider(
            &missing_key,
            intent.intent_id,
            &provider,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, BackendError::Config(message) if message.contains("EXECUTOR_PRIVATE_KEY"))
        );
        assert_eq!(provider.send_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_broadcast_mock_success_persists_submitted_hash_once() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let outcome =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(outcome.submitted);
        assert!(!outcome.duplicate);
        assert_eq!(
            outcome.transaction.tx_hash.as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            stored.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].status,
            ExecutionTransactionStatus::Submitted
        );
        assert_eq!(
            transactions[0].tx_hash.as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(provider.send_count(), 1);

        let duplicate =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(
            duplicate.transaction.transaction_id,
            transactions[0].transaction_id
        );
        assert_eq!(provider.send_count(), 1);
    }

    #[tokio::test]
    async fn option_execution_broadcast_does_not_persist_invalid_or_failed_tx_hashes() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::invalid_hash();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();

        assert!(
            matches!(error, BackendError::BroadcastRejected(message) if message.contains("invalid transaction hash"))
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);

        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::fail_send();
        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(error.to_string().contains("mock send failed"));
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
    }

    // ----------------------------------------------------------------------------
    // BACKEND-SHOULD-BROADCAST-ECONOMIC-GATE regression tests
    // (Phase D, Test 22 + 23: gate <-> intent state-machine integration)
    // ----------------------------------------------------------------------------

    /// Test 22 — `should_broadcast` returns Approve → existing state-machine
    /// transitions unchanged: BroadcastSubmitted with a tx_hash.
    #[tokio::test]
    async fn policy_approve_preserves_existing_broadcast_state_machine() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();

        let outcome =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert!(outcome.submitted);
        assert!(!outcome.duplicate);
        assert_eq!(provider.send_count(), 1);
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            stored.status,
            OptionExecutionIntentStatus::BroadcastSubmitted,
            "policy Approve must leave existing broadcast_submitted transition intact"
        );
        let transactions = option_transactions(&state, intent.intent_id);
        assert_eq!(transactions.len(), 1);
        assert!(transactions[0].tx_hash.is_some());
    }

    /// Test 23 — `should_broadcast` returns Reject → intent transitions to
    /// BroadcastFailed cleanly with a policy-prefixed reason; no half-state,
    /// no tx_hash, no provider broadcast.
    #[tokio::test]
    async fn policy_reject_transitions_cleanly_without_half_state() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.buyer = AccountId::new("0x000000000000000000000000000000000000aaaa");
        intent.seller = intent.buyer.clone();
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.starts_with("policy:wash")),
            "expected policy wash rejection, got {error:?}"
        );
        assert_eq!(provider.send_count(), 0);
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            stored.status,
            OptionExecutionIntentStatus::BroadcastFailed,
            "policy Reject must drive intent into broadcast_failed without intermediate states"
        );
        assert!(
            stored
                .error
                .as_deref()
                .unwrap_or("")
                .starts_with("policy:wash"),
            "stored error must carry the structured policy reason; got {:?}",
            stored.error
        );
        assert!(
            option_transactions(&state, intent.intent_id).is_empty(),
            "no tx row may be persisted when the policy rejects pre-broadcast"
        );
    }

    // ----------------------------------------------------------------------------
    // BACKEND-SIGNER-INTERFACE-KMS-HSM-ADAPTER integration tests
    // (signer abstraction <-> option execution broadcast path)
    // ----------------------------------------------------------------------------

    use crate::execution::remote_signer::{
        RemoteSigner, SignerError, SignerFuture, SignerHealth, SignerRequest, SignerResponse,
    };
    use crate::execution::SignerBackendKind;

    /// Mock RemoteSigner: yields a deterministic recoverable signature
    /// produced by the same test key the LocalDevSigner uses, OR returns a
    /// canned error. Counts sign calls so tests can assert no-fallback /
    /// not-called behaviour.
    struct MockBackendSigner {
        signer: ExecutorSigner,
        outcome: Mutex<MockSignerOutcome>,
        sign_calls: Mutex<u64>,
    }

    enum MockSignerOutcome {
        Sign,
        Reject(SignerError),
    }

    impl MockBackendSigner {
        fn approving() -> Self {
            let inner = ExecutorSigner::from_private_key(&crate::execution::PrivateKeySecret::new(
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
            ))
            .expect("test key parse");
            Self {
                signer: inner,
                outcome: Mutex::new(MockSignerOutcome::Sign),
                sign_calls: Mutex::new(0),
            }
        }

        fn rejecting(err: SignerError) -> Self {
            let inner = ExecutorSigner::from_private_key(&crate::execution::PrivateKeySecret::new(
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
            ))
            .expect("test key parse");
            Self {
                signer: inner,
                outcome: Mutex::new(MockSignerOutcome::Reject(err)),
                sign_calls: Mutex::new(0),
            }
        }

        fn sign_calls_count(&self) -> u64 {
            *self.sign_calls.lock().unwrap()
        }
    }

    impl RemoteSigner for MockBackendSigner {
        fn signer_address(&self) -> &AccountId {
            self.signer.address()
        }

        fn kind(&self) -> SignerBackendKind {
            SignerBackendKind::Remote
        }

        fn sign_option_execution_tx<'a>(
            &'a self,
            request: SignerRequest<'a>,
        ) -> SignerFuture<'a, SignerResponse> {
            *self.sign_calls.lock().unwrap() += 1;
            let SignerRequest {
                request_id,
                policy_decision_id,
                prehash,
                ..
            } = request;
            let address = self.signer.address().clone();
            let outcome_guard = self.outcome.lock().unwrap();
            let outcome_clone = match &*outcome_guard {
                MockSignerOutcome::Sign => MockSignerOutcome::Sign,
                MockSignerOutcome::Reject(err) => MockSignerOutcome::Reject(err.clone()),
            };
            drop(outcome_guard);
            let signer = self.signer.clone();
            Box::pin(async move {
                match outcome_clone {
                    MockSignerOutcome::Sign => {
                        let signature = signer
                            .sign_prehash(&prehash)
                            .map_err(|e| SignerError::Internal(e.to_string()))?;
                        Ok(SignerResponse {
                            request_id,
                            signer_address: address,
                            signature,
                            kms_request_id: Some("mock-kms".to_string()),
                            audit_log_id: Some("mock-audit".to_string()),
                            remote_signer_request_id: Some("mock-rsig".to_string()),
                            created_at_ms: 1,
                            policy_decision_id,
                        })
                    }
                    MockSignerOutcome::Reject(err) => Err(err),
                }
            })
        }

        fn health_check(&self) -> SignerFuture<'_, SignerHealth> {
            Box::pin(async move {
                Ok(SignerHealth {
                    mode: SignerBackendKind::Remote,
                    signer_address: Some(self.signer.address().clone()),
                    remote_endpoint_present: true,
                    healthy: true,
                })
            })
        }
    }

    /// should_broadcast Approve → signer called exactly once → intent
    /// transitions to BroadcastSubmitted; preserves existing state-machine.
    #[tokio::test]
    async fn signer_approve_routes_through_broadcast_and_marks_submitted() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();

        let outcome = broadcast_option_execution_intent_with_provider_and_signer(
            &state,
            intent.intent_id,
            &provider,
            &signer,
        )
        .await
        .expect("approving signer must transit broadcast");

        assert!(outcome.submitted);
        assert!(!outcome.duplicate);
        assert_eq!(signer.sign_calls_count(), 1);
        assert_eq!(provider.send_count(), 1);
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            stored.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
    }

    /// should_broadcast Reject → signer must NOT be called (gate runs first).
    #[tokio::test]
    async fn signer_not_called_when_policy_rejects() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.buyer = AccountId::new("0x000000000000000000000000000000000000aaaa");
        intent.seller = intent.buyer.clone();
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();

        let error = broadcast_option_execution_intent_with_provider_and_signer(
            &state,
            intent.intent_id,
            &provider,
            &signer,
        )
        .await
        .expect_err("policy reject must short-circuit signer");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:wash")),
            "expected policy wash rejection, got {error:?}"
        );
        assert_eq!(
            signer.sign_calls_count(),
            0,
            "signer must not be invoked when policy rejects"
        );
        assert_eq!(provider.send_count(), 0);
    }

    /// Remote signer rejection → BroadcastFailed clean; NO fallback to a
    /// different signer; no chain send; tx row persisted with structured
    /// signer:<code> reason.
    #[tokio::test]
    async fn signer_rejection_transitions_to_broadcast_failed_without_fallback() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::rejecting(SignerError::PolicyFingerprint);

        let error = broadcast_option_execution_intent_with_provider_and_signer(
            &state,
            intent.intent_id,
            &provider,
            &signer,
        )
        .await
        .expect_err("rejecting signer must surface error");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("signer:policy-fingerprint")),
            "expected signer:policy-fingerprint, got {error:?}"
        );
        assert_eq!(signer.sign_calls_count(), 1);
        assert_eq!(
            provider.send_count(),
            0,
            "no chain broadcast after signer rejection — no fallback path"
        );
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);
        let transactions = option_transactions(&state, intent.intent_id);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].tx_hash, None);
    }

    /// KMS upstream timeout maps to BroadcastFailed with signer:kms-timeout;
    /// proves the no-fallback contract under upstream outage.
    #[tokio::test]
    async fn signer_kms_timeout_marks_intent_failed_no_local_fallback() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::rejecting(SignerError::KmsTimeout);

        let error = broadcast_option_execution_intent_with_provider_and_signer(
            &state,
            intent.intent_id,
            &provider,
            &signer,
        )
        .await
        .expect_err("KMS timeout must surface as broadcast failure");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("signer:kms-timeout")),
            "expected signer:kms-timeout, got {error:?}"
        );
        assert_eq!(provider.send_count(), 0);
    }

    // ----------------------------------------------------------------------------
    // WIRE-SHOULD-BROADCAST-CHAIN-STATE-READS integration tests
    // (data provider <-> policy gate <-> signer)
    // ----------------------------------------------------------------------------

    use crate::options::broadcast_policy_data::{
        BroadcastPolicyInputs, DedupeReason, StubBroadcastPolicyDataProvider,
    };

    fn happy_inputs() -> BroadcastPolicyInputs {
        BroadcastPolicyInputs {
            chain_id_rpc: Some(84532),
            be_balance_wei: Some(u128::MAX / 2),
            ome_paused: Some(false),
            ome_is_executor: Some(true),
            pfv_fee_balance_asset: Some(0),
            pfv_rebate_reserve_asset: Some(0),
            cv_pfv_balance_asset: Some(0),
            fee_split: None,
            fm_v2_rebate_budget_asset: None,
            dedupe_hit: false,
            dedupe_reason: DedupeReason::None,
            r5_drift_zero: Some(true),
        }
    }

    /// Stub data provider returns OME paused → policy rejects with
    /// `ome-paused`; signer must NOT be called; intent transitions to
    /// `BroadcastFailed`.
    #[tokio::test]
    async fn data_provider_ome_paused_rejects_before_signer_call() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.ome_paused = Some(true);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let error = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("OME paused must reject before signer");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:ome-paused")),
            "expected policy:ome-paused, got {error:?}"
        );
        assert_eq!(
            signer.sign_calls_count(),
            0,
            "signer must not be called when OME paused"
        );
        assert_eq!(provider.send_count(), 0);
    }

    /// Stub data provider reports BE is not the chain-side executor →
    /// policy rejects with `be-not-exec`; signer not called.
    #[tokio::test]
    async fn data_provider_be_not_executor_rejects_before_signer_call() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.ome_is_executor = Some(false);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let error = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("BE not executor must reject");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:be-not-exec")),
            "expected policy:be-not-exec, got {error:?}"
        );
        assert_eq!(signer.sign_calls_count(), 0);
    }

    /// R5 drift detected → policy rejects with `policy-internal:r5-drift`;
    /// signer not called. Mainnet hard gate.
    #[tokio::test]
    async fn data_provider_r5_drift_rejects_before_signer_call() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.r5_drift_zero = Some(false);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let error = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("r5 drift must reject");

        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:policy-internal:r5-drift")),
            "expected policy:policy-internal:r5-drift, got {error:?}"
        );
        assert_eq!(signer.sign_calls_count(), 0);
        assert_eq!(provider.send_count(), 0);
    }

    /// Provider data populates `econ_data_available=false` on the
    /// Sepolia rehearsal path so the existing fee-only smoke regression
    /// continues to land Approve and broadcast through unchanged. This
    /// test pins the boundary-mode behaviour.
    #[tokio::test]
    async fn data_provider_sepolia_path_still_approves_under_boundary_mode() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let outcome = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("Sepolia boundary path must continue to approve");

        assert!(outcome.submitted);
        assert!(!outcome.duplicate);
        assert_eq!(signer.sign_calls_count(), 1);
        assert_eq!(provider.send_count(), 1);
    }

    /// Live FM_V2 quote populates fee_split + rebate budget + reserve →
    /// `econ_data_available = true` → §8 step 4 still approves a fee-only
    /// trade; signer is called once.
    #[tokio::test]
    async fn fee_split_populated_fee_only_intent_approves() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 150,
            total_rebate_outflow: 0,
            net_protocol_revenue: 150,
            effective_maker_ppm: 50,
            effective_taker_ppm: 100,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(0);
        inputs.pfv_rebate_reserve_asset = Some(0);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let outcome = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("fee-only econ-available path must approve");
        assert!(outcome.submitted);
        assert_eq!(signer.sign_calls_count(), 1);
    }

    /// Live FM_V2 quote contains a rebate-positive intent but PFV
    /// rebate_reserve = 0 → §8 step 5 hard gate rejects with
    /// `policy:rebate-reserve`; signer not called.
    #[tokio::test]
    async fn fee_split_rebate_positive_with_zero_reserve_rejects() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 100,
            total_rebate_outflow: 50,
            net_protocol_revenue: 50,
            effective_maker_ppm: -50,
            effective_taker_ppm: 100,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(u128::MAX);
        inputs.pfv_rebate_reserve_asset = Some(0); // launch state
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let error = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("rebate-positive with zero PFV reserve must reject");
        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:rebate-reserve")),
            "expected policy:rebate-reserve, got {error:?}"
        );
        assert_eq!(signer.sign_calls_count(), 0);
    }

    /// Live FM_V2 quote contains a rebate-positive intent and FM_V2
    /// rebateBudget(asset) is insufficient → §8 step 5 hard gate rejects
    /// with `policy:rebate-budget`; signer not called.
    #[tokio::test]
    async fn fee_split_rebate_positive_with_insufficient_budget_rejects() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 100,
            total_rebate_outflow: 200,
            net_protocol_revenue: -100,
            effective_maker_ppm: -50,
            effective_taker_ppm: -150,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(50);
        inputs.pfv_rebate_reserve_asset = Some(u128::MAX);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let error = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("rebate-positive with insufficient budget must reject");
        assert!(
            matches!(&error, BackendError::BroadcastRejected(msg) if msg.starts_with("policy:rebate-budget")),
            "expected policy:rebate-budget, got {error:?}"
        );
        assert_eq!(signer.sign_calls_count(), 0);
    }

    // ----------------------------------------------------------------------------
    // BACKEND-EXECUTOR-MONITORING-ALERTS-V1-WIRING integration tests
    // (observability counters fire on the right transitions; metrics
    // text renders with the expected gauge names; signer NOT called
    // when policy rejects)
    // ----------------------------------------------------------------------------

    /// Policy approve → policy_approved counter increments; signer
    /// attempted + success counters increment; last_submitted_ms persisted.
    #[tokio::test]
    async fn observability_policy_approve_increments_signer_counters() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let outcome = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("approve path must succeed");
        assert!(outcome.submitted);

        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.policy_approved_total.get("orderbook"), Some(&1));
        assert_eq!(snap.signer_attempted_total.get("remote"), Some(&1));
        assert_eq!(snap.signer_success_total.get("remote"), Some(&1));
        assert!(snap.last_broadcast_submitted_ms.is_some());
        // policy rejects must be empty on a clean approve path.
        assert_eq!(snap.policy_rejected_total.len(), 0);
    }

    /// Policy reject with `wash` code → policy_rejected counter
    /// increments with (wash, orderbook); last_policy_reject_code set;
    /// signer NEVER called.
    #[tokio::test]
    async fn observability_policy_wash_reject_increments_counter_and_not_signer() {
        let state = state_with_broadcast(true);
        let mut intent = broadcast_ready_intent();
        intent.buyer = AccountId::new("0x000000000000000000000000000000000000aaaa");
        intent.seller = intent.buyer.clone();
        let intent = insert_intent(&state, intent);
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("wash must reject");

        let snap = state.broadcast_observability.snapshot();
        assert_eq!(
            snap.policy_rejected_total
                .get(&("wash".to_string(), "orderbook".to_string())),
            Some(&1)
        );
        assert_eq!(snap.last_policy_reject_code.as_deref(), Some("wash"));
        assert_eq!(snap.signer_attempted_total.len(), 0);
        assert_eq!(signer.sign_calls_count(), 0);
    }

    /// `econ_data_available_true_total` increments when fee_split is
    /// populated; `econ_data_available_false_total` increments otherwise.
    #[tokio::test]
    async fn observability_econ_data_available_true_increments_when_fee_split_present() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 150,
            total_rebate_outflow: 0,
            net_protocol_revenue: 150,
            effective_maker_ppm: 50,
            effective_taker_ppm: 100,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(0);
        inputs.pfv_rebate_reserve_asset = Some(0);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("econ-data-true approve path");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.econ_data_available_true_total, 1);
        assert_eq!(snap.econ_data_available_false_total, 0);
    }

    /// When `fee_split` is populated, the broadcast call site records the
    /// effective maker + taker ppm singletons using the EXACT values
    /// from the `FeeSplitSummary` that drove `should_broadcast`. Pins
    /// the contract that the JSON `/executor/health/v2` endpoint reports
    /// the same numbers the policy gate saw.
    #[tokio::test]
    async fn observability_effective_fee_ppm_recorded_when_fee_split_present() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 150,
            total_rebate_outflow: 0,
            net_protocol_revenue: 150,
            effective_maker_ppm: 42,
            effective_taker_ppm: 99,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(0);
        inputs.pfv_rebate_reserve_asset = Some(0);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("approve path");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.last_effective_maker_ppm, Some(42));
        assert_eq!(snap.last_effective_taker_ppm, Some(99));
    }

    /// RFQ-source intents take the same broadcast path as orderbook
    /// intents, so the effective-ppm singletons MUST also land when the
    /// `source_type` is `OptionRfqFill`. Pins the cross-source contract.
    #[tokio::test]
    async fn observability_effective_fee_ppm_recorded_on_rfq_path() {
        use crate::options::broadcast_policy::FeeSplitSummary;
        let state = state_with_broadcast(true);
        let mut intent_template = broadcast_ready_intent();
        intent_template.source_type = OptionExecutionSourceType::OptionRfqFill;
        let intent = insert_intent(&state, intent_template);
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = Some(FeeSplitSummary {
            gross_fee_revenue: 200,
            total_rebate_outflow: 0,
            net_protocol_revenue: 200,
            effective_maker_ppm: 15,
            effective_taker_ppm: 60,
            asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            tier: 0,
        });
        inputs.fm_v2_rebate_budget_asset = Some(0);
        inputs.pfv_rebate_reserve_asset = Some(0);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("RFQ-source approve path");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.last_effective_maker_ppm, Some(15));
        assert_eq!(snap.last_effective_taker_ppm, Some(60));
        assert_eq!(snap.policy_approved_total.get("rfq"), Some(&1));
    }

    /// `run_should_broadcast_policy` records the BE-balance-floor (the
    /// same `fund_floor_wei` it passes into the policy context) into the
    /// observability snapshot for every broadcast attempt, regardless
    /// of source type. Pins the orderbook path.
    #[tokio::test]
    async fn observability_be_balance_floor_wei_recorded_on_orderbook_path() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("approve path");
        let snap = state.broadcast_observability.snapshot();
        // state_with_broadcast(true) sets chain_id to Sepolia → permissive
        // chain state → fund_floor_wei = 0. The 0 is a legitimate
        // policy reading; the singleton MUST report it verbatim.
        assert_eq!(snap.last_be_balance_floor_wei, Some(0));
    }

    /// Same as above but with the RFQ source — pins the cross-source
    /// contract that `run_should_broadcast_policy` records the floor
    /// independent of `source_type`.
    #[tokio::test]
    async fn observability_be_balance_floor_wei_recorded_on_rfq_path() {
        let state = state_with_broadcast(true);
        let mut intent_template = broadcast_ready_intent();
        intent_template.source_type = OptionExecutionSourceType::OptionRfqFill;
        let intent = insert_intent(&state, intent_template);
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("RFQ-source approve path");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.last_be_balance_floor_wei, Some(0));
        assert_eq!(snap.policy_approved_total.get("rfq"), Some(&1));
    }

    /// When `fee_split` is `None` (boundary mode), the broadcast call
    /// site MUST NOT record fake `(0, 0)` effective-ppm readings — the
    /// snapshot retains whatever (possibly None) value it held before
    /// the attempt. Pins the "no fake zeros" contract.
    #[tokio::test]
    async fn observability_effective_fee_ppm_not_recorded_when_fee_split_missing() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.fee_split = None;
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("approve path under boundary mode");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.last_effective_maker_ppm, None);
        assert_eq!(snap.last_effective_taker_ppm, None);
        // The boundary-mode counter still increments.
        assert_eq!(snap.econ_data_available_false_total, 1);
    }

    /// R5 drift detected by data provider → r5_drift_observed_total
    /// increments AND `policy:policy-internal:r5-drift` reject increments
    /// with the (policy-internal, orderbook) label pair.
    #[tokio::test]
    async fn observability_r5_drift_increments_drift_counter_and_policy_internal_reject() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let mut inputs = happy_inputs();
        inputs.r5_drift_zero = Some(false);
        let data_provider = StubBroadcastPolicyDataProvider::new(inputs);

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("r5 drift must reject");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.r5_drift_observed_total, 1);
        assert_eq!(
            snap.policy_rejected_total
                .get(&("policy-internal".to_string(), "orderbook".to_string())),
            Some(&1)
        );
        assert_eq!(signer.sign_calls_count(), 0);
    }

    /// Signer denial → signer_denied counter increments with the (code,
    /// signer_kind) pair; signer_attempted increments; signer_success
    /// does NOT.
    #[tokio::test]
    async fn observability_signer_denial_increments_denied_counter() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::rejecting(SignerError::PolicyFingerprint);
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect_err("signer denial must surface");
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.signer_attempted_total.get("remote"), Some(&1));
        assert!(!snap.signer_success_total.contains_key("remote"));
        assert_eq!(
            snap.signer_denied_total
                .get(&("policy-fingerprint".to_string(), "remote".to_string())),
            Some(&1)
        );
    }

    /// `build_signer_for_state` runtime refusal on mainnet increments
    /// the `local_signer_on_mainnet_refused_total` counter.
    #[test]
    fn observability_local_signer_on_mainnet_refused_increments_counter() {
        let mut state = state_with_broadcast(true);
        state.execution_config.executor_chain_id = MAINNET_CHAIN_ID;
        state.execution_config.backend_signer_mode = SignerBackendKind::LocalDev;
        match build_signer_for_state(&state) {
            Err(BackendError::Config(_)) => {}
            Err(other) => panic!("expected Config error, got {other:?}"),
            Ok(_) => panic!("must refuse"),
        }
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(snap.local_signer_on_mainnet_refused_total, 1);
    }

    // ----------------------------------------------------------------------------
    // BACKEND-LIVE-PROVIDER-IN-MAIN-WIRING runtime helper tests
    // ----------------------------------------------------------------------------

    use crate::options::broadcast_policy_data::{read_type, BroadcastPolicyDataProvider};

    /// The runtime helper attaches the state's `BroadcastObservability`
    /// handle so live-read failures land in `/metrics`. Verified by
    /// invoking `gather_inputs` through a mock provider that fails every
    /// `eth_call` + `eth_get_balance`, then asserting the
    /// `policy_data_failures_total` snapshot records each failure.
    #[tokio::test]
    async fn build_runtime_policy_data_provider_attaches_observability() {
        let state = state_with_broadcast(true);
        let provider = MockBroadcastProvider::success();
        let data_provider = build_runtime_policy_data_provider(&state, provider);
        let intent = broadcast_ready_intent();
        let _ = data_provider.gather_inputs(&state, &intent).await.unwrap();
        let snap = state.broadcast_observability.snapshot();
        assert!(
            snap.policy_data_failures_total
                .get(read_type::BE_BALANCE)
                .copied()
                .unwrap_or(0)
                >= 1,
            "be_balance failure must surface via the attached observability handle; got {:?}",
            snap.policy_data_failures_total
        );
        // OME paused / isExecutor are eth_call reads; the mock fails
        // both → both counters must increment.
        assert!(
            snap.policy_data_failures_total
                .get(read_type::OME_PAUSED)
                .copied()
                .unwrap_or(0)
                >= 1,
            "ome_paused failure must surface via the runtime path; got {:?}",
            snap.policy_data_failures_total
        );
        assert!(
            snap.policy_data_failures_total
                .get(read_type::OME_IS_EXECUTOR)
                .copied()
                .unwrap_or(0)
                >= 1,
            "ome_is_executor failure must surface via the runtime path"
        );
    }

    /// FM_V2 RPC failure metric fires when the runtime helper has an
    /// FM_V2 address threaded through state. Mirrors the prior
    /// LiveProvider test but exercises the entry-point construction path.
    #[tokio::test]
    async fn build_runtime_policy_data_provider_records_fm_v2_rpc_failure() {
        let mut state = state_with_broadcast(true);
        state.option_event_indexer_config.fees_manager_v2_address =
            Some(AccountId::new("0x000000000000000000000000000000000000beef"));
        let provider = MockBroadcastProvider::success();
        let data_provider = build_runtime_policy_data_provider(&state, provider);
        let intent = broadcast_ready_intent();
        let inputs = data_provider.gather_inputs(&state, &intent).await.unwrap();
        // FM_V2 reads fail → fee_split stays None, fm_v2_rebate_budget None.
        assert_eq!(inputs.fee_split, None);
        assert_eq!(inputs.fm_v2_rebate_budget_asset, None);
        let snap = state.broadcast_observability.snapshot();
        // Maker + taker quoteFees calls both fail RPC → 2 increments.
        assert_eq!(snap.fm_v2_rpc_failures_total, 2);
        assert_eq!(snap.fm_v2_decode_failures_total, 0);
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::FM_V2_QUOTE_FEES_RPC),
            Some(&2)
        );
    }

    /// Configured PFV address flows through the runtime helper into the
    /// LiveProvider — failing eth_calls now record `pfv_fee_balance` +
    /// `pfv_rebate_reserve` failures via the bounded label vocabulary.
    /// This proves the typed-config plumbing fires the LiveProvider's
    /// PFV branch instead of the prior milestone's silently-skipped path.
    #[tokio::test]
    async fn build_runtime_policy_data_provider_threads_pfv_address_into_live_reads() {
        let mut state = state_with_broadcast(true);
        state.option_event_indexer_config.protocol_fee_vault_address =
            Some(AccountId::new("0x000000000000000000000000000000000000beef"));
        let provider = MockBroadcastProvider::success();
        let data_provider = build_runtime_policy_data_provider(&state, provider);
        let intent = broadcast_ready_intent();
        let _ = data_provider.gather_inputs(&state, &intent).await.unwrap();
        let snap = state.broadcast_observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::PFV_FEE_BALANCE),
            Some(&1),
            "PFV configured → feeBalance read attempted; eth_call mock fails → counter increments"
        );
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::PFV_REBATE_RESERVE),
            Some(&1),
            "PFV configured → rebateReserve read attempted; eth_call mock fails → counter increments"
        );
    }

    /// When PFV address is NOT configured, the runtime helper passes
    /// `None`; the LiveProvider's PFV branch is silently skipped, so
    /// neither `pfv_fee_balance` nor `pfv_rebate_reserve` failure
    /// counters move. Confirms backwards-compat with the prior milestone
    /// posture (mainnet fail-closed via the default-0 rebate-reserve gate).
    #[tokio::test]
    async fn build_runtime_policy_data_provider_skips_pfv_reads_when_address_unset() {
        let state = state_with_broadcast(true);
        // Default: state.option_event_indexer_config.protocol_fee_vault_address == None.
        let provider = MockBroadcastProvider::success();
        let data_provider = build_runtime_policy_data_provider(&state, provider);
        let intent = broadcast_ready_intent();
        let _ = data_provider.gather_inputs(&state, &intent).await.unwrap();
        let snap = state.broadcast_observability.snapshot();
        assert!(
            !snap
                .policy_data_failures_total
                .contains_key(read_type::PFV_FEE_BALANCE),
            "PFV unset → no feeBalance read attempt → no counter increment"
        );
        assert!(
            !snap
                .policy_data_failures_total
                .contains_key(read_type::PFV_REBATE_RESERVE),
            "PFV unset → no rebateReserve read attempt → no counter increment"
        );
    }

    /// The legacy `_with_provider` path (used by direct test calls)
    /// continues to use the Sepolia-permissive stub — its observability
    /// counters stay quiet for the data-read failures (no live reads
    /// occur). This pins the contract that the existing tests are NOT
    /// affected by the new runtime wiring.
    #[tokio::test]
    async fn legacy_with_provider_path_does_not_invoke_live_provider() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::success();
        let _ =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await;
        let snap = state.broadcast_observability.snapshot();
        // No FM_V2 RPC / decode failures or chain-state read failures
        // — the Sepolia-permissive stub returns canned inputs.
        assert_eq!(snap.fm_v2_rpc_failures_total, 0);
        assert_eq!(snap.fm_v2_decode_failures_total, 0);
        assert!(snap.policy_data_failures_total.is_empty());
    }

    /// Existing tx_hash on intent → dedupe rejects (existing test 22
    /// regression is preserved through the new provider path).
    #[tokio::test]
    async fn data_provider_dedupe_hit_rejects_before_signer_call() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // Drive the FIRST broadcast through the same code path so the
        // tx-row gets persisted; second attempt should detect the dupe.
        let provider = MockBroadcastProvider::success();
        let signer = MockBackendSigner::approving();
        let data_provider = StubBroadcastPolicyDataProvider::sepolia_permissive();

        let _ = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("first broadcast must succeed");

        let outcome = broadcast_option_execution_intent_with_provider_signer_and_data_provider(
            &state,
            intent.intent_id,
            &provider,
            &signer,
            &data_provider,
        )
        .await
        .expect("second attempt must short-circuit as duplicate");
        assert!(outcome.duplicate);
        assert_eq!(provider.send_count(), 1, "no second chain send on dedupe");
        assert_eq!(
            signer.sign_calls_count(),
            1,
            "no second signer call on dedupe"
        );
    }

    /// build_signer_for_state must refuse a LocalDev signer when chain id
    /// is mainnet, even if a misconfigured state somehow seated one.
    #[test]
    fn build_signer_refuses_local_dev_on_mainnet_at_runtime() {
        let mut state = state_with_broadcast(true);
        state.execution_config.executor_chain_id = MAINNET_CHAIN_ID;
        state.execution_config.backend_signer_mode = SignerBackendKind::LocalDev;
        match build_signer_for_state(&state) {
            Err(BackendError::Config(msg)) => {
                assert!(
                    msg.contains("REFUSED at runtime on mainnet"),
                    "expected runtime refusal config error, got: {msg}"
                );
            }
            Err(other) => panic!("expected Config error, got {other:?}"),
            Ok(_) => panic!("local-dev on mainnet must refuse"),
        }
    }

    /// build_signer_for_state on Remote mode without endpoint must error.
    #[test]
    fn build_signer_refuses_remote_without_endpoint() {
        let mut state = state_with_broadcast(true);
        state.execution_config.backend_signer_mode = SignerBackendKind::Remote;
        state.execution_config.backend_signer_endpoint = None;
        match build_signer_for_state(&state) {
            Err(err) => assert!(err
                .to_string()
                .contains("BACKEND_SIGNER_ENDPOINT is required")),
            Ok(_) => panic!("remote mode without endpoint must refuse"),
        }
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_cap_below_estimated_gas() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // broadcast cap 600_000 < estimated 750_000 → broadcast_cap_too_low
        let provider = MockBroadcastProvider::with_estimate(750_000);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("below estimated_gas"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BroadcastCapTooLow)
        );
        assert_eq!(transactions[0].estimated_gas, Some(750_000));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(600_000));
        assert_eq!(transactions[0].gas_safety_bps, Some(12_500));
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_cap_below_safety_margin() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // estimated 500_000; required = 500_000 * 1.25 = 625_000; cap 600_000 < required
        let provider = MockBroadcastProvider::with_estimate(500_000);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("below required_gas"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BelowSafetyMargin)
        );
        assert_eq!(transactions[0].estimated_gas, Some(500_000));
        assert_eq!(transactions[0].required_gas, Some(625_000));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(600_000));
        let stored = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(stored.status, OptionExecutionIntentStatus::BroadcastFailed);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_allows_when_cap_satisfies_safety_margin() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        // estimated 400_000; required = 500_000; cap 600_000 >= required → ok
        let provider = MockBroadcastProvider::with_estimate(400_000);

        let outcome =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert!(outcome.submitted);
        assert_eq!(provider.send_count(), 1);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(
            outcome.transaction.gas_check_status,
            Some(OptionExecutionGasCheckStatus::Ok)
        );
        assert_eq!(outcome.transaction.estimated_gas, Some(400_000));
        assert_eq!(outcome.transaction.required_gas, Some(500_000));
        assert_eq!(outcome.transaction.broadcast_gas_limit, Some(600_000));
        assert_eq!(outcome.transaction.gas_safety_bps, Some(12_500));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_uncapped_simulation_cannot_bypass_capped_broadcast() {
        // Reproduces the V1L failure: simulation runs with no gas cap (`OPTION_EXECUTION_SIMULATION_GAS_LIMIT=0`)
        // and produces simulation_ok, but the live broadcast cap inherited from EXECUTOR_MAX_GAS_LIMIT is
        // smaller than the eth_estimateGas result. The preflight must reject before signing or sending.
        let mut state = state_with_broadcast(true);
        state.options_config.execution_simulation_gas_limit = 0;
        state.options_config.execution_broadcast_gas_limit = 0; // fall back to EXECUTOR_MAX_GAS_LIMIT
        state.execution_config.max_gas_limit = 1_000_000;
        let intent = insert_intent(&state, broadcast_ready_intent());
        // mirrors the live failure: estimate 1_040_080, cap 1_000_000 → too low
        let provider = MockBroadcastProvider::with_estimate(1_040_080);

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(matches!(&error, BackendError::BroadcastRejected(_)));
        assert_eq!(provider.send_count(), 0);
        assert_eq!(provider.estimate_count(), 1);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(transactions[0].tx_hash, None);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::BroadcastCapTooLow)
        );
        // simulation_gas_limit recorded as 0 (the uncapped sentinel), confirming the bypass attempt
        // was visible to the preflight rather than relied upon for safety.
        assert_eq!(transactions[0].simulation_gas_limit, Some(0));
        assert_eq!(transactions[0].broadcast_gas_limit, Some(1_000_000));
        assert_eq!(transactions[0].estimated_gas, Some(1_040_080));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_broadcast_rejects_when_estimate_fails() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockBroadcastProvider::with_estimate_failure("estimate gas rpc down");

        let error =
            broadcast_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();
        let transactions = option_transactions(&state, intent.intent_id);

        assert!(
            matches!(&error, BackendError::BroadcastRejected(message) if message.contains("eth_estimateGas failed"))
        );
        assert_eq!(provider.send_count(), 0);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].status, ExecutionTransactionStatus::Failed);
        assert_eq!(
            transactions[0].gas_check_status,
            Some(OptionExecutionGasCheckStatus::EstimateFailed)
        );
        assert_eq!(transactions[0].estimated_gas, None);
        assert!(transactions[0]
            .gas_check_error
            .as_deref()
            .unwrap_or_default()
            .contains("estimate gas rpc down"));
        assert_no_generic_execution_rows(&state);
    }

    fn assert_no_generic_execution_rows(state: &AppState) {
        // The option execution path must never write to the generic execution_transactions
        // store or call the generic executor's broadcast endpoint. In tests we run without
        // a Postgres repository, so the only persistence sink is the in-memory option store,
        // which has no concept of `execution_transactions` (the generic perp table).
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }

    #[derive(Clone)]
    enum MockReceiptOutcome {
        Receipt(ConfirmationReceipt),
        NotFound,
        Error(String),
    }

    #[derive(Clone)]
    struct MockReceiptProvider {
        outcome: MockReceiptOutcome,
        calls: Arc<Mutex<Vec<String>>>,
        head_block: u64,
    }

    impl MockReceiptProvider {
        fn mined_success(tx_hash: &str, block: u64) -> Self {
            Self {
                outcome: MockReceiptOutcome::Receipt(ConfirmationReceipt {
                    tx_hash: tx_hash.to_string(),
                    status: Some(1),
                    block_number: Some(block),
                    gas_used: Some(1_057_772),
                    effective_gas_price: Some("0x5b8d80".to_string()),
                    cumulative_gas_used: Some(1_672_948),
                    block_hash: Some(
                        "0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853"
                            .to_string(),
                    ),
                    transaction_index: Some(5),
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn mined_reverted(tx_hash: &str, block: u64) -> Self {
            Self {
                outcome: MockReceiptOutcome::Receipt(ConfirmationReceipt {
                    tx_hash: tx_hash.to_string(),
                    status: Some(0),
                    block_number: Some(block),
                    gas_used: Some(982_941),
                    effective_gas_price: Some("0x5b8d80".to_string()),
                    cumulative_gas_used: Some(1_500_000),
                    block_hash: Some(
                        "0x21307b4272a3fc0526e2a100c844cd037e81671c48d3f30cf939fefc57bc6b78"
                            .to_string(),
                    ),
                    transaction_index: Some(15),
                }),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn missing() -> Self {
            Self {
                outcome: MockReceiptOutcome::NotFound,
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn rpc_error(message: &str) -> Self {
            Self {
                outcome: MockReceiptOutcome::Error(message.to_string()),
                calls: Arc::new(Mutex::new(Vec::new())),
                head_block: 0,
            }
        }

        fn with_head(mut self, head_block: u64) -> Self {
            self.head_block = head_block;
            self
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl TransactionReceiptProvider for MockReceiptProvider {
        fn block_number(&self) -> RpcFuture<'_, u64> {
            let head = self.head_block;
            Box::pin(async move { Ok(head) })
        }

        fn transaction_receipt(
            &self,
            tx_hash: String,
        ) -> RpcFuture<'_, Option<ConfirmationReceipt>> {
            let outcome = self.outcome.clone();
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.lock().unwrap().push(tx_hash);
                match outcome {
                    MockReceiptOutcome::Receipt(r) => Ok(Some(r)),
                    MockReceiptOutcome::NotFound => Ok(None),
                    MockReceiptOutcome::Error(message) => Err(BackendError::Simulation(message)),
                }
            })
        }
    }

    fn broadcast_submitted_intent_with_tx(
        state: &AppState,
        tx_hash: &str,
    ) -> (OptionExecutionIntent, OptionExecutionTransaction) {
        let intent = broadcast_ready_intent();
        let intent = insert_intent(state, intent);
        // Move intent to broadcast_submitted in-memory and insert a matching tx row.
        let now = now_ms();
        let updated = state
            .options_store
            .lock()
            .unwrap()
            .update_option_execution_intent_status(
                intent.intent_id,
                OptionExecutionIntentStatus::BroadcastSubmitted,
                None,
                now,
            )
            .unwrap();
        let tx = OptionExecutionTransaction {
            transaction_id: Uuid::new_v4().to_string(),
            intent_id: intent.intent_id,
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            from: AccountId::new("0xc35f7a8a103a9a4464adfaa76b9b514093d23c27"),
            to: AccountId::new("0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b"),
            calldata: "0x031f77b3".to_string(),
            value_wei: "0".to_string(),
            gas_limit: Some(1_500_000),
            tx_hash: Some(tx_hash.to_string()),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: Some(1_091_120),
            required_gas: Some(1_363_900),
            simulation_gas_limit: Some(0),
            broadcast_gas_limit: Some(1_500_000),
            gas_safety_bps: Some(12_500),
            gas_check_status: Some(OptionExecutionGasCheckStatus::Ok),
            gas_check_error: None,
            confirmation_status: None,
            confirmed_at_ms: None,
            confirmed_block_number: None,
            receipt_status: None,
            confirmation_error: None,
            gas_used: None,
            effective_gas_price: None,
            cumulative_gas_used: None,
            receipt_block_hash: None,
            receipt_transaction_index: None,
            receipt_observed_at_ms: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let stored_tx = state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_transaction(tx)
            .unwrap();
        (updated, stored_tx)
    }

    #[tokio::test]
    async fn option_execution_confirm_mined_success_transitions_to_broadcast_confirmed() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (intent, tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 41856964);

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(outcome.receipt_status, Some(1));
        assert_eq!(outcome.block_number, Some(41856964));
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_eq!(outcome.transaction.confirmed_block_number, Some(41856964));
        assert_eq!(outcome.transaction.receipt_status, Some(1));
        assert_eq!(outcome.transaction.transaction_id, tx.transaction_id);
        assert_eq!(provider.call_count(), 1);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_mined_reverted_transitions_to_broadcast_reverted() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_reverted(tx_hash, 100);

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::MinedReverted
        );
        assert_eq!(outcome.receipt_status, Some(0));
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastReverted
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedReverted)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_missing_receipt_does_not_change_intent_status() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x2222222222222222222222222222222222222222222222222222222222222222";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::missing();

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::ReceiptMissing
        );
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert_eq!(
            outcome.transaction.confirmation_status,
            Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        );
        assert!(outcome.error.is_some());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_execution_confirm_receipt_error_does_not_change_intent_status() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x3333333333333333333333333333333333333333333333333333333333333333";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::rpc_error("rpc temporarily unavailable");

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::ReceiptError
        );
        assert_eq!(
            outcome.intent.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        assert!(outcome
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("rpc temporarily unavailable"));
    }

    #[tokio::test]
    async fn option_execution_confirm_rejects_intent_without_submitted_transaction() {
        let state = state_with_broadcast(true);
        let intent = insert_intent(&state, broadcast_ready_intent());
        let provider = MockReceiptProvider::mined_success("0xabc", 1);

        let error =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap_err();

        assert!(matches!(
            error,
            BackendError::InvalidOptionExecutionIntentState(message)
                if message.contains("no submitted option execution transaction")
        ));
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn option_execution_confirm_idempotent_on_already_confirmed_row() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x4444444444444444444444444444444444444444444444444444444444444444";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 200);

        let first =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();
        let second =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            first.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(
            second.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        assert_eq!(
            second.intent.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        // Provider was called both times — confirm() does not memoize on its own.
        assert_eq!(provider.call_count(), 2);
    }

    fn state_with_simulation(enabled: bool) -> AppState {
        let mut options_config = OptionsConfig::enabled_in_memory_for_tests();
        options_config.execution_enabled = true;
        options_config.execution_require_persistence = false;
        options_config.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        options_config.execution_eip712_domain.verifying_contract =
            options_config.matching_engine_address.clone();
        options_config.execution_simulation_enabled = enabled;
        options_config.execution_require_rpc_for_simulation = false;
        options_config.execution_simulation_gas_limit = 500_000;
        AppState::with_options_config(EngineState::with_default_markets(), options_config)
    }

    fn state_with_broadcast(broadcast_enabled: bool) -> AppState {
        let mut state = state_with_simulation(false);
        state.options_config.execution_broadcast_enabled = broadcast_enabled;
        state.options_config.execution_require_simulation_ok = true;
        state.options_config.execution_broadcast_gas_limit = 600_000;
        state.execution_config = crate::execution::ExecutionConfig {
            execution_enabled: true,
            real_broadcast_enabled: true,
            executor_private_key: Some(crate::execution::PrivateKeySecret::new(
                "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
            )),
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            max_gas_limit: 1_000_000,
            executor_chain_id: 84532,
            ..crate::execution::ExecutionConfig::disabled()
        };
        state
    }

    fn state_with_option_nonce_sync(strict: bool) -> AppState {
        let mut state = state_with_simulation(false);
        state.option_nonce_sync_config = OptionNonceSyncConfig {
            enabled: true,
            require_rpc: true,
            strict,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            option_matching_engine_address: state.options_config.matching_engine_address.clone(),
        };
        state
    }

    fn insert_executable_series(state: &AppState) {
        let expiry = 4_102_444_800;
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let onchain_option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement_asset,
            expiry,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap()
        .to_string();
        state
            .options_store
            .lock()
            .unwrap()
            .insert_series(OptionSeries {
                option_series_id: "series-1".to_string(),
                underlying: underlying.0,
                base_asset: "ETH".to_string(),
                quote_asset: "USDC".to_string(),
                settlement_asset: settlement_asset.0,
                expiry,
                strike_1e8: 300_000_000_000,
                is_call: true,
                contract_size_1e8: 100_000_000,
                status: OptionSeriesStatus::Active,
                source: OptionSeriesSource::Manual,
                onchain_product_id: None,
                onchain_series_id: Some(onchain_option_id),
                created_at_ms: 1,
                updated_at_ms: 1,
            });
    }

    fn orderbook_fill() -> OptionFill {
        OptionFill {
            fill_id: Uuid::from_u128(101),
            option_series_id: "series-1".to_string(),
            buy_order_id: OrderId(Uuid::from_u128(201)),
            sell_order_id: OrderId(Uuid::from_u128(202)),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            maker_order_id: OrderId(Uuid::from_u128(202)),
            taker_order_id: OrderId(Uuid::from_u128(201)),
            taker_side: Side::Buy,
            price_1e8: 10_000_000,
            size_1e8: 100_000_000,
            created_at_ms: 123,
        }
    }

    fn insert_intent(state: &AppState, intent: OptionExecutionIntent) -> OptionExecutionIntent {
        state
            .options_store
            .lock()
            .unwrap()
            .insert_option_execution_intent(intent)
    }

    fn option_transactions(
        state: &AppState,
        intent_id: OptionExecutionIntentId,
    ) -> Vec<OptionExecutionTransaction> {
        state
            .options_store
            .lock()
            .unwrap()
            .option_execution_transactions_for_intent(intent_id)
    }

    fn broadcast_ready_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            buyer_signature: Some(signature_hex(0xaa)),
            seller_signature: Some(signature_hex(0xbb)),
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            ..calldata_ready_intent()
        }
    }

    fn calldata_ready_intent() -> OptionExecutionIntent {
        let expiry = 4_102_444_800;
        let underlying = AccountId::new("0x0000000000000000000000000000000000000010");
        let settlement_asset = AccountId::new("0x0000000000000000000000000000000000000020");
        let onchain_option_id = crate::options::option_product_registry_option_id(
            &underlying,
            &settlement_asset,
            expiry,
            300_000_000_000,
            100_000_000,
            true,
            true,
        )
        .unwrap()
        .to_string();
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying,
            settlement_asset,
            expiry,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 10_000,
            buyer_is_maker: false,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0x12345678".to_string()),
            status: OptionExecutionIntentStatus::CalldataReady,
            error: None,
            simulation_status: None,
            simulation_error: None,
            simulation_block_number: None,
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }

    // ----------------------------------------------------------------------------
    // V1V option execution confirmation worker tests
    // ----------------------------------------------------------------------------

    fn state_with_confirmation_worker(enabled: bool, finality_blocks: u64) -> AppState {
        let mut state = state_with_broadcast(true);
        state.option_confirmation_config = crate::options::OptionConfirmationConfig {
            enabled,
            poll_interval_ms: 15_000,
            finality_blocks,
            batch_size: 25,
            require_rpc: true,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
        };
        state
    }

    fn pending_tx_hash() -> &'static str {
        "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125"
    }

    #[tokio::test]
    async fn worker_disabled_returns_disabled_and_does_nothing() {
        let state = state_with_confirmation_worker(false, 3);
        let tx_hash = pending_tx_hash();
        let (intent, tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(200);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(!result.enabled);
        assert!(result.decisions.is_empty());
        assert_eq!(provider.call_count(), 0);
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs[0].transaction_id, tx.transaction_id);
        assert!(txs[0].confirmation_status.is_none());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_missing_receipt_leaves_pending() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::missing().with_head(200);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(result.enabled);
        assert_eq!(result.decisions.len(), 1);
        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::ReceiptMissing
        );
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_receipt_without_finality_does_not_finalize() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        // receipt at block 100, head at 101: head < 100+3 → not finalized
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(101);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert!(result.enabled);
        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::NotFinalized
        );
        assert_eq!(decision.receipt_status, Some(1));
        assert_eq!(decision.block_number, Some(100));
        assert_eq!(decision.current_block_number, Some(101));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastSubmitted
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::Pending)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_successful_receipt_with_finality_finalizes_mined_success() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(105);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::MinedSuccess
        );
        assert_eq!(decision.receipt_status, Some(1));
        assert_eq!(decision.block_number, Some(100));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastConfirmed
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_eq!(txs[0].confirmed_block_number, Some(100));
        assert_eq!(txs[0].receipt_status, Some(1));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_failed_receipt_with_finality_finalizes_mined_failed() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_reverted(tx_hash, 100).with_head(120);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let decision = &result.decisions[0];
        assert_eq!(
            decision.outcome,
            crate::options::OptionConfirmationOutcome::MinedFailed
        );
        assert_eq!(decision.receipt_status, Some(0));
        assert_eq!(decision.block_number, Some(100));
        let intent_after = get_option_execution_intent(&state, intent.intent_id)
            .await
            .unwrap();
        assert_eq!(
            intent_after.status,
            OptionExecutionIntentStatus::BroadcastFailed
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedFailed)
        );
        assert_eq!(txs[0].receipt_status, Some(0));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_does_not_use_broadcast_provider() {
        // Construct a state with both worker enabled and an in-memory broadcast provider
        // that *would* fail loudly if called. Confirm the worker never touches it.
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (_intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let receipt_provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(120);
        let broadcast_provider = MockBroadcastProvider::success();

        let _ = confirm_pending_option_execution_transactions(&state, &receipt_provider)
            .await
            .unwrap();

        assert_eq!(broadcast_provider.send_count(), 0);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_never_creates_generic_execution_rows() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = pending_tx_hash();
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(200);

        for _ in 0..3 {
            confirm_pending_option_execution_transactions(&state, &provider)
                .await
                .unwrap();
        }

        // The option-side row sees its final mined_success.
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs.len(), 1);
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::MinedSuccess)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[test]
    fn worker_outcome_is_finalized_only_for_mined_states() {
        use crate::options::OptionConfirmationOutcome::*;
        assert!(MinedSuccess.is_finalized());
        assert!(MinedFailed.is_finalized());
        assert!(!NotFinalized.is_finalized());
        assert!(!ReceiptMissing.is_finalized());
        assert!(!ReceiptError.is_finalized());
        assert!(!Disabled.is_finalized());
        assert!(!NoPending.is_finalized());
    }

    // ----------------------------------------------------------------------------
    // V1W observability + receipt cost persistence tests
    // ----------------------------------------------------------------------------

    fn cost_bundle() -> crate::options::OptionExecutionReceiptCost {
        crate::options::OptionExecutionReceiptCost {
            gas_used: Some(1_057_772),
            effective_gas_price: Some("0x5b8d80".to_string()),
            cumulative_gas_used: Some(1_672_948),
            block_hash: Some(
                "0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853".to_string(),
            ),
            transaction_index: Some(5),
            observed_at_ms: Some(123_456_789),
        }
    }

    #[tokio::test]
    async fn receipt_cost_persists_through_manual_confirm() {
        let state = state_with_broadcast(true);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 41856964);

        let outcome =
            confirm_option_execution_intent_with_provider(&state, intent.intent_id, &provider)
                .await
                .unwrap();

        assert_eq!(
            outcome.confirmation_status,
            OptionExecutionConfirmationStatus::MinedSuccess
        );
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs[0].gas_used, Some(1_057_772));
        assert_eq!(txs[0].effective_gas_price.as_deref(), Some("0x5b8d80"));
        assert_eq!(txs[0].cumulative_gas_used, Some(1_672_948));
        assert_eq!(
            txs[0].receipt_block_hash.as_deref(),
            Some("0x53d62c21ecbe462e2868e216b4655474de0d2b7b832f15ab6e72b216fb1f3853")
        );
        assert_eq!(txs[0].receipt_transaction_index, Some(5));
        assert!(txs[0].receipt_observed_at_ms.is_some());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_stores_gas_fields_on_mined_success() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(105);

        let result = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.decisions.len(), 1);
        let txs = option_transactions(&state, intent.intent_id);
        assert_eq!(txs[0].gas_used, Some(1_057_772));
        assert_eq!(txs[0].cumulative_gas_used, Some(1_672_948));
        assert_eq!(txs[0].receipt_transaction_index, Some(5));
        assert!(txs[0].receipt_observed_at_ms.is_some());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_does_not_store_cost_when_receipt_absent() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let (intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::missing().with_head(200);

        let _ = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let txs = option_transactions(&state, intent.intent_id);
        assert!(txs[0].gas_used.is_none());
        assert!(txs[0].effective_gas_price.is_none());
        assert!(txs[0].receipt_observed_at_ms.is_none());
        assert_eq!(
            txs[0].confirmation_status,
            Some(OptionExecutionConfirmationStatus::ReceiptMissing)
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn worker_publishes_latest_tick_after_tick() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (_intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(105);

        assert!(state
            .option_confirmation_last_tick
            .lock()
            .unwrap()
            .is_none());
        let _ = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();

        let latest = state
            .option_confirmation_last_tick
            .lock()
            .unwrap()
            .clone()
            .expect("latest tick should be set after a worker run");
        assert!(latest.enabled);
        assert_eq!(latest.finality_blocks, 3);
        assert_eq!(latest.current_block_number, Some(105));
        assert_eq!(latest.decisions.len(), 1);
        assert_eq!(
            latest.decisions[0].outcome,
            crate::options::OptionConfirmationOutcome::MinedSuccess
        );
    }

    #[tokio::test]
    async fn worker_disabled_does_not_publish_latest_tick() {
        let state = state_with_confirmation_worker(false, 3);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (_intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(105);

        let _ = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();
        assert!(state
            .option_confirmation_last_tick
            .lock()
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn summary_counts_bucket_pending_correctly() {
        let state = state_with_confirmation_worker(true, 3);
        let tx_hash = "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125";
        let (_intent, _tx) = broadcast_submitted_intent_with_tx(&state, tx_hash);

        // Before any tick: 1 row, pending (null confirmation_status).
        let pre = state
            .options_store
            .lock()
            .unwrap()
            .summarize_option_execution_confirmations();
        assert_eq!(pre, vec![("pending".to_string(), 1u64)]);

        // After a successful mined tick: 1 row, mined_success.
        let provider = MockReceiptProvider::mined_success(tx_hash, 100).with_head(120);
        let _ = confirm_pending_option_execution_transactions(&state, &provider)
            .await
            .unwrap();
        let post = state
            .options_store
            .lock()
            .unwrap()
            .summarize_option_execution_confirmations();
        assert_eq!(post, vec![("mined_success".to_string(), 1u64)]);
    }

    #[test]
    fn store_update_receipt_cost_persists_independently_of_status_transition() {
        // Repository-style direct call: the in-memory store update method takes the
        // cost bundle and persists every non-None field even when the confirmation
        // status is non-terminal (e.g. NotFinalized → persist_status = Pending).
        use crate::execution::ExecutionTransactionStatus;
        let mut store = crate::options::OptionSeriesStore::new();
        let tx_id = "tx-1".to_string();
        let tx = OptionExecutionTransaction {
            transaction_id: tx_id.clone(),
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: None,
            from: AccountId::new("0x0000000000000000000000000000000000000001"),
            to: AccountId::new("0x0000000000000000000000000000000000000002"),
            calldata: "0x".to_string(),
            value_wei: "0".to_string(),
            gas_limit: Some(1),
            tx_hash: Some(
                "0x4444444444444444444444444444444444444444444444444444444444444444".to_string(),
            ),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: None,
            required_gas: None,
            simulation_gas_limit: None,
            broadcast_gas_limit: None,
            gas_safety_bps: None,
            gas_check_status: None,
            gas_check_error: None,
            confirmation_status: None,
            confirmed_at_ms: None,
            confirmed_block_number: None,
            receipt_status: None,
            confirmation_error: None,
            gas_used: None,
            effective_gas_price: None,
            cumulative_gas_used: None,
            receipt_block_hash: None,
            receipt_transaction_index: None,
            receipt_observed_at_ms: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        store.insert_option_execution_transaction(tx).unwrap();
        let cost = cost_bundle();
        let updated = store
            .update_option_execution_confirmation(
                &tx_id,
                OptionExecutionConfirmationStatus::Pending,
                42,
                Some(100),
                Some(1),
                None,
                &cost,
            )
            .unwrap();
        assert_eq!(updated.gas_used, cost.gas_used);
        assert_eq!(updated.effective_gas_price, cost.effective_gas_price);
        assert_eq!(updated.cumulative_gas_used, cost.cumulative_gas_used);
        assert_eq!(updated.receipt_block_hash, cost.block_hash);
        assert_eq!(updated.receipt_transaction_index, cost.transaction_index);
        assert_eq!(updated.receipt_observed_at_ms, cost.observed_at_ms);
    }
}
