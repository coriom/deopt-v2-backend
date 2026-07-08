//! SUBACCOUNTS-OPTIONS-WS-PAYLOAD-V1
//!
//! Pins the private-WS payload contract shared with the frontend:
//!
//! * **`OrderUpdated.subaccount_id`** — sourced from
//!   `option_orders.subaccount_id`. Serialised as `subaccount_id` on
//!   the wire with `#[serde(default = "one_subaccount_id")]` so an
//!   older subscriber reading a pre-migration payload defaults to
//!   `1`.
//! * **`FillCreated.{buyer,seller}_subaccount_id`** — both sides
//!   captured so a wallet that owned both legs (rare but possible)
//!   can filter side-aware. Same default semantics.
//! * **`ConditionalOrderUpdated.subaccount_id`** — sourced from
//!   `options_conditional_orders.subaccount_id`. Attached TP/SL
//!   rows inherit the parent's value at materialisation time.
//! * **`AttachmentPlanUpdated.subaccount_id`** — inherited from the
//!   plan's parent order at emit time.
//!
//! Subscription posture: server-side subscription remains wallet-
//! level (per-session address gate in the WS handler). Subaccount
//! is a payload field, not a subscription parameter — the frontend
//! filters or refetches based on it. This posture matches how RFQ
//! WS payloads shipped in `SUBACCOUNTS-RFQ-INTEGRATION-V1`; adding
//! a subscription parameter would require a deeper handler rewrite
//! and is deferred honestly.

use deopt_v2_backend::api::public_ws::{LifecycleChannel, LifecycleEvent, LifecyclePayload};
use deopt_v2_backend::types::{AccountId, TimestampMs};

fn ts() -> TimestampMs {
    1_782_000_000_000
}

fn account() -> AccountId {
    AccountId::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
}

// ===========================================================================
// Wire shape freeze — the payload struct must expose `subaccount_id` (or the
// per-side pair for fills). Building the enum variant is enough to prove the
// field exists; serialising to JSON then asserting the key is present pins
// the wire contract as the frontend will consume it.
// ===========================================================================

#[test]
fn order_updated_carries_subaccount_id_on_the_wire() {
    let event = LifecycleEvent {
        account: account(),
        channel: LifecycleChannel::AccountOrders,
        payload: LifecyclePayload::OrderUpdated {
            order_id: "o1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 2,
            status: "open".into(),
            remaining_size_1e8: "100".into(),
            size_1e8: "100".into(),
        },
        emitted_at_ms: ts(),
    };
    let json = serde_json::to_value(&event.payload).expect("serialize");
    assert_eq!(json["subaccount_id"], 2);
}

#[test]
fn fill_created_carries_both_side_subaccount_ids_on_the_wire() {
    let event = LifecycleEvent {
        account: account(),
        channel: LifecycleChannel::AccountFills,
        payload: LifecyclePayload::FillCreated {
            fill_id: "f1".into(),
            option_series_id: "SERIES-A".into(),
            order_id: "o1".into(),
            side: "buy".into(),
            price_1e8: "1000000000".into(),
            size_1e8: "100000000".into(),
            created_at_ms: ts(),
            buyer_subaccount_id: 2,
            seller_subaccount_id: 3,
        },
        emitted_at_ms: ts(),
    };
    let json = serde_json::to_value(&event.payload).expect("serialize");
    assert_eq!(json["buyer_subaccount_id"], 2);
    assert_eq!(json["seller_subaccount_id"], 3);
}

#[test]
fn conditional_order_updated_carries_subaccount_id_on_the_wire() {
    let event = LifecycleEvent {
        account: account(),
        channel: LifecycleChannel::AccountConditionalOrders,
        payload: LifecyclePayload::ConditionalOrderUpdated {
            conditional_order_id: "c1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 2,
            status: "armed".into(),
            child_order_id: None,
            oco_group_id: None,
            failure_code: None,
        },
        emitted_at_ms: ts(),
    };
    let json = serde_json::to_value(&event.payload).expect("serialize");
    assert_eq!(json["subaccount_id"], 2);
}

#[test]
fn attachment_plan_updated_carries_subaccount_id_on_the_wire() {
    let event = LifecycleEvent {
        account: account(),
        channel: LifecycleChannel::AccountConditionalOrders,
        payload: LifecyclePayload::AttachmentPlanUpdated {
            plan_id: "p1".into(),
            parent_order_id: "o1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 2,
            status: "pending".into(),
            materialized_size_1e8: None,
            tp_conditional_order_id: None,
            sl_conditional_order_id: None,
            oco_group_id: None,
            failure_code: None,
            failure_message: None,
            updated_at_ms: ts(),
        },
        emitted_at_ms: ts(),
    };
    let json = serde_json::to_value(&event.payload).expect("serialize");
    assert_eq!(json["subaccount_id"], 2);
}

// ===========================================================================
// Wire compatibility — older subscribers sending a pre-migration payload
// (no subaccount fields) must still deserialise. The `#[serde(default)]`
// attribute is what makes this safe; the tests prove it.
// ===========================================================================

