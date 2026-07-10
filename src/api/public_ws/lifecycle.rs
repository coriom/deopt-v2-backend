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
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — Options RFQ lifecycle. Emitted
    /// AFTER the corresponding Options RFQ service call has
    /// persisted its state change (create, quote-submit, accept,
    /// cancel) or, in the accept path, AFTER the fill row has been
    /// written. Consumers can drive `/rfq-strategy` refetches from
    /// this single channel — quote deltas, RFQ status deltas, and
    /// fill deltas all land here so the workspace only needs one
    /// subscription. Privacy: events are per-account (`taker` and,
    /// where relevant, `mm_account`); the WS session filter
    /// forwards only events matching the authenticated address.
    AccountRfqs,
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
            Self::AccountRfqs => "account.rfqs",
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
    ///
    /// SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1 — `subaccount_id` sourced
    /// from `option_orders.subaccount_id`. `#[serde(default)]`
    /// preserves wire-compat: consumers reading an older payload
    /// see `1` for pre-migration rows.
    OrderUpdated {
        order_id: String,
        option_series_id: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        subaccount_id: u32,
        status: String,
        remaining_size_1e8: String,
        size_1e8: String,
    },
    /// A new option fill landed for this account (as buyer or seller).
    ///
    /// SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1 — carries BOTH sides'
    /// subaccount ids so a wallet that owned both legs can filter
    /// side-aware. The recipient reads `buyer_subaccount_id` when
    /// they were the buyer and `seller_subaccount_id` when they
    /// were the seller. Emitter fires ONE frame per interested
    /// account (buyer + seller); routing is per session identity.
    FillCreated {
        fill_id: String,
        option_series_id: String,
        order_id: String,
        side: String,
        price_1e8: String,
        size_1e8: String,
        created_at_ms: TimestampMs,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        buyer_subaccount_id: u32,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        seller_subaccount_id: u32,
    },
    /// A conditional order (TP/SL) state changed.
    ///
    /// SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1 — `subaccount_id` sourced
    /// from `options_conditional_orders.subaccount_id`.
    ConditionalOrderUpdated {
        conditional_order_id: String,
        option_series_id: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        subaccount_id: u32,
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
    ///
    /// SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1 — `subaccount_id` is
    /// inherited from the parent Options order that owns the plan
    /// (attached TP/SL always shares the parent's subaccount).
    AttachmentPlanUpdated {
        plan_id: String,
        parent_order_id: String,
        option_series_id: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        subaccount_id: u32,
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
        /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — subaccount of the
        /// order. Wire-compat: legacy pre-milestone payloads omit
        /// the field and deserialise to `1`.
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        subaccount_id: u32,
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
        /// PERPS-SUBACCOUNTS-ENGINE-ROUTING-V1 — side-specific
        /// subaccount ids for the fill counterparties.
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        taker_subaccount_id: u32,
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        maker_subaccount_id: u32,
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
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        subaccount_id: u32,
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
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        subaccount_id: u32,
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
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        subaccount_id: u32,
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
        #[serde(default = "crate::perps::positions::one_subaccount_id")]
        subaccount_id: u32,
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
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — a taker created a new Options
    /// RFQ. Emitted AFTER the RFQ has been persisted. Routed on
    /// `account.rfqs` to the taker's authenticated session.
    ///
    /// Never carries signatures, write-auth nonces, envelopes,
    /// headers, or bearer tokens.
    OptionRfqCreated {
        option_rfq_id: String,
        option_series_id: String,
        taker: String,
        /// SUBACCOUNTS-RFQ-INTEGRATION-V1 — subaccount the taker
        /// signed for. `1` for pre-migration data.
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        side: String,
        size_1e8: String,
        limit_price_1e8: Option<String>,
        status: String,
        created_at_ms: TimestampMs,
        expires_at_ms: TimestampMs,
    },
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — a maker submitted a quote on
    /// an open RFQ. Emitted AFTER the quote has been persisted.
    /// The service emits ONE frame per interested account (taker
    /// AND mm_account); the per-session handler filter routes
    /// each frame to the correct authenticated session.
    OptionRfqQuoteSubmitted {
        option_rfq_id: String,
        quote_id: String,
        option_series_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        mm_account: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        maker_subaccount_id: u32,
        price_1e8: String,
        size_1e8: String,
        status: String,
        created_at_ms: TimestampMs,
        expires_at_ms: TimestampMs,
    },
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — a taker accepted a maker
    /// quote. Emitted AFTER the RFQ + quote status transitions
    /// and the fill row have been persisted. One frame per
    /// interested account (buyer + seller).
    OptionRfqAccepted {
        option_rfq_id: String,
        quote_id: String,
        option_series_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        mm_account: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        maker_subaccount_id: u32,
        rfq_status: String,
        quote_status: String,
        option_fill_id: String,
        accepted_at_ms: TimestampMs,
    },
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — the fill created by an
    /// accepted quote landed in `option_rfq_fills`. Emitted AFTER
    /// persistence, ONE frame per interested account
    /// (buyer + seller). Consumers already subscribed to
    /// `account.rfqs` see this on the same subscription as the
    /// preceding `OptionRfqAccepted`.
    OptionRfqFillCreated {
        option_rfq_id: String,
        quote_id: String,
        fill_id: String,
        option_series_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        mm_account: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        maker_subaccount_id: u32,
        taker_side: String,
        price_1e8: String,
        size_1e8: String,
        created_at_ms: TimestampMs,
    },
    /// OPTIONS-RFQ-LIFECYCLE-WS-V1 — a taker cancelled their open
    /// RFQ. Emitted AFTER persistence, routed on `account.rfqs`
    /// to the taker's authenticated session. Makers that had
    /// quoted this RFQ receive a subsequent quote-status
    /// transition through their next quote refresh; V1 does not
    /// fan out per-maker cancel events.
    OptionRfqCancelled {
        option_rfq_id: String,
        option_series_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        status: String,
        cancelled_at_ms: TimestampMs,
    },
    /// RFQ-MULTI-LEG-CREATE-QUOTE-V1 — a taker created a new
    /// multi-leg atomic Options RFQ. Emitted AFTER the parent RFQ
    /// row + N leg rows are committed. Carries `legs_count` and
    /// the taker's subaccount but does NOT enumerate per-leg
    /// series ids on the wire; consumers fetch the full leg list
    /// via `GET /options/multi-leg-rfqs/:id`. This keeps the WS
    /// frame small and avoids leaking every leg's series across
    /// the pub-sub bus.
    ///
    /// Privacy: no signatures, no write-auth nonce, no envelope.
    OptionMultiLegRfqCreated {
        option_rfq_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        legs_count: u32,
        status: String,
        created_at_ms: TimestampMs,
        expires_at_ms: TimestampMs,
    },
    /// RFQ-MULTI-LEG-CREATE-QUOTE-V1 — a maker submitted a package
    /// quote on an open multi-leg RFQ. Emitted AFTER the quote
    /// row + N quote-leg rows are committed. Fan-out: one frame
    /// per interested account (taker + maker). Consumers refetch
    /// the full quote (with per-leg prices) via
    /// `GET /options/multi-leg-rfqs/:id/quotes/:quote_id`.
    OptionMultiLegRfqQuoteSubmitted {
        option_rfq_id: String,
        quote_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        mm_account: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        maker_subaccount_id: u32,
        legs_count: u32,
        package_price_1e8: String,
        size_1e8: String,
        status: String,
        created_at_ms: TimestampMs,
        expires_at_ms: TimestampMs,
    },
    /// RFQ-MULTI-LEG-ATOMIC-ACCEPT-V1 — taker accepted a maker
    /// package quote. Emitted AFTER the atomic transaction
    /// commits (RFQ + winning quote flipped to Accepted, losing
    /// quotes flipped to Rejected, parent fill + N fill-leg rows
    /// inserted). Fan-out: one frame per interested account
    /// (taker + maker). Consumers refetch the full fill (with
    /// per-leg prices) via `GET /options/multi-leg-rfqs/:id`
    /// once the fill-read route lands.
    ///
    /// Privacy: no signatures, no write-auth nonce, no envelope.
    OptionMultiLegRfqAccepted {
        option_rfq_id: String,
        quote_id: String,
        fill_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        mm_account: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        maker_subaccount_id: u32,
        legs_count: u32,
        package_price_1e8: String,
        size_1e8: String,
        rfq_status: String,
        quote_status: String,
        accepted_at_ms: TimestampMs,
    },
    /// RFQ-MULTI-LEG-CANCEL-V1 — taker cancelled their open
    /// multi-leg RFQ. Emitted AFTER the atomic transaction commits
    /// (RFQ status flipped to Cancelled, every open quote flipped
    /// to Cancelled). Fan-out: taker only in V1. Makers with open
    /// quotes see their quotes flip status on the next quote
    /// refresh via `GET /options/multi-leg-rfqs/:id/quotes`; a
    /// per-maker fan-out variant is deferred to
    /// `RFQ-MULTI-LEG-MM-GATEWAY-V1` when the maker-side channel
    /// gains a routing hook.
    ///
    /// Privacy: no signatures, no write-auth nonce, no envelope.
    OptionMultiLegRfqCancelled {
        option_rfq_id: String,
        taker: String,
        #[serde(default = "crate::options::types::one_subaccount_id")]
        taker_subaccount_id: u32,
        status: String,
        cancelled_quotes: u32,
        cancelled_at_ms: TimestampMs,
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
                subaccount_id: 1,
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
                buyer_subaccount_id: 1,
                seller_subaccount_id: 1,
            },
            emitted_at_ms: now_ms(),
        });
        // No panic, no error propagated. Reaching this line is the assertion.
    }
}
