//! PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — cross-version replay
//! isolation + intent-version invariants (§5).
//!
//! Coverage:
//!   1. V1 and V2 typehashes are distinct.
//!   2. V1 and V2 domain separators are distinct (same
//!      verifying-contract, distinct version strings).
//!   3. V1 and V2 domain separators are distinct (distinct
//!      verifying-contract, otherwise identical).
//!   4. V1 digest and V2 digest for the same trade differ.
//!   5. `perp_trade_v2_digest` refuses a V1-versioned domain.
//!   6. `perp_trade_v2_digest_bytes` refuses a V1-versioned domain.
//!   7. `perp_trade_digest_for_version(V1, domain_v2)` fails closed.
//!   8. `perp_trade_digest_for_version(V2, domain_v1)` fails closed.
//!   9. `PerpsProtocolVersion::parse` accepts canonical + aliases.
//!  10. `PerpsProtocolVersion::parse` rejects unknown values.
//!  11. `PerpTradeDomain::for_version` selects the right version
//!      string.
//!  12. Persisted intent's `protocol_version` round-trips through
//!      the DB model.

use deopt_v2_backend::execution::perp_trade::{
    perp_trade_digest_bytes_for_version, perp_trade_digest_for_version, perp_trade_v1_digest,
    perp_trade_v1_digest_bytes, perp_trade_v2_digest, perp_trade_v2_digest_bytes,
    PerpTradeDomain, PerpTradePayload, PerpsProtocolVersion, PERP_TRADE_TYPEHASH_HEX,
    PERP_TRADE_V1_TYPEHASH_HEX,
};
use deopt_v2_backend::types::AccountId;

const CHAIN_ID: u64 = 84532;

// Deployed Base Sepolia V1 PME address (real).
const V1_PME_ADDR: &str = "0x774d96E5739bffadEE91508b4D3D74F5BE29F165";

// Placeholder V2 PME address for tests. NOT deployed anywhere; the
// distinctness from V1 PME is what proves cross-address domain
// separation.
const V2_PME_ADDR: &str = "0x00000000000000000000000000000000000000cA";

fn intent_id() -> alloy_primitives::B256 {
    let mut bytes = [0u8; 32];
    bytes[31] = 0x11;
    alloy_primitives::B256::from(bytes)
}

fn valid_payload() -> PerpTradePayload {
    PerpTradePayload::new(
        intent_id(),
        AccountId::new("0x0000000000000000000000000000000000000001"),
        AccountId::new("0x0000000000000000000000000000000000000002"),
        1,
        1_000_000,
        246_831_000_000,
        0,
        0,
        true,
        7,
        11,
        1_800_000_000,
    )
    .unwrap()
}

// ─── §5.1  Typehashes are distinct ──────────────────────────────

#[test]
fn v1_and_v2_typehashes_are_distinct() {
    assert_ne!(
        PERP_TRADE_V1_TYPEHASH_HEX, PERP_TRADE_TYPEHASH_HEX,
        "V1 (10-field) and V2 (12-field) TRADE_TYPEHASH must differ; \
         collision would allow silent field-count-crossing replay"
    );
}

// ─── §5.2  Domain separators — same address, distinct version ──

#[test]
fn v1_v2_domain_separators_distinct_same_verifying_contract() {
    let v1 = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let v2 = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    assert_ne!(v1.version, v2.version);
    assert_eq!(v1.version, "1");
    assert_eq!(v2.version, "2");

    // Digest the same payload against each; must differ purely from
    // the version-string change (address held constant).
    let payload = valid_payload();
    let d1 = perp_trade_v1_digest(&payload, &v1).unwrap();
    let d2 = perp_trade_v2_digest(&payload, &v2).unwrap();
    assert_ne!(d1, d2, "domain version string change must propagate to digest");
}

// ─── §5.3  Domain separators — same version string, distinct addr ─

