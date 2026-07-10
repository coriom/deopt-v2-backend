//! RFQ-MULTI-LEG-CANCEL-V1 — service-level assertions for the taker
//! cancel path.
//!
//! HTTP-level auth (v2 canonical byte-freeze, resolve_options_v2_subaccount,
//! require_write_auth_v2_aware) is exercised by the shared auth
//! machinery + the byte-freeze test in `rfq_multi_leg_canonical_bytes_v1_tests.rs`.
//! These tests focus on service-layer behaviour: taker guard, flag
//! gate, atomicity, quote status flip, cross-subaccount refusal,
//! lifecycle, and post-cancel guards.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::auth::write_authorization::WriteAuthAction;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::error::BackendError;
use deopt_v2_backend::options::multi_leg_service::{
    accept_option_multi_leg_rfq_quote, cancel_option_multi_leg_rfq, create_option_multi_leg_rfq,
    ensure_option_multi_leg_rfq_enabled, get_option_multi_leg_rfq,
    list_option_multi_leg_rfq_quotes, submit_option_multi_leg_rfq_quote,
    AcceptOptionMultiLegRfqQuoteInput, CancelOptionMultiLegRfqInput, CreateOptionMultiLegRfqInput,
    LegInput, QuoteLegInput, SubmitOptionMultiLegRfqQuoteInput,
};
use deopt_v2_backend::options::service::{create_option_series, CreateOptionSeriesInput};
use deopt_v2_backend::options::{
    OptionMultiLegRfqQuoteStatus, OptionMultiLegRfqStatus, OptionsConfig,
};
use deopt_v2_backend::types::{now_ms, AccountId, Side};
use uuid::Uuid;

const TAKER_HEX: &str = "0x1111111111111111111111111111111111111111";
const MM_HEX: &str = "0x2222222222222222222222222222222222222222";
const OTHER_HEX: &str = "0x3333333333333333333333333333333333333333";

fn taker() -> AccountId {
    AccountId::new(TAKER_HEX)
}

fn mm() -> AccountId {
    AccountId::new(MM_HEX)
}

fn state_with_flag(flag: bool) -> AppState {
    let mut cfg = OptionsConfig::enabled_in_memory_for_tests();
    cfg.rfq_enabled = true;
    cfg.rfq_min_quote_ttl_ms = 1;
    cfg.rfq_max_quote_ttl_ms = 500;
    cfg.rfq_multi_leg_enabled = flag;
    AppState::with_options_config(EngineState::with_default_markets(), cfg)
}

async fn seed_series(state: &AppState, salt: u128) -> String {
    let expiry = u64::try_from(now_ms() / 1000).unwrap() + 86_400 + salt as u64;
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying: "ETH".to_string(),
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: "USDC".to_string(),
            expiry,
            strike_1e8: 300_000_000_000 + salt,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: None,
        },
    )
    .await
    .unwrap()
    .option_series_id
}

fn leg(index: u32, series: &str, side: Side) -> LegInput {
    LegInput {
        leg_index: index,
        option_series_id: series.to_string(),
        side,
        size_1e8: 100_000_000,
        ratio_num: 1,
        ratio_den: 1,
    }
}

fn quote_leg(index: u32, price: u128) -> QuoteLegInput {
    QuoteLegInput {
        leg_index: index,
        price_1e8: price,
    }
}

async fn seed_open_rfq(state: &AppState, taker_subaccount: u32) -> Uuid {
    let series = seed_series(state, taker_subaccount as u128).await;
    let (rfq, _) = create_option_multi_leg_rfq(
        state,
        CreateOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: taker_subaccount,
            legs: vec![leg(0, &series, Side::Buy), leg(1, &series, Side::Sell)],
            ttl_ms: Some(30_000),
        },
    )
    .await
    .unwrap();
    rfq.option_rfq_id
}

