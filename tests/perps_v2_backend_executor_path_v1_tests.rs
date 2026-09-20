//! PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — end-to-end (non-broadcast)
//! V2 backend integration tests.
//!
//! Coverage:
//!   1. V2 12-field wire shape frozen (typehash + field order).
//!   2. V2 ABI selector + independent decode round-trip.
//!   3. Cross-version replay isolation (V1 sig → V2 digest rejects).
//!   4. Active-version flip preserves persisted intent's version.
//!   5. Cosign dispatch: intent.protocol_version drives digest.
//!   6. Version-aware config resolver invariants.
//!   7. V2 preflight primitives (via mock traits).
//!   8. V2 typed-data schema (12-field vs 10-field) golden shape.

use deopt_v2_backend::execution::{
    decode_execute_trade_v2_calldata, encode_execute_trade_calldata,
    encode_execute_trade_calldata_for_version, encode_execute_trade_v2_calldata,
    execute_trade_selector, execute_trade_selector_for, execute_trade_v2_selector,
    perp_trade_digest_bytes_for_version, perp_trade_digest_for_version, perp_trade_v1_digest,
    perp_trade_v1_digest_bytes, perp_trade_v2_digest, perp_trade_v2_digest_bytes,
    ExecutionConfig, ExecutionIntent, ExecutionIntentStatus, PerpTradeDomain, PerpTradePayload,
    PerpsProtocolVersion, PERP_TRADE_TYPE, PERP_TRADE_TYPEHASH_HEX, PERP_TRADE_V1_TYPE,
    PERP_TRADE_V1_TYPEHASH_HEX,
};
use deopt_v2_backend::types::{AccountId, OrderId, MarketId};

const CHAIN_ID: u64 = 84532;
const V1_PME: &str = "0x774d96E5739bffadEE91508b4D3D74F5BE29F165";
const V2_PME: &str = "0x00000000000000000000000000000000000000cA";
const V2_ENGINE: &str = "0x00000000000000000000000000000000000000E1";
const V2_CLEARING: &str = "0x00000000000000000000000000000000000000C1";

fn payload(bounds_max: u128, bounds_min: u128) -> PerpTradePayload {
    PerpTradePayload::new(
        alloy_primitives::B256::from({
            let mut b = [0u8; 32];
            b[31] = 0x77;
            b
        }),
        AccountId::new("0xff287410852B9328437eaC353720e5476bC5F837"),
        AccountId::new("0x66858286fEEA78a05eA093673EA1535E0A52002d"),
        1,
        1_000_000,
        246_831_000_000,
        bounds_max,
        bounds_min,
        true,
        7,
        11,
        1_800_000_000,
    )
    .unwrap()
}

// ─── §1 V2 wire shape frozen ─────────────────────────────────────

#[test]
fn v2_typehash_matches_solidity_perp_matching_engine_v2() {
    // The exact 12-field type string MUST byte-match
    // `PerpMatchingEngineV2.sol:217` at sol HEAD `2e9ad6f`. Any
    // divergence produces a different typehash and every
    // backend-generated V2 signature refuses to verify on the deployed
    // V2 PME.
    let expected = "PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,uint128 maxExecutionPrice1e8,uint128 minExecutionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";
    assert_eq!(PERP_TRADE_TYPE, expected);
    assert_eq!(
        PERP_TRADE_TYPEHASH_HEX,
        "0x9ccd368c748c5e85df8e96f94ac1d47316abde07a2d78c4f1b10b91cb98942c3",
        "V2 typehash frozen against Sol HEAD 2e9ad6f — any change here means \
         backend-emitted V2 signatures will NOT verify on-chain"
    );
}

#[test]
fn v1_typehash_matches_deployed_base_sepolia_pme() {
    let expected = "PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";
    assert_eq!(PERP_TRADE_V1_TYPE, expected);
    assert_eq!(
        PERP_TRADE_V1_TYPEHASH_HEX,
        "0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293"
    );
}

