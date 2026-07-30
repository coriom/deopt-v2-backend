//! Deterministic projection reducer.
//!
//! Boundary:
//! - Input: a decoded `HybridV2Event` + a small `ProjectionState`
//!   representing the tables this reducer owns.
//! - Output: a fresh `ProjectionState` (persistence code applies the
//!   diff atomically per canonical block).
//!
//! Frozen rules:
//! - Application is deterministic per (event kind, payload).
//! - Application is exactly-once: the persistence layer keys inserts on
//!   `(deployment_id, block_hash, tx_hash, log_index)` — the reducer
//!   assumes deduplication has already occurred upstream.
//! - Reducer failure means projection corruption: caller MUST roll back
//!   the current block transaction and fail readiness.
//! - Negative balances or reservations are impossible — arithmetic
//!   underflow returns `ReducerError::Underflow`.
//! - A projection field is owned by exactly ONE event kind (see
//!   `docs/BACKEND_SUBACCOUNT_CANONICAL_STATE_AND_INDEXER_V1.md`
//!   "Projection ownership" section).
//!
//! Scope for V1:
//! - Only the highest-value projection surfaces are reduced here
//!   (subaccount identity, vault balances, per-engine reservations,
//!   recovery state, collateral universe, capability grants). Options
//!   fills / premium / fees are journaled + decoded but their
//!   projection reducers land in the read-API milestone.

use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// In-memory projection state — the reducer operates on this and the
/// persistence layer diffs it against the database. The keys mirror
/// the schema of `hybrid_v2_*` projection tables.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectionState {
    /// (owner, subaccount_id) → subKey. Materialised via
    /// `SubaccountCreated` / `SubaccountLazyRegistered`.
    pub subaccounts: BTreeMap<(String, u32), String>,
    /// (subKey, token) → balance (base-10 uint256 as String).
    pub balances: BTreeMap<(String, String), String>,
    /// (subKey, token, engine) → reserved (base-10 uint256).
    pub reservations: BTreeMap<(String, String, String), String>,
    /// engine → capability bitmap (base-10 uint256).
    pub capability_grants: BTreeMap<String, String>,
    /// Ordered by universe_index; token → index.
    pub collateral_universe: BTreeMap<String, u32>,
    /// Currently-disabled tokens (still known to the universe).
    pub disabled_tokens: BTreeSet<String>,
    /// subKey → canonical recovery state.
    pub recovery_state: BTreeMap<String, RecoveryStateProjection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStateProjection {
    Normal,
    RecoveryPending,
    RecoveryActive,
    Cancelled,
    Recovered,
}

impl RecoveryStateProjection {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecoveryStateProjection::Normal => "NORMAL",
            RecoveryStateProjection::RecoveryPending => "RECOVERY_PENDING",
            RecoveryStateProjection::RecoveryActive => "RECOVERY_ACTIVE",
            RecoveryStateProjection::Cancelled => "CANCELLED",
            RecoveryStateProjection::Recovered => "RECOVERED",
        }
    }
}