async fn seed_open_rfq_with_quote(state: &AppState) -> (Uuid, Uuid) {
    let rfq_id = seed_open_rfq(state, 1).await;
    let (quote, _) = submit_option_multi_leg_rfq_quote(
        state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "50000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: Some(1),
            quote_ttl_ms: Some(200),
            signature: None,
        },
    )
    .await
    .unwrap();
    (rfq_id, quote.quote_id)
}

// ---------------------------------------------------------------------
// Flag gate.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part1_flag_off_blocks_cancel() {
    let state = state_with_flag(false);
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::OptionMultiLegRfqNotLive));
    assert!(ensure_option_multi_leg_rfq_enabled(&state).is_err());
}

// ---------------------------------------------------------------------
// Integrity.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part2_valid_cancel_flips_status_and_updates_quote_count() {
    let state = state_with_flag(true);
    let (rfq_id, _quote_id) = seed_open_rfq_with_quote(&state).await;
    let outcome = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.rfq.status, OptionMultiLegRfqStatus::Cancelled);
    assert_eq!(outcome.cancelled_quotes, 1);
    // Reload confirms the flip is persisted.
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert_eq!(rfq.status, OptionMultiLegRfqStatus::Cancelled);
    assert!(rfq.accepted_quote_id.is_none());
    assert!(rfq.accepted_fill_id.is_none());
}

#[tokio::test]
async fn part2_valid_cancel_flips_every_open_quote_to_cancelled() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id) = seed_open_rfq_with_quote(&state).await;
    let _ = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();
    let list = list_option_multi_leg_rfq_quotes(&state, rfq_id)
        .await
        .unwrap();
    let (quote, _) = list
        .iter()
        .find(|(q, _)| q.quote_id == quote_id)
        .expect("quote present");
    assert_eq!(quote.status, OptionMultiLegRfqQuoteStatus::Cancelled);
}

#[tokio::test]
async fn part3_cancel_missing_rfq_returns_not_found() {
    let state = state_with_flag(true);
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqId));
}

#[tokio::test]
async fn part3_cancel_from_wrong_taker_rejects() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state, 1).await;
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: AccountId::new(OTHER_HEX),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_cancel_from_wrong_subaccount_rejects() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state, 1).await;
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 2,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
    // State untouched.
    let (rfq, _) = get_option_multi_leg_rfq(&state, rfq_id).await.unwrap();
    assert_eq!(rfq.status, OptionMultiLegRfqStatus::Open);
}

#[tokio::test]
async fn part3_zero_subaccount_rejects() {
    let state = state_with_flag(true);
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 0,
            option_rfq_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_cancel_already_accepted_rfq_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id) = seed_open_rfq_with_quote(&state).await;
    let (_quote, quote_legs) =
        deopt_v2_backend::options::multi_leg_service::get_option_multi_leg_rfq_quote(
            &state, quote_id,
        )
        .await
        .unwrap();
    // Accept first, then attempt cancel.
    let _ = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: "50000000".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap();
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part3_second_cancel_of_same_rfq_rejects() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state, 1).await;
    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();
    let err = cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