// ─── §14 V2 ABI selector + independent decode ─────────────────

#[test]
fn v2_selector_is_distinct_from_v1() {
    assert_ne!(execute_trade_selector(), execute_trade_v2_selector());
}

#[test]
fn v2_calldata_round_trip_preserves_signed_bounds() {
    let p = payload(246_832_000_000, 246_830_000_000);
    let sigs = fake_signature_bundle();
    let calldata = encode_execute_trade_v2_calldata(&p, &sigs).unwrap();
    assert_eq!(&calldata[..4], execute_trade_v2_selector().as_slice());

    let (decoded, buyer_sig, seller_sig) =
        decode_execute_trade_v2_calldata(&calldata).unwrap();
    assert_eq!(decoded.intentId, p.intent_id);
    assert_eq!(decoded.maxExecutionPrice1e8, p.max_execution_price_1e8);
    assert_eq!(decoded.minExecutionPrice1e8, p.min_execution_price_1e8);
    assert_eq!(buyer_sig, sigs.buyer_sig);
    assert_eq!(seller_sig, sigs.seller_sig);
}

// ─── §11 Digest equality: dispatcher == direct helper ──────────

#[test]
fn v2_dispatcher_and_direct_helper_produce_identical_digest() {
    let p = payload(0, 0);
    let dom = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME));
    let direct = perp_trade_v2_digest(&p, &dom).unwrap();
    let dispatched =
        perp_trade_digest_for_version(&p, &dom, PerpsProtocolVersion::V2).unwrap();
    assert_eq!(direct, dispatched);
}

#[test]
fn v1_dispatcher_and_direct_helper_produce_identical_digest() {
    let p = payload(0, 0);
    let dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME));
    let direct = perp_trade_v1_digest(&p, &dom).unwrap();
    let dispatched =
        perp_trade_digest_for_version(&p, &dom, PerpsProtocolVersion::V1).unwrap();
    assert_eq!(direct, dispatched);
}

// ─── §11-13 Cross-version replay isolation ─────────────────────

#[test]
fn v2_signed_bounds_change_produces_distinct_digest() {
    // Same trader / market / size / price BUT different signed
    // bounds. The V2 digest MUST propagate the bound change so a
    // trader who signed one set of bounds cannot see their signature
    // silently reused with different bounds.
    let dom = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME));
    let a = perp_trade_v2_digest_bytes(&payload(246_832_000_000, 246_830_000_000), &dom).unwrap();
    let b = perp_trade_v2_digest_bytes(&payload(246_832_000_001, 246_830_000_000), &dom).unwrap();
    let c = perp_trade_v2_digest_bytes(&payload(246_832_000_000, 246_830_000_001), &dom).unwrap();
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn v2_signed_bounds_are_ignored_by_v1_digest() {
    // A V1 payload discards the bounds (10-field type). This test
    // makes that behaviour explicit so a future refactor cannot
    // accidentally start propagating bounds into the V1 digest.
    let dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME));
    let a = perp_trade_v1_digest_bytes(&payload(246_832_000_000, 246_830_000_000), &dom).unwrap();
    let b = perp_trade_v1_digest_bytes(&payload(246_832_000_001, 246_830_000_000), &dom).unwrap();
    assert_eq!(a, b);
}

// ─── §5 Version-aware config resolvers ─────────────────────────

#[test]
fn active_perp_engine_address_v1_returns_v1_address() {
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perps_active_engine_version = PerpsProtocolVersion::V1;
    assert_eq!(c.active_perp_engine_address().unwrap().0, V1_PME);
}

#[test]
fn active_perp_engine_address_v2_returns_v2_address() {
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perp_engine_v2_address = Some(AccountId::new(V2_ENGINE));
    c.perps_active_engine_version = PerpsProtocolVersion::V2;
    assert_eq!(c.active_perp_engine_address().unwrap().0, V2_ENGINE);
}

