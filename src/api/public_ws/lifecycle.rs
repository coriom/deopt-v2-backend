//! ORDER-LIFECYCLE-OBSERVABILITY-V1 — lifecycle event broadcast.
//!
//! A single process-wide `tokio::sync::broadcast::Sender<LifecycleEvent>`
//! lives on `AppState::lifecycle_events`. Services emit lifecycle events
//! AFTER their DB commit succeeds; per-session WS listeners receive them
//! and forward only the events matching their authenticated `account`
//! AND a currently-active subscription channel.
//!
//! Design properties:
//!
//!  * **Commit-safe**: emitters call `LifecycleEventSender::emit_after_commit`
//!    only after the service's `Result::Ok` arm — a rolled-back txn never
//!    produces an event.
//!  * **Best-effort**: a full or absent receiver is logged but not
//!    propagated; the periodic snapshot ticker and REST snapshot are the
//!    canonical recovery surface for missed events.
//!  * **Privacy**: events carry only the resource_id, status, timestamps,
//!    and lightweight scalar fields — never a signature, never a write-auth
//!    nonce, never a raw envelope.
//!  * **Disjoint from WS auth**: this `broadcast::Sender` is a separate
//!    primitive from the EIP-191 challenge/verify nonce table; a wire
//!    event cannot be replayed back into the auth handshake and vice
//!    versa.

use crate::types::{AccountId, TimestampMs};
use serde::{Deserialize, Serialize};

/// One lifecycle event published by a backend service. The `channel`
/// field tells the WS dispatcher which channel ID to put on the frame
/// (e.g. `account.orders` → `Channel::AccountOrders`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleEvent {
    /// The account this event is about. WS sessions only forward events
    /// where `event.account == session.account`.
    pub account: AccountId,
    /// The WS channel this event belongs to. Must be one of
    /// `account.orders`, `account.fills`, `account.conditional_orders`.
    pub channel: LifecycleChannel,
    /// The lifecycle payload.
    pub payload: LifecyclePayload,
    /// Server-side wall-clock timestamp at emission.
    pub emitted_at_ms: TimestampMs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleChannel {
    AccountOrders,
    AccountFills,
    AccountConditionalOrders,
    // PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — distinct Perps channels
    // so consumers can subscribe to Perps traffic without also
    // receiving Options order/fill deltas. Perp positions get their
    // own channel too so a UI showing PnL / margin ratio can
    // subscribe just to that stream.
    AccountPerpOrders,
    AccountPerpFills,
    AccountPerpPositions,
    /// PERPS-FUNDING-V1 — funding payment lifecycle. Kept as its own
    /// channel so consumers only interested in funding history don't
    /// have to filter every position update. Emitted alongside a
    /// `PerpPositionUpdated` on `AccountPerpPositions` when margin
    /// changed.
    AccountPerpFunding,
}

impl LifecycleChannel {
    pub fn ws_channel_str(self) -> &'static str {
        match self {
            Self::AccountOrders => "account.orders",
            Self::AccountFills => "account.fills",
            Self::AccountConditionalOrders => "account.conditional_orders",
            Self::AccountPerpOrders => "account.perp_orders",
            Self::AccountPerpFills => "account.perp_fills",
            Self::AccountPerpPositions => "account.perp_positions",
            Self::AccountPerpFunding => "account.perp_funding",
        }
    }
}