#[test]
fn order_updated_deserialises_without_subaccount_id_defaulting_to_one() {
    // `LifecyclePayload` is `#[serde(tag = "type", rename_all =
    // "snake_case")]` — the pre-migration wire looks like this,
    // with no `subaccount_id` field.
    let pre_migration_json = serde_json::json!({
        "type": "order_updated",
        "order_id": "o1",
        "option_series_id": "SERIES-A",
        "status": "open",
        "remaining_size_1e8": "100",
        "size_1e8": "100"
    });
    let payload: LifecyclePayload =
        serde_json::from_value(pre_migration_json).expect("deserialize v1");
    match payload {
        LifecyclePayload::OrderUpdated { subaccount_id, .. } => {
            assert_eq!(subaccount_id, 1, "missing subaccount_id must default to 1");
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn fill_created_deserialises_without_side_subaccount_ids_defaulting_to_one() {
    let pre_migration_json = serde_json::json!({
        "type": "fill_created",
        "fill_id": "f1",
        "option_series_id": "SERIES-A",
        "order_id": "o1",
        "side": "buy",
        "price_1e8": "1000000000",
        "size_1e8": "100000000",
        "created_at_ms": 1_782_000_000_000i64
    });
    let payload: LifecyclePayload =
        serde_json::from_value(pre_migration_json).expect("deserialize v1");
    match payload {
        LifecyclePayload::FillCreated {
            buyer_subaccount_id,
            seller_subaccount_id,
            ..
        } => {
            assert_eq!(buyer_subaccount_id, 1);
            assert_eq!(seller_subaccount_id, 1);
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn conditional_order_updated_deserialises_without_subaccount_id_defaulting_to_one() {
    let pre_migration_json = serde_json::json!({
        "type": "conditional_order_updated",
        "conditional_order_id": "c1",
        "option_series_id": "SERIES-A",
        "status": "armed",
        "child_order_id": null,
        "oco_group_id": null,
        "failure_code": null
    });
    let payload: LifecyclePayload =
        serde_json::from_value(pre_migration_json).expect("deserialize v1");
    match payload {
        LifecyclePayload::ConditionalOrderUpdated { subaccount_id, .. } => {
            assert_eq!(subaccount_id, 1);
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn attachment_plan_updated_deserialises_without_subaccount_id_defaulting_to_one() {
    let pre_migration_json = serde_json::json!({
        "type": "attachment_plan_updated",
        "plan_id": "p1",
        "parent_order_id": "o1",
        "option_series_id": "SERIES-A",
        "status": "pending",
        "materialized_size_1e8": null,
        "tp_conditional_order_id": null,
        "sl_conditional_order_id": null,
        "oco_group_id": null,
        "failure_code": null,
        "failure_message": null,
        "updated_at_ms": 1_782_000_000_000i64
    });
    let payload: LifecyclePayload =
        serde_json::from_value(pre_migration_json).expect("deserialize v1");
    match payload {
        LifecyclePayload::AttachmentPlanUpdated { subaccount_id, .. } => {
            assert_eq!(subaccount_id, 1);
        }
        _ => panic!("unexpected variant"),
    }
}

// ===========================================================================
// Privacy: even with subaccount fields added, payloads carry NO signatures,
// nonces, envelopes, headers, or bearer tokens. Only trader-meaningful
// scalars.
// ===========================================================================

#[test]
fn options_ws_payloads_carry_no_secret_material() {
    // Serialise every variant that grew a subaccount field and grep
    // the JSON for any forbidden substring.
    let variants = vec![
        serde_json::to_string(&LifecyclePayload::OrderUpdated {
            order_id: "o1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 1,
            status: "open".into(),
            remaining_size_1e8: "100".into(),
            size_1e8: "100".into(),
        })
        .unwrap(),
        serde_json::to_string(&LifecyclePayload::FillCreated {
            fill_id: "f1".into(),
            option_series_id: "SERIES-A".into(),
            order_id: "o1".into(),
            side: "buy".into(),
            price_1e8: "1000000000".into(),
            size_1e8: "100000000".into(),
            created_at_ms: ts(),
            buyer_subaccount_id: 1,
            seller_subaccount_id: 1,
        })
        .unwrap(),
        serde_json::to_string(&LifecyclePayload::ConditionalOrderUpdated {
            conditional_order_id: "c1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 1,
            status: "armed".into(),
            child_order_id: None,
            oco_group_id: None,
            failure_code: None,
        })
        .unwrap(),
        serde_json::to_string(&LifecyclePayload::AttachmentPlanUpdated {
            plan_id: "p1".into(),
            parent_order_id: "o1".into(),
            option_series_id: "SERIES-A".into(),
            subaccount_id: 1,
            status: "pending".into(),
            materialized_size_1e8: None,
            tp_conditional_order_id: None,
            sl_conditional_order_id: None,
            oco_group_id: None,
            failure_code: None,
            failure_message: None,
            updated_at_ms: ts(),
        })
        .unwrap(),
    ];
    let forbidden = [
        "signature",
        "authorization",
        "authorisation",
        "bearer",
        "nonce",
        "deadline",
        "envelope",
        "api_key",
        "private_key",
        "cookie",
    ];
    for json in &variants {
        let lower = json.to_lowercase();
        for term in &forbidden {
            assert!(
                !lower.contains(term),
                "payload leaked forbidden substring `{term}`: {json}"
            );
        }
    }
}