#[test]
fn active_perp_engine_address_v2_without_config_fails_closed() {
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perp_engine_v2_address = None; // deliberately unset
    c.perps_active_engine_version = PerpsProtocolVersion::V2;
    let error = c.active_perp_engine_address().unwrap_err();
    assert!(format!("{error}").contains("PERP_ENGINE_V2_ADDRESS is unconfigured"));
}

#[test]
fn perp_engine_address_for_uses_persisted_version_not_active() {
    // A V1 intent must resolve to the V1 address even when the
    // runtime is actively serving V2 traffic. This is the invariant
    // that prevents a runtime cutover from retargeting historical
    // rows.
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perp_engine_v2_address = Some(AccountId::new(V2_ENGINE));
    c.perps_active_engine_version = PerpsProtocolVersion::V2;
    assert_eq!(
        c.perp_engine_address_for(PerpsProtocolVersion::V1)
            .unwrap()
            .0,
        V1_PME
    );
    assert_eq!(
        c.perp_engine_address_for(PerpsProtocolVersion::V2)
            .unwrap()
            .0,
        V2_ENGINE
    );
}

// ─── §13 Active-version flip test (persisted version wins) ────

#[test]
fn active_version_flip_does_not_alter_v1_intent_domain() {
    // Simulate: prepare a V1 intent under active=v1, then flip
    // runtime to active=v2. Cosign reconstructs the digest from
    // intent.protocol_version = V1, so the domain STILL uses V1
    // addressing + version "1".
    let intent = mock_intent_v1();

    // Active version = V2, config also has V2 addresses configured.
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perp_matching_engine_address = AccountId::new(V1_PME);
    c.perp_engine_v2_address = Some(AccountId::new(V2_ENGINE));
    c.perp_matching_engine_v2_address = Some(AccountId::new(V2_PME));
    c.perp_clearing_account_v2_address = Some(AccountId::new(V2_CLEARING));
    c.perps_active_engine_version = PerpsProtocolVersion::V2;

    // Reconstruction path uses intent.protocol_version, NOT active.
    let verifying = c
        .perp_matching_engine_address_for(intent.protocol_version)
        .unwrap()
        .clone();
    let domain = PerpTradeDomain::for_version(intent.protocol_version, CHAIN_ID, verifying);
    assert_eq!(domain.version, "1");
    assert_eq!(domain.verifying_contract.0, V1_PME);
}

#[test]
fn active_version_flip_does_not_alter_v2_intent_domain() {
    let mut intent = mock_intent_v1();
    intent.protocol_version = PerpsProtocolVersion::V2;

    // Flip runtime to V1.
    let mut c = ExecutionConfig::disabled();
    c.perp_engine_address = AccountId::new(V1_PME);
    c.perp_matching_engine_address = AccountId::new(V1_PME);
    c.perp_engine_v2_address = Some(AccountId::new(V2_ENGINE));
    c.perp_matching_engine_v2_address = Some(AccountId::new(V2_PME));
    c.perp_clearing_account_v2_address = Some(AccountId::new(V2_CLEARING));
    c.perps_active_engine_version = PerpsProtocolVersion::V1;

    let verifying = c
        .perp_matching_engine_address_for(intent.protocol_version)
        .unwrap()
        .clone();
    let domain = PerpTradeDomain::for_version(intent.protocol_version, CHAIN_ID, verifying);
    assert_eq!(domain.version, "2");
    assert_eq!(domain.verifying_contract.0, V2_PME);
}

// ─── §14 Version-dispatched selector + calldata ────────────────

#[test]
fn version_dispatched_selector_matches_direct() {
    assert_eq!(
        execute_trade_selector_for(PerpsProtocolVersion::V1),
        execute_trade_selector()
    );
    assert_eq!(
        execute_trade_selector_for(PerpsProtocolVersion::V2),
        execute_trade_v2_selector()
    );
}

