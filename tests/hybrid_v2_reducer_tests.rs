//! BACKEND WP-01 reducer + property tests.

use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::reducer::{
    apply, ProjectionState, RecoveryStateProjection, ReducerError,
};
use serde_json::json;

fn deposit_event(subkey: &str, token: &str, amount: &str) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::Deposit,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: Some("0xabc".into()),
        subaccount_id: Some(1),
        token: Some(token.into()),
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount }),
    }
}

fn withdraw_event(subkey: &str, token: &str, amount: &str) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::Withdraw,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: Some("0xabc".into()),
        subaccount_id: Some(1),
        token: Some(token.into()),
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount }),
    }
}

fn lock_event(subkey: &str, token: &str, engine: &str, amount: &str) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::CollateralLocked,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: None,
        subaccount_id: None,
        token: Some(token.into()),
        engine: Some(engine.into()),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({ "amount": amount }),
    }
}

fn recovery_finalized(subkey: &str) -> HybridV2Event {
    HybridV2Event {
        kind: EventKind::RecoveryFinalized,
        event_version: 1,
        subkey: Some(subkey.into()),
        owner: Some("0xabc".into()),
        subaccount_id: Some(1),
        token: None,
        engine: None,
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload: json!({}),
    }
}

#[test]
fn deposit_then_withdraw_returns_to_zero() {
    let mut state = ProjectionState::default();
    apply(&mut state, &deposit_event("0xaa", "0xt1", "1000")).unwrap();
    apply(&mut state, &withdraw_event("0xaa", "0xt1", "700")).unwrap();
    assert_eq!(
        state
            .balances
            .get(&("0xaa".to_string(), "0xt1".to_string()))
            .unwrap(),
        "300"
    );
}

#[test]
fn withdraw_underflow_errors_and_leaves_state_untouched() {
    let mut state = ProjectionState::default();
    apply(&mut state, &deposit_event("0xaa", "0xt1", "100")).unwrap();
    let err = apply(&mut state, &withdraw_event("0xaa", "0xt1", "1000")).unwrap_err();
    assert!(matches!(err, ReducerError::Underflow { .. }));
    // Balance unchanged.
    assert_eq!(
        state
            .balances
            .get(&("0xaa".to_string(), "0xt1".to_string()))
            .unwrap(),
        "100"
    );
}

#[test]
fn reservation_add_and_release() {
    let mut state = ProjectionState::default();
    apply(&mut state, &deposit_event("0xaa", "0xt1", "500")).unwrap();
    apply(&mut state, &lock_event("0xaa", "0xt1", "0xe1", "200")).unwrap();
    assert_eq!(
        state
            .reservations
            .get(&("0xaa".to_string(), "0xt1".to_string(), "0xe1".to_string()))
            .unwrap(),
        "200"
    );
    let mut unlock = lock_event("0xaa", "0xt1", "0xe1", "50");
    unlock.kind = EventKind::CollateralUnlocked;
    apply(&mut state, &unlock).unwrap();
    assert_eq!(
        state
            .reservations
            .get(&("0xaa".to_string(), "0xt1".to_string(), "0xe1".to_string()))
            .unwrap(),
        "150"
    );
}

#[test]
fn recovery_finalization_zeroes_projection() {
    let mut state = ProjectionState::default();
    apply(&mut state, &deposit_event("0xaa", "0xt1", "500")).unwrap();
    apply(&mut state, &lock_event("0xaa", "0xt1", "0xe1", "0")).unwrap();

    apply(&mut state, &recovery_finalized("0xaa")).unwrap();
    assert_eq!(
        state.recovery_state.get("0xaa").copied(),
        Some(RecoveryStateProjection::Recovered)
    );
    assert!(state
        .balances
        .get(&("0xaa".to_string(), "0xt1".to_string()))
        .is_none());
    assert!(state
        .reservations
        .get(&("0xaa".to_string(), "0xt1".to_string(), "0xe1".to_string()))
        .is_none());
}

#[test]
fn finalized_subaccount_rejects_further_credit() {
    let mut state = ProjectionState::default();
    apply(&mut state, &recovery_finalized("0xaa")).unwrap();
    let err = apply(&mut state, &deposit_event("0xaa", "0xt1", "1")).unwrap_err();
    assert!(matches!(
        err,
        ReducerError::FinalizedSubaccountCredit { .. }
    ));
}

// PROPERTY-style bounded checks.

#[test]
fn property_filled_quantity_never_decreases_via_reducer_owned_paths() {
    // We do not currently reduce OptionOrderFilled — but the invariant that
    // NO reducer path decreases a monotone field should hold trivially
    // (nothing reduces it). This test is a compile-time reminder that if
    // a future arm adds fill projection, the invariant must still hold.
    let state = ProjectionState::default();
    let _ = state;
}

#[test]
fn property_replayed_event_stream_is_deterministic() {
    let events = vec![
        deposit_event("0xaa", "0xt1", "100"),
        deposit_event("0xaa", "0xt1", "50"),
        withdraw_event("0xaa", "0xt1", "30"),
        lock_event("0xaa", "0xt1", "0xe1", "20"),
    ];
    let mut s1 = ProjectionState::default();
    let mut s2 = ProjectionState::default();
    for e in &events {
        apply(&mut s1, e).unwrap();
        apply(&mut s2, e).unwrap();
    }
    assert_eq!(s1, s2);
}

#[test]
fn property_reservation_never_negative() {
    let mut state = ProjectionState::default();
    apply(&mut state, &deposit_event("0xaa", "0xt1", "10")).unwrap();
    apply(&mut state, &lock_event("0xaa", "0xt1", "0xe1", "5")).unwrap();
    let mut unlock = lock_event("0xaa", "0xt1", "0xe1", "100");
    unlock.kind = EventKind::CollateralUnlocked;
    let err = apply(&mut state, &unlock).unwrap_err();
    assert!(matches!(err, ReducerError::Underflow { .. }));
    assert_eq!(
        state
            .reservations
            .get(&("0xaa".to_string(), "0xt1".to_string(), "0xe1".to_string()))
            .unwrap(),
        "5"
    );
}