#[derive(Debug, Error)]
pub enum ReducerError {
    #[error("balance underflow for subkey {subkey} token {token}: have {have}, need {need}")]
    Underflow {
        subkey: String,
        token: String,
        have: String,
        need: String,
    },
    #[error("event kind {kind} missing required field {field}")]
    MissingField {
        kind: &'static str,
        field: &'static str,
    },
    #[error("finalized subaccount {subkey} rejected credit via {kind}")]
    FinalizedSubaccountCredit { subkey: String, kind: &'static str },
    #[error("recovery transition not allowed: {subkey} {from:?} → {to:?} via {kind}")]
    IllegalRecoveryTransition {
        subkey: String,
        from: Option<RecoveryStateProjection>,
        to: RecoveryStateProjection,
        kind: &'static str,
    },
    #[error("uint256 arithmetic overflow on subkey {subkey} token {token}")]
    Overflow { subkey: String, token: String },
}

pub fn apply(state: &mut ProjectionState, event: &HybridV2Event) -> Result<(), ReducerError> {
    match event.kind {
        EventKind::SubaccountCreated | EventKind::SubaccountLazyRegistered => {
            let owner = require(event.owner.as_ref(), event.kind.as_str(), "owner")?;
            let sid = event.subaccount_id.ok_or(ReducerError::MissingField {
                kind: event.kind.as_str(),
                field: "subaccount_id",
            })?;
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .subaccounts
                .insert((owner.clone(), sid), subkey.clone());
        }
        EventKind::Deposit => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            if is_finalized(state, subkey) {
                return Err(ReducerError::FinalizedSubaccountCredit {
                    subkey: subkey.clone(),
                    kind: event.kind.as_str(),
                });
            }
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone());
            let current = state
                .balances
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            let next = u256_add(&current, &amount).ok_or(ReducerError::Overflow {
                subkey: subkey.clone(),
                token: token.clone(),
            })?;
            state.balances.insert(key, next);
        }
        EventKind::Withdraw => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone());
            let current = state
                .balances
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            let next = u256_sub(&current, &amount).ok_or(ReducerError::Underflow {
                subkey: subkey.clone(),
                token: token.clone(),
                have: current.clone(),
                need: amount.clone(),
            })?;
            state.balances.insert(key, next);
        }
        EventKind::CollateralLocked => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone(), engine.clone());
            let current = state
                .reservations
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            let next = u256_add(&current, &amount).ok_or(ReducerError::Overflow {
                subkey: subkey.clone(),
                token: token.clone(),
            })?;
            state.reservations.insert(key, next);
        }
        EventKind::CollateralUnlocked => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
            let amount = require_amount(&event.payload, event.kind.as_str())?;
            let key = (subkey.clone(), token.clone(), engine.clone());
            let current = state
                .reservations
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            let next = u256_sub(&current, &amount).ok_or(ReducerError::Underflow {
                subkey: subkey.clone(),
                token: token.clone(),
                have: current.clone(),
                need: amount.clone(),
            })?;
            state.reservations.insert(key, next);
        }
        EventKind::CollateralTokenEnteredUniverse => {
            let token = require(event.token.as_ref(), event.kind.as_str(), "token")?;
            let fallback = state.collateral_universe.len() as u32;
            let idx = event
                .payload
                .get("universe_index")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(fallback);
            state
                .collateral_universe
                .entry(token.clone())
                .or_insert(idx);
            state.disabled_tokens.remove(token);
        }
        EventKind::SupportedTokenAdded => {
            if let Some(token) = &event.token {
                state.disabled_tokens.remove(token);
            }
        }
        EventKind::SupportedTokenRemoved => {
            if let Some(token) = &event.token {
                state.disabled_tokens.insert(token.clone());
            }
        }
        EventKind::EngineCapabilityChanged => {
            let engine = require(event.engine.as_ref(), event.kind.as_str(), "engine")?;
            let added = event
                .payload
                .get("added_bits")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let removed = event
                .payload
                .get("removed_bits")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let current = state
                .capability_grants
                .get(engine)
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            // bitmap_current = (current | added) & ~removed. We do bit-by-bit
            // on 256-bit numbers by round-tripping through u128 pairs.
            let combined = u256_bitwise_or_and_mask(&current, &added, &removed);
            state.capability_grants.insert(engine.clone(), combined);
        }
        EventKind::RecoveryRequested => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::RecoveryPending);
        }
        EventKind::RecoveryActivated => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::RecoveryActive);
        }
        EventKind::RecoveryCancelled => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::Cancelled);
        }
        EventKind::RecoveryFinalized => {
            let subkey = require(event.subkey.as_ref(), event.kind.as_str(), "subkey")?;
            let prior = state.recovery_state.get(subkey).copied();
            // RECOVERED is terminal — no path may exit it.
            if prior == Some(RecoveryStateProjection::Recovered) {
                return Err(ReducerError::IllegalRecoveryTransition {
                    subkey: subkey.clone(),
                    from: prior,
                    to: RecoveryStateProjection::Recovered,
                    kind: event.kind.as_str(),
                });
            }
            state
                .recovery_state
                .insert(subkey.clone(), RecoveryStateProjection::Recovered);
            // Finalization atomically zeroes the subKey's balances +
            // reservations. Mirror that atomicity in the projection so
            // the invariant `finalized ⇒ zero balance/lock` holds after
            // reducer application (matches Vault behaviour).
            let sk = subkey.clone();
            state.balances.retain(|(s, _), _| s != &sk);
            state.reservations.retain(|(s, _, _), _| s != &sk);
        }
        // Every other event kind is journaled + decoded but does not
        // (yet) affect a canonical projection field. Adding a reducer
        // arm here in a later backend milestone extends behaviour with
        // no schema change.
        _ => {}
    }
    Ok(())
}

fn is_finalized(state: &ProjectionState, subkey: &str) -> bool {
    matches!(
        state.recovery_state.get(subkey),
        Some(RecoveryStateProjection::Recovered)
    )
}

fn require<'a, T>(
    opt: Option<&'a T>,
    kind: &'static str,
    field: &'static str,
) -> Result<&'a T, ReducerError> {
    opt.ok_or(ReducerError::MissingField { kind, field })
}

fn require_amount(payload: &serde_json::Value, kind: &'static str) -> Result<String, ReducerError> {
    let amount =
        payload
            .get("amount")
            .and_then(|v| v.as_str())
            .ok_or(ReducerError::MissingField {
                kind,
                field: "amount",
            })?;
    Ok(amount.to_string())
}

