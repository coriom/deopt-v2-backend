//! PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — Perps WS lifecycle
//! emitters.
//!
//! Called from the internal execution service AFTER a successful
//! in-memory state mutation (V1). A future PG write path will keep
//! the same invariant: emit AFTER the DB commit succeeds.
//!
//! Privacy: every helper hand-populates scalar fields — never a
//! signature, nonce, auth envelope, header, or bearer token.

use crate::api::public_ws::{
    LifecycleChannel, LifecycleEvent, LifecycleEventSender, LifecyclePayload,
};
use crate::error::BackendError;
use crate::perps::orders::{PerpFill, PerpOrder, PerpOrderSide};
use crate::perps::positions::PerpPosition;
use crate::perps::rejection::classify_perp_rejection;
use crate::types::{now_ms, AccountId};

/// Emit a `PerpOrderUpdated` frame for the given order's owner.
/// Routed on `account.perp_orders`.
pub fn emit_perp_order_lifecycle(sender: &LifecycleEventSender, order: &PerpOrder) {
    let now = now_ms();
    sender.emit(LifecycleEvent {
        account: order.account.clone(),
        channel: LifecycleChannel::AccountPerpOrders,
        payload: LifecyclePayload::PerpOrderUpdated {
            order_id: order.id.to_string(),
            market_id: order.market_id.clone(),
            subaccount_id: order.subaccount_id,
            side: order.side.as_str().to_string(),
            status: order.status.as_str().to_string(),
            price_1e8: order.price_1e8.to_string(),
            size_1e8: order.size_1e8.to_string(),
            remaining_size_1e8: order.remaining_size_1e8.to_string(),
            filled_size_1e8: order.filled_size_1e8.to_string(),
            time_in_force: order.time_in_force.as_str().to_string(),
            post_only: order.post_only,
            reduce_only: order.reduce_only,
            client_order_id: order.client_order_id.clone(),
            terminal_reason_code: order.terminal_reason_code.clone(),
            updated_at_ms: order.updated_at_ms,
        },
        emitted_at_ms: now,
    });
}

/// Emit `PerpFillCreated` frames for BOTH taker and maker. Per-side
/// filtering happens on the WS handler where the session's
/// authenticated account gates delivery.
pub fn emit_perp_fill_lifecycle(sender: &LifecycleEventSender, fill: &PerpFill) {
    let now = now_ms();
    // Taker view.
    sender.emit(LifecycleEvent {
        account: fill.taker_account.clone(),
        channel: LifecycleChannel::AccountPerpFills,
        payload: LifecyclePayload::PerpFillCreated {
            fill_id: fill.id.to_string(),
            market_id: fill.market_id.clone(),
            taker_subaccount_id: fill.taker_subaccount_id,
            maker_subaccount_id: fill.maker_subaccount_id,
            order_id: fill.taker_order_id.to_string(),
            counterparty_order_id: fill.maker_order_id.to_string(),
            liquidity_role: "taker".to_string(),
            side: fill.taker_side.as_str().to_string(),
            price_1e8: fill.price_1e8.to_string(),
            size_1e8: fill.size_1e8.to_string(),
            created_at_ms: fill.created_at_ms,
        },
        emitted_at_ms: now,
    });
    // Maker view — opposite side.
    let maker_side = maker_side_for(fill.taker_side);
    sender.emit(LifecycleEvent {
        account: fill.maker_account.clone(),
        channel: LifecycleChannel::AccountPerpFills,
        payload: LifecyclePayload::PerpFillCreated {
            fill_id: fill.id.to_string(),
            market_id: fill.market_id.clone(),
            taker_subaccount_id: fill.taker_subaccount_id,
            maker_subaccount_id: fill.maker_subaccount_id,
            order_id: fill.maker_order_id.to_string(),
            counterparty_order_id: fill.taker_order_id.to_string(),
            liquidity_role: "maker".to_string(),
            side: maker_side.as_str().to_string(),
            price_1e8: fill.price_1e8.to_string(),
            size_1e8: fill.size_1e8.to_string(),
            created_at_ms: fill.created_at_ms,
        },
        emitted_at_ms: now,
    });
}

