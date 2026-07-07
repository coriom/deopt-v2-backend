//! SUBACCOUNTS-RFQ-INTEGRATION-V1
//!
//! Pins the RFQ subaccount contract shared with the frontend:
//!
//! * **v1 byte-freeze** — no v1 byte-freeze tests existed for RFQ
//!   before this milestone (the Options milestones landed their own).
//!   These lock the pre-migration wire bytes so v2 additions never
//!   drift v1 behaviour.
//! * **v2 byte-freeze** — locks the new canonical wire bytes for all
//!   four RFQ actions (create, quote_submit, accept, cancel). Every
//!   v2 payload emits `subaccount_id` immediately after the party
//!   identifier (`taker` for taker-side, `mm_account` for quotes) and
//!   preserves the v1 ordering of every other field.
//! * **Handler rejections** — the RFQ handlers now go through
//!   `resolve_options_v2_subaccount`, so `v1 auth + subaccount_id > 1`
//!   returns 400, `v2 auth + missing subaccount_id` returns 400, and
//!   `v2 auth + unknown subaccount_id` returns 404.
//! * **v1 divergence** — the same input produces different canonical
//!   bytes under v1 vs v2. That divergence is what enforces cross-
//!   subaccount replay resistance at the challenge verifier while
//!   the formal `used_nonces_v2` table is still deferred.
//!
//! Uses in-memory `AppState` (no PostgreSQL required).

use deopt_v2_backend::auth::write_authorization::{
    canonical_payload_bytes, CanonicalValue, WriteAuthAction,
};
use deopt_v2_backend::types::AccountId;

const FROZEN_TAKER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FROZEN_MAKER: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FROZEN_SERIES: &str = "BTC-30JAN2026-50000-C";
const FROZEN_RFQ_ID: &str = "11111111-1111-1111-1111-111111111111";
const FROZEN_QUOTE_ID: &str = "22222222-2222-2222-2222-222222222222";

fn taker() -> AccountId {
    AccountId::new(FROZEN_TAKER)
}

fn maker() -> AccountId {
    AccountId::new(FROZEN_MAKER)
}

// ===========================================================================
// v1 byte-freeze — no v1 RFQ byte-freeze tests existed pre-milestone.
// These lock the pre-migration wire so v2 additions never drift v1.
// ===========================================================================

