//! SUBACCOUNTS-V2-NONCE-TABLE-V1
//!
//! Store-level tests for the formal v2 nonce-consumption ledger
//! (`used_nonces_v2`). These pin the deterministic replay/collision
//! properties that every live v2 write-auth handler now depends on:
//!
//! * A single `(account, subaccount_id, action, nonce)` tuple can be
//!   consumed **exactly once**. Replay against the same tuple rejects.
//! * Same nonce across **different subaccounts** does not false-collide
//!   — subaccount_id 2 vs 3 for the same wallet/action/nonce both
//!   succeed independently.
//! * Same nonce across **different actions** does not false-collide —
//!   the same wallet/subaccount/nonce succeeds independently for
//!   `OPTION_ORDER_SUBMIT` and `OPTION_ORDER_CANCEL`.
//! * Same nonce across **different wallets** does not false-collide.
//! * Account matching is **case-insensitive** (the PG unique index is
//!   keyed by `lower(account)`; the in-memory store mirrors this).
//!
//! The tests exercise the in-memory `InMemoryUsedNonceV2Store`; the
//! PgRepository backend implements the same trait against the migration
//! 0041 unique index, so the same semantics apply in production.

use deopt_v2_backend::auth::write_authorization::memory_store::InMemoryUsedNonceV2Store;
use deopt_v2_backend::auth::{UsedNonceV2Store, V2NonceClaimOutcome, WriteAuthAction};
use deopt_v2_backend::types::AccountId;

fn wallet_a() -> AccountId {
    AccountId::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
}

fn wallet_b() -> AccountId {
    AccountId::new("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
}

fn nonce(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn digest(seed: u8) -> [u8; 32] {
    [seed ^ 0x5a; 32]
}

// ---------------------------------------------------------------------
// Replay: duplicate (account, subaccount_id, action, nonce) rejects.
// ---------------------------------------------------------------------

#[tokio::test]
async fn duplicate_same_tuple_rejects() {
    let store = InMemoryUsedNonceV2Store::new();
    let first = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(1),
            digest(1),
            100,
        )
        .await
        .expect("first fresh");
    assert_eq!(first, V2NonceClaimOutcome::Fresh);
    let second = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(1),
            digest(1),
            101,
        )
        .await
        .expect("second call ok");
    assert_eq!(second, V2NonceClaimOutcome::Duplicate);
}

// ---------------------------------------------------------------------
// Independence axes.
// ---------------------------------------------------------------------

#[tokio::test]
async fn same_nonce_across_subaccounts_does_not_collide() {
    let store = InMemoryUsedNonceV2Store::new();
    let a2 = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(2),
            digest(2),
            0,
        )
        .await
        .unwrap();
    let a3 = store
        .consume_v2_nonce(
            &wallet_a(),
            3,
            WriteAuthAction::OptionOrderSubmit,
            nonce(2),
            digest(2),
            0,
        )
        .await
        .unwrap();
    assert_eq!(a2, V2NonceClaimOutcome::Fresh);
    assert_eq!(a3, V2NonceClaimOutcome::Fresh);
}

#[tokio::test]
async fn same_nonce_across_actions_does_not_collide() {
    let store = InMemoryUsedNonceV2Store::new();
    let submit = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(3),
            digest(3),
            0,
        )
        .await
        .unwrap();
    let cancel = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderCancel,
            nonce(3),
            digest(3),
            0,
        )
        .await
        .unwrap();
    assert_eq!(submit, V2NonceClaimOutcome::Fresh);
    assert_eq!(cancel, V2NonceClaimOutcome::Fresh);
}

#[tokio::test]
async fn same_nonce_across_wallets_does_not_collide() {
    let store = InMemoryUsedNonceV2Store::new();
    let a = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionRfqCreate,
            nonce(4),
            digest(4),
            0,
        )
        .await
        .unwrap();
    let b = store
        .consume_v2_nonce(
            &wallet_b(),
            2,
            WriteAuthAction::OptionRfqCreate,
            nonce(4),
            digest(4),
            0,
        )
        .await
        .unwrap();
    assert_eq!(a, V2NonceClaimOutcome::Fresh);
    assert_eq!(b, V2NonceClaimOutcome::Fresh);
}

// ---------------------------------------------------------------------
// Account normalization.
// ---------------------------------------------------------------------

#[tokio::test]
async fn account_matching_is_case_insensitive() {
    let store = InMemoryUsedNonceV2Store::new();
    let lower = AccountId::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let upper = AccountId::new("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let first = store
        .consume_v2_nonce(
            &lower,
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(5),
            digest(5),
            0,
        )
        .await
        .unwrap();
    assert_eq!(first, V2NonceClaimOutcome::Fresh);
    let second = store
        .consume_v2_nonce(
            &upper,
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(5),
            digest(5),
            0,
        )
        .await
        .unwrap();
    assert_eq!(second, V2NonceClaimOutcome::Duplicate);
}

// ---------------------------------------------------------------------
// Coverage: every currently live v2 action can be claimed via the same
// store surface. Freezes the enum → store mapping so a future rename
// of `WriteAuthAction` breaks this test loudly.
// ---------------------------------------------------------------------

#[tokio::test]
async fn all_live_v2_actions_have_independent_ledger_slots() {
    let store = InMemoryUsedNonceV2Store::new();
    let actions = [
        WriteAuthAction::OptionOrderSubmit,
        WriteAuthAction::OptionOrderCancel,
        WriteAuthAction::ConditionalOrderCreate,
        WriteAuthAction::ConditionalOrderCancel,
        WriteAuthAction::OptionTwapCreate,
        WriteAuthAction::OptionTwapCancel,
        WriteAuthAction::OptionRfqCreate,
        WriteAuthAction::OptionRfqQuoteSubmit,
        WriteAuthAction::OptionRfqAccept,
        WriteAuthAction::OptionRfqCancel,
    ];
    // One nonce reused across all actions must be accepted 10 times
    // (independent slots), then rejected on the second submit-attempt
    // as a proof of same-slot duplication.
    for action in actions {
        let outcome = store
            .consume_v2_nonce(&wallet_a(), 2, action, nonce(6), digest(6), 0)
            .await
            .unwrap();
        assert_eq!(outcome, V2NonceClaimOutcome::Fresh, "action={action}");
    }
    let dup = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(6),
            digest(6),
            0,
        )
        .await
        .unwrap();
    assert_eq!(dup, V2NonceClaimOutcome::Duplicate);
}

// ---------------------------------------------------------------------
// Deterministic behaviour: the store does NOT store anything on
// Duplicate — a duplicate call is idempotent and does not mutate state.
// ---------------------------------------------------------------------

#[tokio::test]
async fn duplicate_call_is_idempotent_and_does_not_shift_state() {
    let store = InMemoryUsedNonceV2Store::new();
    store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(7),
            digest(7),
            0,
        )
        .await
        .unwrap();
    for _ in 0..3 {
        let outcome = store
            .consume_v2_nonce(
                &wallet_a(),
                2,
                WriteAuthAction::OptionOrderSubmit,
                nonce(7),
                digest(7),
                0,
            )
            .await
            .unwrap();
        assert_eq!(outcome, V2NonceClaimOutcome::Duplicate);
    }
    // A NEW nonce for the same slot still works.
    let fresh = store
        .consume_v2_nonce(
            &wallet_a(),
            2,
            WriteAuthAction::OptionOrderSubmit,
            nonce(8),
            digest(8),
            0,
        )
        .await
        .unwrap();
    assert_eq!(fresh, V2NonceClaimOutcome::Fresh);
}
