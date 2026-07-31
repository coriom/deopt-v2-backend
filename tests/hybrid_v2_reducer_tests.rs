//! Reducer coverage: every canonical projection mutation, monotonicity
//! invariants, and reorg/rebuild replay determinism.

use deopt_v2_backend::hybrid_v2::events::{EventKind, HybridV2Event};
use deopt_v2_backend::hybrid_v2::reducer::{
    apply, ApplyContext, ProjectionState, RecoveryStateProjection, ReducerError,
};
use serde_json::json;

fn ctx(block: u64, tx: &str, log_index: u32) -> ApplyContext {
    ApplyContext {
        block_number: block,
        tx_hash: tx.to_string(),
        log_index,
        block_timestamp: block * 12,
    }
}

fn ev(
    kind: EventKind,
    subkey: Option<&str>,
    owner: Option<&str>,
    subaccount_id: Option<u32>,
    token: Option<&str>,
    engine: Option<&str>,
    payload: serde_json::Value,
) -> HybridV2Event {
    HybridV2Event {
        kind,
        event_version: 1,
        subkey: subkey.map(String::from),
        owner: owner.map(String::from),
        subaccount_id,
        token: token.map(String::from),
        engine: engine.map(String::from),
        execution_id: None,
        order_hash: None,
        series_id: None,
        payload,
    }
}