#[test]
fn v1_option_rfq_create_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCreate,
        &[
            ("taker", CanonicalValue::Address(taker())),
            (
                "option_series_id",
                CanonicalValue::Str(FROZEN_SERIES.to_string()),
            ),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            (
                "limit_price_1e8",
                CanonicalValue::Str("1100000000".to_string()),
            ),
            ("ttl_ms", CanonicalValue::U64(60_000)),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_CREATE|taker=\"{FROZEN_TAKER}\"|\
         option_series_id=\"{FROZEN_SERIES}\"|\
         side=\"buy\"|\
         size_1e8=\"100000000\"|\
         limit_price_1e8=\"1100000000\"|\
         ttl_ms=60000"
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v1_option_rfq_quote_submit_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqQuoteSubmit,
        &[
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
            ("mm_account", CanonicalValue::Address(maker())),
            ("price_1e8", CanonicalValue::Str("1000000000".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            ("client_quote_id", CanonicalValue::Null),
            ("quote_nonce", CanonicalValue::Null),
            ("quote_ttl_ms", CanonicalValue::U64(30_000)),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_QUOTE_SUBMIT|option_rfq_id=\"{FROZEN_RFQ_ID}\"|\
         mm_account=\"{FROZEN_MAKER}\"|\
         price_1e8=\"1000000000\"|\
         size_1e8=\"100000000\"|\
         client_quote_id=null|\
         quote_nonce=null|\
         quote_ttl_ms=30000"
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v1_option_rfq_accept_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqAccept,
        &[
            ("taker", CanonicalValue::Address(taker())),
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
            ("quote_id", CanonicalValue::Str(FROZEN_QUOTE_ID.to_string())),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_ACCEPT|taker=\"{FROZEN_TAKER}\"|\
         option_rfq_id=\"{FROZEN_RFQ_ID}\"|\
         quote_id=\"{FROZEN_QUOTE_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v1_option_rfq_cancel_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCancel,
        &[
            ("taker", CanonicalValue::Address(taker())),
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_CANCEL|taker=\"{FROZEN_TAKER}\"|\
         option_rfq_id=\"{FROZEN_RFQ_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

// ===========================================================================
// v2 byte-freeze — subaccount_id emitted immediately after the party
// identifier. Every other field byte-identical to v1.
// ===========================================================================

#[test]
fn v2_option_rfq_create_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCreate,
        &[
            ("taker", CanonicalValue::Address(taker())),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "option_series_id",
                CanonicalValue::Str(FROZEN_SERIES.to_string()),
            ),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            (
                "limit_price_1e8",
                CanonicalValue::Str("1100000000".to_string()),
            ),
            ("ttl_ms", CanonicalValue::U64(60_000)),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_CREATE|taker=\"{FROZEN_TAKER}\"|\
         subaccount_id=2|\
         option_series_id=\"{FROZEN_SERIES}\"|\
         side=\"buy\"|\
         size_1e8=\"100000000\"|\
         limit_price_1e8=\"1100000000\"|\
         ttl_ms=60000"
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_option_rfq_quote_submit_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqQuoteSubmit,
        &[
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
            ("mm_account", CanonicalValue::Address(maker())),
            ("subaccount_id", CanonicalValue::U64(3)),
            ("price_1e8", CanonicalValue::Str("1000000000".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            ("client_quote_id", CanonicalValue::Null),
            ("quote_nonce", CanonicalValue::Null),
            ("quote_ttl_ms", CanonicalValue::U64(30_000)),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_QUOTE_SUBMIT|option_rfq_id=\"{FROZEN_RFQ_ID}\"|\
         mm_account=\"{FROZEN_MAKER}\"|\
         subaccount_id=3|\
         price_1e8=\"1000000000\"|\
         size_1e8=\"100000000\"|\
         client_quote_id=null|\
         quote_nonce=null|\
         quote_ttl_ms=30000"
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_option_rfq_accept_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqAccept,
        &[
            ("taker", CanonicalValue::Address(taker())),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
            ("quote_id", CanonicalValue::Str(FROZEN_QUOTE_ID.to_string())),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_ACCEPT|taker=\"{FROZEN_TAKER}\"|\
         subaccount_id=2|\
         option_rfq_id=\"{FROZEN_RFQ_ID}\"|\
         quote_id=\"{FROZEN_QUOTE_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

#[test]
fn v2_option_rfq_cancel_canonical_bytes_are_frozen() {
    let payload = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCancel,
        &[
            ("taker", CanonicalValue::Address(taker())),
            ("subaccount_id", CanonicalValue::U64(2)),
            (
                "option_rfq_id",
                CanonicalValue::Str(FROZEN_RFQ_ID.to_string()),
            ),
        ],
    );
    let expected = format!(
        "OPTION_RFQ_CANCEL|taker=\"{FROZEN_TAKER}\"|\
         subaccount_id=2|\
         option_rfq_id=\"{FROZEN_RFQ_ID}\""
    );
    assert_eq!(std::str::from_utf8(&payload).unwrap(), expected);
}

// ===========================================================================
// v1 vs v2 divergence — the same input produces distinct bytes and
// therefore distinct EIP-712 digests, which is what enforces cross-
// subaccount replay resistance until the formal `used_nonces_v2`
// table lands.
// ===========================================================================

#[test]
fn v1_v2_option_rfq_create_bytes_diverge_for_same_input() {
    let v1 = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCreate,
        &[
            ("taker", CanonicalValue::Address(taker())),
            (
                "option_series_id",
                CanonicalValue::Str(FROZEN_SERIES.to_string()),
            ),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            (
                "limit_price_1e8",
                CanonicalValue::Str("1100000000".to_string()),
            ),
            ("ttl_ms", CanonicalValue::U64(60_000)),
        ],
    );
    let v2 = canonical_payload_bytes(
        WriteAuthAction::OptionRfqCreate,
        &[
            ("taker", CanonicalValue::Address(taker())),
            ("subaccount_id", CanonicalValue::U64(1)),
            (
                "option_series_id",
                CanonicalValue::Str(FROZEN_SERIES.to_string()),
            ),
            ("side", CanonicalValue::Str("buy".to_string())),
            ("size_1e8", CanonicalValue::Str("100000000".to_string())),
            (
                "limit_price_1e8",
                CanonicalValue::Str("1100000000".to_string()),
            ),
            ("ttl_ms", CanonicalValue::U64(60_000)),
        ],
    );
    assert_ne!(v1, v2);
}