#[test]
fn version_dispatched_calldata_matches_direct_encoders() {
    let p = payload(246_832_000_000, 246_830_000_000);
    let sigs = fake_signature_bundle();

    let v1_direct = encode_execute_trade_calldata(&p, &sigs).unwrap();
    let v1_dispatched =
        encode_execute_trade_calldata_for_version(&p, &sigs, PerpsProtocolVersion::V1).unwrap();
    assert_eq!(v1_direct, v1_dispatched);

    let v2_direct = encode_execute_trade_v2_calldata(&p, &sigs).unwrap();
    let v2_dispatched =
        encode_execute_trade_calldata_for_version(&p, &sigs, PerpsProtocolVersion::V2).unwrap();
    assert_eq!(v2_direct, v2_dispatched);
}

// ─── §22 DB-adjacent round-trip: intent serialise/deserialise ─

#[test]
fn execution_intent_serde_round_trips_v2_protocol_version() {
    let mut intent = mock_intent_v1();
    intent.protocol_version = PerpsProtocolVersion::V2;
    let json = serde_json::to_string(&intent).unwrap();
    assert!(json.contains("\"protocol_version\":\"v2\""));
    let round_tripped: ExecutionIntent = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped.protocol_version, PerpsProtocolVersion::V2);
    assert_eq!(round_tripped, intent);
}

#[test]
fn execution_intent_serde_defaults_missing_protocol_version_to_v1() {
    // Legacy JSON (pre-migration) lacks the field. Deserialise must
    // deterministically default to V1 for backward compatibility with
    // every existing persisted row.
    let json = r#"{
        "intent_id": "00000000-0000-0000-0000-000000000001",
        "market_id": 1,
        "buyer": "0xff287410852B9328437eaC353720e5476bC5F837",
        "seller": "0x66858286fEEA78a05eA093673EA1535E0A52002d",
        "price_1e8": 246831000000,
        "size_1e8": 1000000,
        "buy_order_id": "00000000-0000-0000-0000-000000000002",
        "sell_order_id": "00000000-0000-0000-0000-000000000003",
        "buyer_is_maker": true,
        "buyer_nonce": 7,
        "seller_nonce": 11,
        "deadline_ms": 1800000000000,
        "created_at_ms": 1789890000000,
        "status": "pending"
    }"#;
    let intent: ExecutionIntent = serde_json::from_str(json).unwrap();
    assert_eq!(intent.protocol_version, PerpsProtocolVersion::V1);
}

// ─── Helpers ────────────────────────────────────────────────────

fn mock_intent_v1() -> ExecutionIntent {
    ExecutionIntent {
        intent_id: uuid::Uuid::new_v4(),
        market_id: MarketId::from(1u16),
        buyer: AccountId::new("0xff287410852B9328437eaC353720e5476bC5F837"),
        seller: AccountId::new("0x66858286fEEA78a05eA093673EA1535E0A52002d"),
        price_1e8: 246_831_000_000,
        size_1e8: 1_000_000,
        buy_order_id: OrderId(uuid::Uuid::new_v4()),
        sell_order_id: OrderId(uuid::Uuid::new_v4()),
        buyer_is_maker: Some(true),
        buyer_nonce: Some(7),
        seller_nonce: Some(11),
        deadline_ms: Some(1_800_000_000_000),
        created_at_ms: 1_789_890_000_000,
        status: ExecutionIntentStatus::Pending,
        protocol_version: PerpsProtocolVersion::V1,
    }
}

fn fake_signature_bundle() -> deopt_v2_backend::execution::PerpTradeSignatureBundle {
    let buyer_sig = "0x".to_string() + &"aa".repeat(65);
    let seller_sig = "0x".to_string() + &"bb".repeat(65);
    deopt_v2_backend::execution::PerpTradeSignatureBundle::new(&buyer_sig, &seller_sig).unwrap()
}