/// Discriminated payload. Each variant maps to one user-facing
/// lifecycle moment. Fields are intentionally small + scalar to avoid
/// shipping the full resource shape on every delta — the WS client
/// has the canonical snapshot via `*.snapshot` and the resource id to
/// look up details if needed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecyclePayload {
    /// A direct option order's status moved. `status` is the
    /// `OptionOrderStatusValue` wire string (e.g. `"open"`,
    /// `"partially_filled"`, `"filled"`, `"cancelled"`, `"rejected"`).
    OrderUpdated {
        order_id: String,
        option_series_id: String,
        status: String,
        remaining_size_1e8: String,
        size_1e8: String,
    },
    /// A new option fill landed for this account (as buyer or seller).
    FillCreated {
        fill_id: String,
        option_series_id: String,
        order_id: String,
        side: String,
        price_1e8: String,
        size_1e8: String,
        created_at_ms: TimestampMs,
    },
    /// A conditional order (TP/SL) state changed.
    ConditionalOrderUpdated {
        conditional_order_id: String,
        option_series_id: String,
        status: String,
        child_order_id: Option<String>,
        oco_group_id: Option<String>,
        failure_code: Option<String>,
    },
    /// ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — a submit attempt was
    /// classified as trader-facing rejected and persisted in the
    /// rejected-attempts feed. Emitted AFTER durable persistence so
    /// a receiver observing this event can trust the row is on disk.
    /// Routed on `account.orders`.
    ///
    /// Privacy: never carries signatures, write-auth nonces, raw
    /// envelope, headers, or bearer tokens. `reason_message` is the
    /// same trader-meaningful `BackendError::Display` string that is
    /// persisted in the rejection row.
    OrderRejected {
        rejection_id: String,
        option_series_id: Option<String>,
        side: Option<String>,
        price_1e8: Option<String>,
        size_1e8: Option<String>,
        time_in_force: Option<String>,
        post_only: Option<bool>,
        client_order_id: Option<String>,
        reason_code: String,
        reason_message: Option<String>,
        reason_source: String,
        created_at_ms: TimestampMs,
    },
    /// ACCOUNT-LIFECYCLE-REALTIME-GAPS-V2 — an attached TP/SL plan
    /// transitioned. Emitted AFTER durable persistence at:
    ///   * pending create,
    ///   * active materialization,
    ///   * active resize (materialized_size_1e8 increased),
    ///   * cancelled (parent terminated before fill),
    ///   * failed (materialization errored, parent unaffected).
    ///
    /// Routed on `account.conditional_orders` because the plan is
    /// the container of the conditional-order legs; consumers that
    /// already track the conditional-orders channel get both leg
    /// deltas and plan transitions on one subscription.
    AttachmentPlanUpdated {
        plan_id: String,
        parent_order_id: String,
        option_series_id: String,
        status: String,
        materialized_size_1e8: Option<String>,
        tp_conditional_order_id: Option<String>,
        sl_conditional_order_id: Option<String>,
        oco_group_id: Option<String>,
        failure_code: Option<String>,
        failure_message: Option<String>,
        updated_at_ms: TimestampMs,
    },
    // PERPS-PERSISTENCE-HISTORY-LIFECYCLE-V1 — Perps order lifecycle.
    // Emitted AFTER an in-memory `PerpOrderStore` mutation lands (V1
    // uses the in-memory ledger; a future PG write path will emit
    // AFTER the DB commit under the same invariant). Routed on
    // `account.perp_orders`. Never carries signatures, nonces, auth
    // envelopes, headers, or bearer tokens.
    PerpOrderUpdated {
        order_id: String,
        market_id: String,
        side: String,
        status: String,
        price_1e8: String,
        size_1e8: String,
        remaining_size_1e8: String,
        filled_size_1e8: String,
        time_in_force: String,
        post_only: bool,
        reduce_only: bool,
        client_order_id: Option<String>,
        terminal_reason_code: Option<String>,
        updated_at_ms: TimestampMs,
    },
    /// Perps fill created. Routed on `account.perp_fills`. The
    /// requesting account may be either taker or maker; the emitter
    /// fires ONE frame per account (taker + maker) with the account
    /// field routing per the WS handler's per-session filter.
    PerpFillCreated {
        fill_id: String,
        market_id: String,
        order_id: String,
        counterparty_order_id: String,
        liquidity_role: String,
        side: String,
        price_1e8: String,
        size_1e8: String,
        created_at_ms: TimestampMs,
    },
    /// Perps position transitioned. Routed on `account.perp_positions`.
    PerpPositionUpdated {
        position_id: String,
        market_id: String,
        side: String,
        size_1e8: String,
        entry_price_1e8: String,
        margin_1e8: String,
        realized_pnl_1e8: String,
        status: String,
        updated_at_ms: TimestampMs,
    },
    /// Perps submit attempt was classified as trader-facing rejected.
    /// V1 fires only from internal execution paths (unit tests exercise
    /// this); public routes stay fail-closed. Routed on
    /// `account.perp_orders`. Never carries auth material.
    PerpOrderRejected {
        market_id: Option<String>,
        side: Option<String>,
        price_1e8: Option<String>,
        size_1e8: Option<String>,
        time_in_force: Option<String>,
        post_only: Option<bool>,
        reduce_only: Option<bool>,
        client_order_id: Option<String>,
        reason_code: String,
        reason_message: Option<String>,
        reason_source: String,
        created_at_ms: TimestampMs,
    },
    /// PERPS-LIQUIDATION-AND-RISK-V1 — a Perps position was liquidated
    /// (or a candidate was skipped because the mark was unavailable).
    /// Routed on `account.perp_positions` so consumers already tracking
    /// position deltas receive both `PerpPositionUpdated` (status flip
    /// to 'liquidated') and this event on one subscription. Never
    /// carries signatures, nonces, envelopes, headers, or bearer
    /// tokens.
    PerpPositionLiquidated {
        liquidation_id: String,
        market_id: String,
        position_id: String,
        side: String,
        size_1e8: String,
        mark_price_1e8: String,
        realized_pnl_1e8: String,
        bad_debt_1e8: String,
        liquidation_fee_1e8: String,
        reason_code: String,
        created_at_ms: TimestampMs,
    },
    /// PERPS-FUNDING-V1 — a funding settlement recorded a payment
    /// against one Perps position. Emitted AFTER the durable state
    /// mutation (in-memory or PG commit). Positive `payment_1e8`
    /// means the trader OWED funding (margin reduced); negative means
    /// the trader RECEIVED funding (margin increased). Routed on
    /// `account.perp_funding`. Consumers already subscribed to
    /// `account.perp_positions` will additionally see a
    /// `PerpPositionUpdated` frame reflecting the new margin.
    ///
    /// Never carries signatures, nonces, envelopes, headers, or
    /// bearer tokens.
    PerpFundingPaymentCreated {
        funding_event_id: String,
        market_id: String,
        position_id: String,
        side: String,
        position_size_1e8: String,
        funding_index_before_1e18: String,
        funding_index_after_1e18: String,
        funding_delta_1e18: String,
        payment_1e8: String,
        margin_before_1e8: String,
        margin_after_1e8: String,
        bad_debt_1e8: String,
        reason_code: String,
        created_at_ms: TimestampMs,
    },
}