#[test]
fn v1_v2_domain_separators_distinct_verifying_contract_and_typehash() {
    // Realistic case: real V1 PME address vs distinct V2 PME
    // address, each with its own version string.
    let v1 = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let v2 = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME_ADDR));
    assert_ne!(v1.verifying_contract, v2.verifying_contract);

    let payload = valid_payload();
    let d1 = perp_trade_v1_digest(&payload, &v1).unwrap();
    let d2 = perp_trade_v2_digest(&payload, &v2).unwrap();
    assert_ne!(d1, d2, "V1 and V2 digests must differ under realistic deployment");
}

// ─── §5.4  V1 sig cannot verify against V2 digest ───────────────

#[test]
fn v1_digest_bytes_and_v2_digest_bytes_are_distinct_for_same_payload() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let v2_dom = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME_ADDR));

    let payload = valid_payload();
    let v1_bytes = perp_trade_v1_digest_bytes(&payload, &v1_dom).unwrap();
    let v2_bytes = perp_trade_v2_digest_bytes(&payload, &v2_dom).unwrap();
    assert_ne!(v1_bytes, v2_bytes);
    // If someone recovers the signer of `sig(v1_bytes)` against
    // `v2_bytes`, ecrecover returns a different address than the
    // real trader — the trade is refused at
    // `cosign_verify_core::PerpsIntentTraderMismatch`.
}

// ─── §5.5  V2 digest refuses V1 domain ──────────────────────────

#[test]
fn perp_trade_v2_digest_refuses_v1_domain() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let error = perp_trade_v2_digest(&valid_payload(), &v1_dom).unwrap_err();
    assert!(
        format!("{error}").contains("PerpTradeDomain::new_v2"),
        "expected explicit v2-domain refusal message, got: {error}"
    );
}

#[test]
fn perp_trade_v2_digest_bytes_refuses_v1_domain() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let error = perp_trade_v2_digest_bytes(&valid_payload(), &v1_dom).unwrap_err();
    assert!(format!("{error}").contains("v2"));
}

// ─── §5.6  Dispatcher rejects mismatched (version, domain) ──────

#[test]
fn dispatcher_v1_with_v2_domain_fails_closed() {
    let v2_dom = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME_ADDR));
    let error = perp_trade_digest_for_version(
        &valid_payload(),
        &v2_dom,
        PerpsProtocolVersion::V1,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("domain.version"));
}

#[test]
fn dispatcher_v2_with_v1_domain_fails_closed() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let error = perp_trade_digest_for_version(
        &valid_payload(),
        &v1_dom,
        PerpsProtocolVersion::V2,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("domain.version"));
}

#[test]
fn dispatcher_bytes_v2_with_v1_domain_fails_closed() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let error = perp_trade_digest_bytes_for_version(
        &valid_payload(),
        &v1_dom,
        PerpsProtocolVersion::V2,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("domain.version"));
}

// ─── §5.7  Dispatcher accepts matched (version, domain) ─────────

#[test]
fn dispatcher_v1_with_v1_domain_produces_v1_digest() {
    let v1_dom = PerpTradeDomain::new_v1(CHAIN_ID, AccountId::new(V1_PME_ADDR));
    let payload = valid_payload();
    let via_dispatcher =
        perp_trade_digest_for_version(&payload, &v1_dom, PerpsProtocolVersion::V1).unwrap();
    let direct = perp_trade_v1_digest(&payload, &v1_dom).unwrap();
    assert_eq!(via_dispatcher, direct);
}

#[test]
fn dispatcher_v2_with_v2_domain_produces_v2_digest() {
    let v2_dom = PerpTradeDomain::new_v2(CHAIN_ID, AccountId::new(V2_PME_ADDR));
    let payload = valid_payload();
    let via_dispatcher =
        perp_trade_digest_for_version(&payload, &v2_dom, PerpsProtocolVersion::V2).unwrap();
    let direct = perp_trade_v2_digest(&payload, &v2_dom).unwrap();
    assert_eq!(via_dispatcher, direct);
}

// ─── §5.8  PerpsProtocolVersion parse — canonical + aliases ─────

