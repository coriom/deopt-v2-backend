//! OPTIONS-HYBRID-V2-CORRELATION-OPERATIONAL-CORE-V1 Part E — in-
//! memory integration coverage for the execution-intent linkage
//! wiring. Every canonical orderbook execution intent created via
//! `create_option_orderbook_execution_intent` must carry its
//! originating fill's `canonical_execution_id`.
//!
//! In-memory `AppState` (no PostgreSQL required). The in-memory
//! matcher path produces fills with `canonical_execution_id` per
//! Package A wiring; this test proves the intent creation flow
//! threads the same identity into `option_execution_intents`.
//!
//! Zero broadcast, zero real chain.

use deopt_v2_backend::api::AppState;
use deopt_v2_backend::engine::EngineState;
use deopt_v2_backend::options::service::{
    create_option_orderbook_execution_intent, create_option_series, submit_option_order,
    CreateOptionSeriesInput, SubmitOptionOrderInput,
};
use deopt_v2_backend::options::{
    option_product_registry_option_id, OptionExecutionSourceType, OptionsConfig,
};
use deopt_v2_backend::signing::Eip712Domain;
use deopt_v2_backend::types::{now_ms, AccountId, Side, TimeInForce};

const VALID_SIGNATURE: &str = concat!(
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);

fn options_execution_state() -> AppState {
    let mut config = OptionsConfig::enabled_in_memory_for_tests();
    config.execution_enabled = true;
    config.execution_require_persistence = false;
    // Matches OPTIONS_CANONICAL_CHAIN_ID default; test uses in-memory
    // store so no persistence signer requirement applies.
    config.execution_eip712_domain = Eip712Domain {
        name: "DeOptV2-OptionMatchingEngine".to_string(),
        version: "1".to_string(),
        chain_id: 84532,
        verifying_contract: AccountId::new("0x00000000000000000000000000000000000000ee"),
    };
    config.matching_engine_address = AccountId::new("0x00000000000000000000000000000000000000ee");
    AppState::with_options_config(EngineState::with_default_markets(), config)
}

fn future_expiry_sec() -> u64 {
    ((now_ms() / 1_000) + 60 * 60 * 24 * 30) as u64
}

async fn active_series_id(state: &AppState) -> String {
    // Series with onchain identifiers so execution intent creation
    // (which requires `onchain_series_id | onchain_product_id`) can
    // proceed. Mirrors the existing `onchain_create_input()` fixture
    // pattern in `tests/options_tests.rs`.
    let expiry = future_expiry_sec();
    let underlying = "0x0000000000000000000000000000000000000010".to_string();
    let settlement = "0x0000000000000000000000000000000000000020".to_string();
    let onchain_option_id = option_product_registry_option_id(
        &AccountId::new(&underlying),
        &AccountId::new(&settlement),
        expiry,
        300_000_000_000,
        100_000_000,
        true,
        true,
    )
    .expect("onchain option id")
    .to_string();
    create_option_series(
        state,
        CreateOptionSeriesInput {
            underlying,
            base_asset: "ETH".to_string(),
            quote_asset: "USDC".to_string(),
            settlement_asset: settlement,
            expiry,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: Some(100_000_000),
            onchain_product_id: None,
            onchain_series_id: Some(onchain_option_id),
        },
    )
    .await
    .expect("series")
    .option_series_id
}

fn order_input(
    series: &str,
    account: AccountId,
    subaccount_id: u32,
    side: Side,
    price_1e8: u128,
    size_1e8: u128,
    client_order_id: &str,
    nonce: u64,
) -> SubmitOptionOrderInput {
    SubmitOptionOrderInput {
        option_series_id: series.to_string(),
        account,
        subaccount_id,
        side,
        price_1e8,
        size_1e8,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        client_order_id: Some(client_order_id.to_string()),
        nonce: Some(nonce),
        deadline_ms: Some(now_ms() + 60_000),
        signature: Some(VALID_SIGNATURE.to_string()),
        attached_tp_sl: None,
    }
}