/// Lightweight wrapper around `tokio::sync::broadcast::Sender` that
/// emitters use. The wrapper exists so emission can be a no-op when
/// no sender is wired (tests that don't care about WS) and so all
/// emit-site logging is centralized.
#[derive(Clone)]
pub struct LifecycleEventSender {
    inner: tokio::sync::broadcast::Sender<LifecycleEvent>,
}

impl LifecycleEventSender {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx_drop) = tokio::sync::broadcast::channel(capacity);
        Self { inner: tx }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LifecycleEvent> {
        self.inner.subscribe()
    }

    /// Emit one event. Logs and swallows the `SendError` produced when
    /// no receiver is currently subscribed — that is the normal case
    /// when no client is watching, and we MUST NOT fail the parent
    /// mutation because of an observability-layer concern.
    pub fn emit(&self, event: LifecycleEvent) {
        if let Err(err) = self.inner.send(event) {
            tracing::trace!(
                target: "deopt.lifecycle",
                reason = %err,
                "lifecycle event dropped — no active receiver"
            );
        }
    }
}

impl Default for LifecycleEventSender {
    fn default() -> Self {
        // Default capacity sized for ~256 in-flight events per receiver
        // before the oldest are dropped. Receivers that lag get an
        // explicit `RecvError::Lagged` and resync via REST snapshot.
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::now_ms;

    #[test]
    fn channel_string_round_trip() {
        for c in [
            LifecycleChannel::AccountOrders,
            LifecycleChannel::AccountFills,
            LifecycleChannel::AccountConditionalOrders,
        ] {
            let s = c.ws_channel_str();
            assert!(matches!(
                s,
                "account.orders" | "account.fills" | "account.conditional_orders"
            ));
        }
    }

    #[tokio::test]
    async fn emit_with_receiver_delivers_event() {
        let sender = LifecycleEventSender::new(16);
        let mut rx = sender.subscribe();
        sender.emit(LifecycleEvent {
            account: AccountId::new("0xabc"),
            channel: LifecycleChannel::AccountOrders,
            payload: LifecyclePayload::OrderUpdated {
                order_id: "o1".into(),
                option_series_id: "s1".into(),
                status: "open".into(),
                remaining_size_1e8: "100".into(),
                size_1e8: "100".into(),
            },
            emitted_at_ms: now_ms(),
        });
        let received = rx.recv().await.expect("recv");
        assert_eq!(received.account.0, "0xabc");
        assert_eq!(received.channel.ws_channel_str(), "account.orders");
    }

    #[tokio::test]
    async fn emit_with_no_receiver_is_silently_dropped() {
        // The point of this test is to assert that the parent mutation
        // is not poisoned when no WS client is listening.
        let sender = LifecycleEventSender::new(16);
        sender.emit(LifecycleEvent {
            account: AccountId::new("0xnobody"),
            channel: LifecycleChannel::AccountFills,
            payload: LifecyclePayload::FillCreated {
                fill_id: "f".into(),
                option_series_id: "s".into(),
                order_id: "o".into(),
                side: "buy".into(),
                price_1e8: "1".into(),
                size_1e8: "1".into(),
                created_at_ms: now_ms(),
            },
            emitted_at_ms: now_ms(),
        });
        // No panic, no error propagated. Reaching this line is the assertion.
    }
}