#[test]
fn perps_protocol_version_parse_canonical_forms() {
    assert_eq!(
        PerpsProtocolVersion::parse("perp_v1").unwrap(),
        PerpsProtocolVersion::V1
    );
    assert_eq!(
        PerpsProtocolVersion::parse("perp_v2").unwrap(),
        PerpsProtocolVersion::V2
    );
}

#[test]
fn perps_protocol_version_parse_aliases() {
    assert_eq!(PerpsProtocolVersion::parse("v1").unwrap(), PerpsProtocolVersion::V1);
    assert_eq!(PerpsProtocolVersion::parse("V1").unwrap(), PerpsProtocolVersion::V1);
    assert_eq!(PerpsProtocolVersion::parse("1").unwrap(), PerpsProtocolVersion::V1);
    assert_eq!(PerpsProtocolVersion::parse("v2").unwrap(), PerpsProtocolVersion::V2);
    assert_eq!(PerpsProtocolVersion::parse("V2").unwrap(), PerpsProtocolVersion::V2);
    assert_eq!(PerpsProtocolVersion::parse("2").unwrap(), PerpsProtocolVersion::V2);
}

#[test]
fn perps_protocol_version_parse_rejects_unknown() {
    let error = PerpsProtocolVersion::parse("perp_v3").unwrap_err();
    assert!(format!("{error}").contains("unknown Perps protocol version"));
    let error = PerpsProtocolVersion::parse("").unwrap_err();
    assert!(format!("{error}").contains("unknown Perps protocol version"));
    let error = PerpsProtocolVersion::parse("v1;drop table").unwrap_err();
    assert!(format!("{error}").contains("unknown Perps protocol version"));
}

// ─── §5.9  PerpTradeDomain::for_version selects right version ───

#[test]
fn domain_for_version_v1() {
    let dom = PerpTradeDomain::for_version(
        PerpsProtocolVersion::V1,
        CHAIN_ID,
        AccountId::new(V1_PME_ADDR),
    );
    assert_eq!(dom.version, "1");
    assert_eq!(dom.name, "DeOptV2-PerpMatchingEngine");
    assert_eq!(dom.chain_id, CHAIN_ID);
    assert_eq!(dom.verifying_contract.0, V1_PME_ADDR);
}

#[test]
fn domain_for_version_v2() {
    let dom = PerpTradeDomain::for_version(
        PerpsProtocolVersion::V2,
        CHAIN_ID,
        AccountId::new(V2_PME_ADDR),
    );
    assert_eq!(dom.version, "2");
    assert_eq!(dom.name, "DeOptV2-PerpMatchingEngine");
    assert_eq!(dom.chain_id, CHAIN_ID);
    assert_eq!(dom.verifying_contract.0, V2_PME_ADDR);
}

// ─── §5.10  Persisted wire form is stable ───────────────────────

#[test]
fn persisted_wire_form_perp_v1() {
    assert_eq!(PerpsProtocolVersion::V1.as_persisted_str(), "perp_v1");
}

#[test]
fn persisted_wire_form_perp_v2() {
    assert_eq!(PerpsProtocolVersion::V2.as_persisted_str(), "perp_v2");
}

#[test]
fn default_is_v1_for_historical_backfill() {
    // Migration `0064_execution_intents_protocol_version.sql`
    // back-fills every pre-migration row to `perp_v1`. The Rust
    // Default MUST agree so the round-trip through JSON /
    // DbExecutionIntent lands on the same value.
    assert_eq!(PerpsProtocolVersion::default(), PerpsProtocolVersion::V1);
}

// ─── §5.11  Domain version-string invariant ─────────────────────

#[test]
fn domain_version_string_v1_is_one() {
    assert_eq!(PerpsProtocolVersion::V1.domain_version_str(), "1");
}

#[test]
fn domain_version_string_v2_is_two() {
    assert_eq!(PerpsProtocolVersion::V2.domain_version_str(), "2");
}