#[test]
fn subaccount_identity_materialised_once() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::SubaccountCreated,
            Some("0xaa"),
            Some("0xo1"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    assert_eq!(
        state.subaccounts.get(&("0xo1".into(), 1)),
        Some(&"0xaa".to_string())
    );
    assert!(
        state
            .subaccount_meta
            .get("0xaa")
            .unwrap()
            .materialised_via_created
    );
    // Idempotent re-apply is safe.
    apply(
        &mut state,
        &ev(
            EventKind::SubaccountCreated,
            Some("0xaa"),
            Some("0xo1"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap();
    assert_eq!(state.subaccounts.len(), 1);
}

#[test]
fn deposit_withdraw_underflow_rejected() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::Deposit,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            Some("0xt"),
            None,
            json!({ "amount": "100" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    assert_eq!(
        state.balances.get(&("0xaa".into(), "0xt".into())),
        Some(&"100".to_string())
    );
    let err = apply(
        &mut state,
        &ev(
            EventKind::Withdraw,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            Some("0xt"),
            None,
            json!({ "amount": "200" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap_err();
    assert!(matches!(err, ReducerError::Underflow { .. }));
}

#[test]
fn internal_transfer_moves_balance() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::Deposit,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            Some("0xt"),
            None,
            json!({ "amount": "100" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::InternalTransfer,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            None,
            json!({ "from_subkey": "0xaa", "to_subkey": "0xbb", "amount": "40" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap();
    assert_eq!(
        state.balances.get(&("0xaa".into(), "0xt".into())),
        Some(&"60".to_string())
    );
    assert_eq!(
        state.balances.get(&("0xbb".into(), "0xt".into())),
        Some(&"40".to_string())
    );
}

#[test]
fn reservation_lock_unlock_underflow_rejected() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::CollateralLocked,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            Some("0xe1"),
            json!({ "amount": "50" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    assert_eq!(
        state
            .reservations
            .get(&("0xaa".into(), "0xt".into(), "0xe1".into())),
        Some(&"50".to_string())
    );
    let err = apply(
        &mut state,
        &ev(
            EventKind::CollateralUnlocked,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            Some("0xe1"),
            json!({ "amount": "100" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap_err();
    assert!(matches!(err, ReducerError::Underflow { .. }));
}

#[test]
fn collateral_universe_capacity_enforced() {
    let mut state = ProjectionState::default();
    for i in 0..8 {
        apply(
            &mut state,
            &ev(
                EventKind::CollateralTokenEnteredUniverse,
                None,
                None,
                None,
                Some(&format!("0xt{}", i)),
                None,
                json!({ "universe_index": format!("{}", i) }),
            ),
            &ctx(1 + i as u64, "tx", 0),
        )
        .unwrap();
    }
    let err = apply(
        &mut state,
        &ev(
            EventKind::CollateralTokenEnteredUniverse,
            None,
            None,
            None,
            Some("0xt9"),
            None,
            json!({ "universe_index": "9" }),
        ),
        &ctx(20, "tx", 0),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ReducerError::CollateralUniverseCapacity { .. }
    ));
}

#[test]
fn recovery_transitions_and_finalized_credit_rejected() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::RecoveryRequested,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    assert_eq!(
        state.recovery_state.get("0xaa").copied(),
        Some(RecoveryStateProjection::RecoveryPending)
    );
    apply(
        &mut state,
        &ev(
            EventKind::RecoveryActivated,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::RecoveryFinalized,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(3, "tx3", 0),
    )
    .unwrap();
    // Balances/reservations zeroed on finalize.
    apply(
        &mut state,
        &ev(
            EventKind::Deposit,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            Some("0xt"),
            None,
            json!({ "amount": "100" }),
        ),
        &ctx(4, "tx4", 0),
    )
    .expect_err("credit to finalized should be rejected");
    // Re-finalize is illegal.
    let err = apply(
        &mut state,
        &ev(
            EventKind::RecoveryFinalized,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(5, "tx5", 0),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ReducerError::IllegalRecoveryTransition { .. }
    ));
}

#[test]
fn order_filled_monotonic_and_terminal() {
    let mut state = ProjectionState::default();
    let mut e = ev(
        EventKind::OptionOrderFilled,
        Some("0xaa"),
        Some("0xo"),
        Some(1),
        None,
        None,
        json!({
            "filled_delta_1e8": "100",
            "total_qty_1e8": "500",
            "terminal": false,
            "time_in_force": 0,
            "side": 0,
        }),
    );
    e.order_hash = Some("0xoh1".into());
    apply(&mut state, &e, &ctx(1, "tx1", 0)).unwrap();
    let row = state.order_lifecycle.get("0xoh1").unwrap();
    assert_eq!(row.filled_qty_1e8, "100");

    let mut e2 = ev(
        EventKind::OptionOrderFilled,
        Some("0xaa"),
        Some("0xo"),
        Some(1),
        None,
        None,
        json!({
            "filled_delta_1e8": "400",
            "total_qty_1e8": "500",
            "terminal": true,
            "time_in_force": 0,
            "side": 0,
        }),
    );
    e2.order_hash = Some("0xoh1".into());
    apply(&mut state, &e2, &ctx(2, "tx2", 0)).unwrap();
    let row = state.order_lifecycle.get("0xoh1").unwrap();
    assert_eq!(row.filled_qty_1e8, "500");
    assert!(row.terminal);
}

#[test]
fn min_valid_nonce_monotonic() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::OptionSubaccountMinValidOrderNonceAdvanced,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({ "new_min_valid_nonce": "5" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    let err = apply(
        &mut state,
        &ev(
            EventKind::OptionSubaccountMinValidOrderNonceAdvanced,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({ "new_min_valid_nonce": "3" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap_err();
    assert!(matches!(err, ReducerError::MinNonceDecrease { .. }));
}

#[test]
fn owner_nonce_cancelled_monotonic() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::NonceCancelled,
            None,
            Some("0xo"),
            None,
            None,
            None,
            json!({ "new_min_valid_nonce": "10" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    let err = apply(
        &mut state,
        &ev(
            EventKind::NonceCancelled,
            None,
            Some("0xo"),
            None,
            None,
            None,
            json!({ "new_min_valid_nonce": "3" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap_err();
    assert!(matches!(err, ReducerError::MinNonceDecrease { .. }));
}

#[test]
fn owner_recovery_epoch_advances() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::OwnerRecoveryEpochAdvanced,
            None,
            Some("0xo"),
            None,
            None,
            None,
            json!({}),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::OwnerRecoveryEpochAdvanced,
            None,
            Some("0xo"),
            None,
            None,
            None,
            json!({}),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap();
    assert_eq!(
        state.owner_recovery_epochs.get("0xo").unwrap().epoch_count,
        2
    );
}

#[test]
fn positions_open_modify_liquidate() {
    let mut state = ProjectionState::default();
    let mut e = ev(
        EventKind::OptionPositionOpened,
        Some("0xaa"),
        None,
        None,
        None,
        None,
        json!({ "long_delta_1e8": "1000", "short_delta_1e8": "0" }),
    );
    e.series_id = Some("0xser1".into());
    apply(&mut state, &e, &ctx(1, "tx1", 0)).unwrap();
    let row = state
        .positions
        .get(&("0xaa".into(), "0xser1".into()))
        .unwrap();
    assert_eq!(row.long_qty_1e8, "1000");
    assert!(state.active_series.get("0xaa").unwrap().contains("0xser1"));

    let mut e2 = ev(
        EventKind::OptionPositionModified,
        Some("0xaa"),
        None,
        None,
        None,
        None,
        json!({ "long_delta_1e8_signed": "-400", "short_delta_1e8_signed": 0 }),
    );
    e2.series_id = Some("0xser1".into());
    apply(&mut state, &e2, &ctx(2, "tx2", 0)).unwrap();
    let row = state
        .positions
        .get(&("0xaa".into(), "0xser1".into()))
        .unwrap();
    assert_eq!(row.long_qty_1e8, "600");

    let mut e3 = ev(
        EventKind::OptionPositionLiquidated,
        Some("0xaa"),
        None,
        None,
        None,
        None,
        json!({}),
    );
    e3.series_id = Some("0xser1".into());
    apply(&mut state, &e3, &ctx(3, "tx3", 0)).unwrap();
    let row = state
        .positions
        .get(&("0xaa".into(), "0xser1".into()))
        .unwrap();
    assert_eq!(row.long_qty_1e8, "0");
}

#[test]
fn active_series_capacity_enforced() {
    let mut state = ProjectionState::default();
    for i in 0..33 {
        let mut e = ev(
            EventKind::OptionPositionOpened,
            Some("0xaa"),
            None,
            None,
            None,
            None,
            json!({ "long_delta_1e8": "1", "short_delta_1e8": "0" }),
        );
        e.series_id = Some(format!("0xser{}", i));
        let res = apply(&mut state, &e, &ctx(1 + i as u64, "tx", 0));
        if i < 32 {
            res.unwrap();
        } else {
            assert!(matches!(
                res.unwrap_err(),
                ReducerError::ActiveSeriesCapacity { .. }
            ));
        }
    }
}

#[test]
fn recovery_pause_recorded() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::RecoveryPauseSet,
            None,
            None,
            None,
            None,
            None,
            json!({ "paused": true, "until_ts": "1000" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    assert!(state.recovery_pause.as_ref().unwrap().paused);
}

#[test]
fn fee_events_journaled() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::OptionPremiumTransferred,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            None,
            json!({ "amount": "1000", "from_subkey": "0xaa", "to_subkey": "0xbb" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::OptionFeeCharged,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            None,
            json!({ "amount": "10", "fee_subkey": "0xffe" }),
        ),
        &ctx(1, "tx1", 1),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::OptionRebatePaid,
            Some("0xbb"),
            None,
            None,
            Some("0xt"),
            None,
            json!({ "amount": "5", "rebate_subkey": "0xreb" }),
        ),
        &ctx(1, "tx1", 2),
    )
    .unwrap();
    assert_eq!(state.fee_events.len(), 3);
}

#[test]
fn subaccount_finalization_zeroes_state() {
    let mut state = ProjectionState::default();
    apply(
        &mut state,
        &ev(
            EventKind::Deposit,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            Some("0xt"),
            None,
            json!({ "amount": "100" }),
        ),
        &ctx(1, "tx1", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::CollateralLocked,
            Some("0xaa"),
            None,
            None,
            Some("0xt"),
            Some("0xe1"),
            json!({ "amount": "50" }),
        ),
        &ctx(2, "tx2", 0),
    )
    .unwrap();
    apply(
        &mut state,
        &ev(
            EventKind::RecoveryFinalized,
            Some("0xaa"),
            Some("0xo"),
            Some(1),
            None,
            None,
            json!({}),
        ),
        &ctx(3, "tx3", 0),
    )
    .unwrap();
    assert!(state.balances.get(&("0xaa".into(), "0xt".into())).is_none());
    assert!(state
        .reservations
        .get(&("0xaa".into(), "0xt".into(), "0xe1".into()))
        .is_none());
}