fn maker_side_for(taker: PerpOrderSide) -> PerpOrderSide {
    taker.opposite()
}

/// Emit `PerpPositionUpdated` for the position's owner.
pub fn emit_perp_position_lifecycle(sender: &LifecycleEventSender, position: &PerpPosition) {
    let now = now_ms();
    sender.emit(LifecycleEvent {
        account: position.account.clone(),
        channel: LifecycleChannel::AccountPerpPositions,
        payload: LifecyclePayload::PerpPositionUpdated {
            position_id: position.id.to_string(),
            market_id: position.market_id.clone(),
            subaccount_id: position.subaccount_id,
            side: position.side.as_str().to_string(),
            size_1e8: position.size_1e8.to_string(),
            entry_price_1e8: position.entry_price_1e8.to_string(),
            margin_1e8: position.margin_1e8.to_string(),
            realized_pnl_1e8: position.realized_pnl_1e8.to_string(),
            status: position.status.as_str().to_string(),
            updated_at_ms: position.updated_at_ms,
        },
        emitted_at_ms: now,
    });
}

/// Convenience: emit the full lifecycle bundle produced by one
/// internal execution submit — the taker order, each fill (with both
/// sides routed by account), and the updated position for every
/// affected account. Callers pass the fresh positions store so we
/// can read the post-fill positions here.
pub fn emit_lifecycle_for_submit_outcome(
    sender: &LifecycleEventSender,
    positions_store: &crate::perps::positions::PerpPositionsStore,
    outcome: &crate::perps::SubmitPerpOrderOutcome,
) {
    emit_perp_order_lifecycle(sender, &outcome.order);
    for fill in &outcome.fills {
        emit_perp_fill_lifecycle(sender, fill);
        // PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — each side of the fill
        // carries its own subaccount id; look up positions with the
        // exact (account, subaccount, market) key so cross-subaccount
        // rows do not leak into another wallet's WS stream.
        let sides: [(&AccountId, u32); 2] = [
            (&fill.taker_account, fill.taker_subaccount_id),
            (&fill.maker_account, fill.maker_subaccount_id),
        ];
        for (account, sub) in sides {
            if let Some(position) = positions_store.get_active(account, sub, &fill.market_id) {
                emit_perp_position_lifecycle(sender, &position);
            } else if let Some(closed) = positions_store
                .list_for_account_and_subaccount(account, sub)
                .into_iter()
                .find(|p| p.market_id == fill.market_id)
            {
                emit_perp_position_lifecycle(sender, &closed);
            }
        }
    }
}

/// Emit `PerpOrderRejected` for a classified rejection. Callers pass
/// the input they attempted to submit so the payload can carry the
/// same scalar fields the trader would see in their history feed.
#[allow(clippy::too_many_arguments)]
pub fn emit_perp_rejection_lifecycle(
    sender: &LifecycleEventSender,
    account: &AccountId,
    subaccount_id: u32,
    error: &BackendError,
    market_id: Option<String>,
    side: Option<PerpOrderSide>,
    price_1e8: Option<u128>,
    size_1e8: Option<u128>,
    time_in_force: Option<String>,
    post_only: Option<bool>,
    reduce_only: Option<bool>,
    client_order_id: Option<String>,
) {
    let Some((reason_code, reason_source)) = classify_perp_rejection(error) else {
        return;
    };
    let now = now_ms();
    sender.emit(LifecycleEvent {
        account: account.clone(),
        channel: LifecycleChannel::AccountPerpOrders,
        payload: LifecyclePayload::PerpOrderRejected {
            market_id,
            subaccount_id,
            side: side.map(|s| s.as_str().to_string()),
            price_1e8: price_1e8.map(|p| p.to_string()),
            size_1e8: size_1e8.map(|s| s.to_string()),
            time_in_force,
            post_only,
            reduce_only,
            client_order_id,
            reason_code: reason_code.to_string(),
            reason_message: Some(error.to_string()),
            reason_source: reason_source.to_string(),
            created_at_ms: now,
        },
        emitted_at_ms: now,
    });
}