/// Match two orders and prove the resulting fill carries a
/// `canonical_execution_id`. Then create the execution intent from
/// that fill and prove the intent carries the SAME identity.
#[tokio::test]
async fn intent_created_from_fill_carries_canonical_execution_id() {
    let state = options_execution_state();
    let series = active_series_id(&state).await;
    // Resting sell from wallet A.
    submit_option_order(
        &state,
        order_input(
            &series,
            AccountId::new("0x00000000000000000000000000000000000000a1"),
            1,
            Side::Sell,
            1_000_000_000,
            100_000_000,
            "linkage-maker",
            10,
        ),
    )
    .await
    .expect("resting sell");
    // Aggressive buy from wallet B produces a fill.
    let taker = submit_option_order(
        &state,
        order_input(
            &series,
            AccountId::new("0x00000000000000000000000000000000000000b2"),
            1,
            Side::Buy,
            1_000_000_000,
            100_000_000,
            "linkage-taker",
            11,
        ),
    )
    .await
    .expect("taker");
    assert_eq!(taker.fills.len(), 1, "one fill");
    let fill = &taker.fills[0];
    let fill_canonical = fill
        .canonical_execution_id
        .clone()
        .expect("fill must carry canonical_execution_id (Package A)");

    // Create the execution intent from the fill.
    let intent_opt = create_option_orderbook_execution_intent(&state, fill)
        .await
        .expect("intent creation");
    let intent = intent_opt.expect("intent must be created when execution_enabled");
    // Part E invariant: intent inherits the fill's canonical execution identity.
    assert_eq!(
        intent.canonical_execution_id.as_deref(),
        Some(fill_canonical.as_str()),
        "intent must carry the fill's canonical_execution_id"
    );
    // Sanity: source_type is orderbook and source_id is fill_id.
    assert_eq!(
        intent.source_type,
        OptionExecutionSourceType::OptionOrderbookFill
    );
    assert_eq!(intent.source_id, fill.fill_id.to_string());
    // Sanity: identities agree between fill and intent.
    assert_eq!(intent.buyer, fill.buyer);
    assert_eq!(intent.seller, fill.seller);
    assert_eq!(intent.source_size_1e8, fill.size_1e8);
}

/// User-initiated intent creation (no matcher fill) MUST NOT invent
/// a canonical_execution_id — the intent's canonical_execution_id
/// remains None, which the frontend flow signals as "quote-based
/// intent, no matcher origin".
#[tokio::test]
async fn user_initiated_intent_from_quote_has_no_canonical_execution_id() {
    use deopt_v2_backend::options::service::create_user_initiated_execution_intent_from_quote;
    let state = options_execution_state();
    let series = active_series_id(&state).await;
    let series_row = deopt_v2_backend::options::service::get_option_series(&state, &series)
        .await
        .expect("series");
    let intent = create_user_initiated_execution_intent_from_quote(
        &state,
        &series_row,
        AccountId::new("0x00000000000000000000000000000000000000cc"),
        AccountId::new("0x00000000000000000000000000000000000000dd"),
        Side::Buy,
        100_000_000,
        1_000_000_000,
    )
    .await
    .expect("user-initiated intent");
    // Intent has NO canonical_execution_id — user-initiated quote-
    // based flow is not tied to a matcher fill in this milestone.
    assert!(
        intent.canonical_execution_id.is_none(),
        "user-initiated intent MUST NOT invent a canonical_execution_id \
         (documented as F-order 3 to link RFQ / quote paths)"
    );
    // Intent still carries the buyer + seller identities and quote
    // economics — the identity chain is broken by design for this
    // path until F-order 3 lands.
    assert_eq!(
        intent.source_type,
        OptionExecutionSourceType::OptionOrderbookFill
    );
}