// ---------------------------------------------------------------------
// Post-cancel guards.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part4_accept_after_cancel_rejects() {
    let state = state_with_flag(true);
    let (rfq_id, quote_id) = seed_open_rfq_with_quote(&state).await;
    let (_quote, quote_legs) =
        deopt_v2_backend::options::multi_leg_service::get_option_multi_leg_rfq_quote(
            &state, quote_id,
        )
        .await
        .unwrap();
    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();
    let err = accept_option_multi_leg_rfq_quote(
        &state,
        AcceptOptionMultiLegRfqQuoteInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
            quote_id,
            expected_package_price_1e8: "50000000".to_string(),
            expected_legs_count: 2,
            expected_leg_prices_1e8: quote_legs.iter().map(|q| q.price_1e8).collect(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

#[tokio::test]
async fn part4_quote_submit_after_cancel_rejects() {
    let state = state_with_flag(true);
    let rfq_id = seed_open_rfq(&state, 1).await;
    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();
    let err = submit_option_multi_leg_rfq_quote(
        &state,
        rfq_id,
        SubmitOptionMultiLegRfqQuoteInput {
            mm_account: mm(),
            maker_subaccount_id: 1,
            session_id: None,
            client_quote_id: None,
            package_price_1e8: "50000000".to_string(),
            size_1e8: 100_000_000,
            legs: vec![quote_leg(0, 1_000_000), quote_leg(1, 2_000_000)],
            quote_nonce: None,
            quote_ttl_ms: Some(100),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BackendError::InvalidOptionRfqState(_)));
}

// ---------------------------------------------------------------------
// Subaccount isolation.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part5_account_1_and_account_2_cancellation_paths_are_isolated() {
    let state = state_with_flag(true);
    let rfq_1 = seed_open_rfq(&state, 1).await;
    let rfq_2 = seed_open_rfq(&state, 2).await;

    // Cancel from Account 1 must NOT affect the Account-2 RFQ, and
    // vice versa.
    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_1,
        },
    )
    .await
    .unwrap();
    let (rfq_2_loaded, _) = get_option_multi_leg_rfq(&state, rfq_2).await.unwrap();
    assert_eq!(rfq_2_loaded.status, OptionMultiLegRfqStatus::Open);

    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 2,
            option_rfq_id: rfq_2,
        },
    )
    .await
    .unwrap();
    let (rfq_2_after, _) = get_option_multi_leg_rfq(&state, rfq_2).await.unwrap();
    assert_eq!(rfq_2_after.status, OptionMultiLegRfqStatus::Cancelled);
}

// ---------------------------------------------------------------------
// Lifecycle payload — no secrets.
// ---------------------------------------------------------------------

#[tokio::test]
async fn part6_cancelled_lifecycle_payload_has_taker_subaccount_and_no_secrets() {
    let state = state_with_flag(true);
    let mut rx = state.lifecycle_events.subscribe();
    let (rfq_id, _quote_id) = seed_open_rfq_with_quote(&state).await;
    cancel_option_multi_leg_rfq(
        &state,
        CancelOptionMultiLegRfqInput {
            taker: taker(),
            taker_subaccount_id: 1,
            option_rfq_id: rfq_id,
        },
    )
    .await
    .unwrap();

    let mut saw_cancelled = false;
    for _ in 0..16 {
        match rx.try_recv() {
            Ok(event) => {
                let json = serde_json::to_string(&event.payload).unwrap();
                if json.contains("option_multi_leg_rfq_cancelled") {
                    saw_cancelled = true;
                    assert!(json.contains("\"taker_subaccount_id\":1"));
                    assert!(json.contains("\"cancelled_quotes\":1"));
                    assert!(!json.contains("signature"));
                    assert!(!json.contains("nonce"));
                    assert!(!json.contains("authorization"));
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_cancelled,
        "cancelled lifecycle payload must be broadcast"
    );
}

// ---------------------------------------------------------------------
// WriteAuthAction round-trip.
// ---------------------------------------------------------------------

#[test]
fn part7_cancel_action_str_and_parse_round_trip() {
    assert_eq!(
        WriteAuthAction::OptionMultiLegRfqCancel.as_str(),
        "OPTION_MULTI_LEG_RFQ_CANCEL"
    );
    assert_eq!(
        WriteAuthAction::parse("OPTION_MULTI_LEG_RFQ_CANCEL"),
        Some(WriteAuthAction::OptionMultiLegRfqCancel)
    );
    // Cross-action isolation from every sibling multi-leg action AND
    // the single-leg cancel.
    let siblings = [
        WriteAuthAction::OptionMultiLegRfqCreate.as_str(),
        WriteAuthAction::OptionMultiLegRfqQuoteSubmit.as_str(),
        WriteAuthAction::OptionMultiLegRfqAccept.as_str(),
        WriteAuthAction::OptionRfqCancel.as_str(),
    ];
    for sibling in siblings {
        assert_ne!(WriteAuthAction::OptionMultiLegRfqCancel.as_str(), sibling);
    }
}