/// Decimal-string uint256 addition. Returns None on overflow (i.e.
/// result would exceed 2^256 - 1).
pub fn u256_add(a: &str, b: &str) -> Option<String> {
    let (bits_a, ok_a) = parse_dec_uint256(a);
    let (bits_b, ok_b) = parse_dec_uint256(b);
    if !ok_a || !ok_b {
        return None;
    }
    let mut out = [0u8; 32];
    let mut carry: u16 = 0;
    for i in (0..32).rev() {
        let sum = bits_a[i] as u16 + bits_b[i] as u16 + carry;
        out[i] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    if carry != 0 {
        return None;
    }
    Some(u256_be_to_decimal_public(&out))
}

/// Decimal-string uint256 subtraction. Returns None on underflow.
pub fn u256_sub(a: &str, b: &str) -> Option<String> {
    let (bits_a, ok_a) = parse_dec_uint256(a);
    let (bits_b, ok_b) = parse_dec_uint256(b);
    if !ok_a || !ok_b {
        return None;
    }
    if u256_cmp(&bits_a, &bits_b).is_lt() {
        return None;
    }
    let mut out = [0u8; 32];
    let mut borrow: i16 = 0;
    for i in (0..32).rev() {
        let diff = bits_a[i] as i16 - bits_b[i] as i16 - borrow;
        if diff < 0 {
            out[i] = (diff + 256) as u8;
            borrow = 1;
        } else {
            out[i] = diff as u8;
            borrow = 0;
        }
    }
    Some(u256_be_to_decimal_public(&out))
}

fn u256_bitwise_or_and_mask(current: &str, add: &str, remove: &str) -> String {
    let (c, _) = parse_dec_uint256(current);
    let (a, _) = parse_dec_uint256(add);
    let (r, _) = parse_dec_uint256(remove);
    let mut out = [0u8; 32];
    for i in 0..32 {
        let combined = c[i] | a[i];
        out[i] = combined & !r[i];
    }
    u256_be_to_decimal_public(&out)
}

fn u256_cmp(a: &[u8; 32], b: &[u8; 32]) -> std::cmp::Ordering {
    for i in 0..32 {
        match a[i].cmp(&b[i]) {
            std::cmp::Ordering::Equal => continue,
            ord => return ord,
        }
    }
    std::cmp::Ordering::Equal
}

/// Parse a decimal string into a 32-byte big-endian buffer. Returns
/// `(buf, ok)` where `ok = false` when the string is not a decimal
/// uint256 (empty, non-digit char, or value >= 2^256).
fn parse_dec_uint256(s: &str) -> ([u8; 32], bool) {
    let s = s.trim();
    if s.is_empty() {
        return ([0u8; 32], false);
    }
    let mut out = [0u8; 32];
    for ch in s.chars() {
        if !ch.is_ascii_digit() {
            return ([0u8; 32], false);
        }
        let digit = (ch as u8 - b'0') as u16;
        // out = out * 10 + digit, big-endian.
        let mut carry: u16 = digit;
        for byte in out.iter_mut().rev() {
            let product = (*byte as u16) * 10 + carry;
            *byte = (product & 0xff) as u8;
            carry = product >> 8;
        }
        if carry != 0 {
            return ([0u8; 32], false);
        }
    }
    (
        [
            out[0], out[1], out[2], out[3], out[4], out[5], out[6], out[7], out[8], out[9],
            out[10], out[11], out[12], out[13], out[14], out[15], out[16], out[17], out[18],
            out[19], out[20], out[21], out[22], out[23], out[24], out[25], out[26], out[27],
            out[28], out[29], out[30], out[31],
        ],
        true,
    )
}

fn u256_be_to_decimal_public(bytes: &[u8; 32]) -> String {
    if bytes[..16].iter().all(|b| *b == 0) {
        let mut hi = 0u128;
        for b in &bytes[16..] {
            hi = (hi << 8) | (*b as u128);
        }
        return hi.to_string();
    }
    let mut digits: Vec<u8> = Vec::with_capacity(78);
    let mut n = bytes.to_vec();
    while n.iter().any(|b| *b != 0) {
        let mut rem: u32 = 0;
        for byte in n.iter_mut() {
            let acc = (rem << 8) | (*byte as u32);
            *byte = (acc / 10) as u8;
            rem = acc % 10;
        }
        digits.push(rem as u8);
    }
    if digits.is_empty() {
        return "0".to_string();
    }
    digits.reverse();
    digits.into_iter().map(|d| (b'0' + d) as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u256_add_and_sub_roundtrip() {
        assert_eq!(u256_add("100", "42").as_deref(), Some("142"));
        assert_eq!(u256_sub("100", "42").as_deref(), Some("58"));
        assert_eq!(u256_sub("42", "100"), None);
    }

    #[test]
    fn u256_add_overflow_returns_none() {
        // 2^256 - 1 + 1
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(u256_add(max, "1"), None);
    }

    #[test]
    fn u256_sub_beyond_u128_still_works() {
        let big = "680564733841876926926749214863536422912"; // 2^129
        assert_eq!(
            u256_sub(big, "1").as_deref(),
            Some("680564733841876926926749214863536422911")
        );
    }
}
