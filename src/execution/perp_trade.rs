use crate::error::{BackendError, Result};
use crate::signing::eip712::{keccak256, parse_evm_address, EIP712_DOMAIN_TYPE};
use crate::types::AccountId;
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradePayload {
    pub intent_id: B256,
    pub buyer: AccountId,
    pub seller: AccountId,
    pub market_id: u128,
    pub size_delta_1e8: u128,
    pub execution_price_1e8: u128,
    /// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — buyer's max
    /// acceptable execution price (`1e8`). `0` == strict (must equal
    /// `execution_price_1e8`, legacy V1 behaviour). Non-zero requires
    /// `execution_price_1e8 <= max_execution_price_1e8` (inclusive).
    /// Inserted between `execution_price_1e8` and `buyer_is_maker` to
    /// mirror the Solidity `PerpTrade` struct field order exactly.
    pub max_execution_price_1e8: u128,
    /// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — seller's min
    /// acceptable execution price (`1e8`). `0` == strict. Non-zero
    /// requires `execution_price_1e8 >= min_execution_price_1e8`
    /// (inclusive).
    pub min_execution_price_1e8: u128,
    pub buyer_is_maker: bool,
    pub buyer_nonce: u128,
    pub seller_nonce: u128,
    pub deadline: u128,
}

impl PerpTradePayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        intent_id: B256,
        buyer: AccountId,
        seller: AccountId,
        market_id: u128,
        size_delta_1e8: u128,
        execution_price_1e8: u128,
        max_execution_price_1e8: u128,
        min_execution_price_1e8: u128,
        buyer_is_maker: bool,
        buyer_nonce: u128,
        seller_nonce: u128,
        deadline: u128,
    ) -> Result<Self> {
        let payload = Self {
            intent_id,
            buyer,
            seller,
            market_id,
            size_delta_1e8,
            execution_price_1e8,
            max_execution_price_1e8,
            min_execution_price_1e8,
            buyer_is_maker,
            buyer_nonce,
            seller_nonce,
            deadline,
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn validate(&self) -> Result<()> {
        if self.intent_id == B256::ZERO {
            return Err(BackendError::InvalidPerpTradeIntentId);
        }
        parse_evm_address(&self.buyer)?;
        parse_evm_address(&self.seller)?;
        if self.size_delta_1e8 == 0 {
            return Err(BackendError::ZeroSize);
        }
        if self.execution_price_1e8 == 0 {
            return Err(BackendError::ZeroPrice);
        }
        // PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — user-bound
        // envelope checks. `0` reproduces V1 exact-price behaviour;
        // non-zero enforces the inclusive bound. NEVER widen — a
        // request whose bounds cannot honour the execution price is
        // rejected here rather than silently relaxed.
        if self.max_execution_price_1e8 != 0
            && self.execution_price_1e8 > self.max_execution_price_1e8
        {
            return Err(BackendError::PerpsUserBoundAboveLimit(format!(
                "execution_price_1e8 {} exceeds max_execution_price_1e8 {}",
                self.execution_price_1e8, self.max_execution_price_1e8
            )));
        }
        if self.min_execution_price_1e8 != 0
            && self.execution_price_1e8 < self.min_execution_price_1e8
        {
            return Err(BackendError::PerpsUserBoundBelowLimit(format!(
                "execution_price_1e8 {} below min_execution_price_1e8 {}",
                self.execution_price_1e8, self.min_execution_price_1e8
            )));
        }
        Ok(())
    }
}

pub fn intent_id_to_b256(intent_id: &str) -> Result<B256> {
    let mapped = B256::from(keccak256(intent_id.as_bytes()));
    if mapped == B256::ZERO {
        return Err(BackendError::InvalidPerpTradeIntentId);
    }
    Ok(mapped)
}

pub fn intent_id_to_hex_bytes32(intent_id: &str) -> Result<String> {
    Ok(b256_to_hex_bytes32(&intent_id_to_b256(intent_id)?))
}

pub fn b256_to_hex_bytes32(intent_id: &B256) -> String {
    hex_0x(intent_id.as_slice())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradeSignatureBundle {
    pub buyer_sig: Vec<u8>,
    pub seller_sig: Vec<u8>,
}

impl PerpTradeSignatureBundle {
    pub fn new(buyer_sig: &str, seller_sig: &str) -> Result<Self> {
        Ok(Self {
            buyer_sig: decode_signature(buyer_sig)?,
            seller_sig: decode_signature(seller_sig)?,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoredTradeSignatures {
    pub buyer_sig: Option<String>,
    pub seller_sig: Option<String>,
}

impl StoredTradeSignatures {
    pub fn upsert(&mut self, buyer_sig: Option<String>, seller_sig: Option<String>) -> Result<()> {
        if let Some(signature) = buyer_sig {
            validate_signature_hex(&signature)?;
            self.buyer_sig = Some(signature);
        }
        if let Some(signature) = seller_sig {
            validate_signature_hex(&signature)?;
            self.seller_sig = Some(signature);
        }
        Ok(())
    }

    pub fn buyer_signature_present(&self) -> bool {
        self.buyer_sig.is_some()
    }

    pub fn seller_signature_present(&self) -> bool {
        self.seller_sig.is_some()
    }

    pub fn calldata_ready(&self) -> bool {
        self.buyer_signature_present() && self.seller_signature_present()
    }

    pub fn missing_signatures(&self) -> bool {
        !self.calldata_ready()
    }

    pub fn bundle(&self) -> Result<Option<PerpTradeSignatureBundle>> {
        let Some(buyer_sig) = self.buyer_sig.as_deref() else {
            return Ok(None);
        };
        let Some(seller_sig) = self.seller_sig.as_deref() else {
            return Ok(None);
        };
        Ok(Some(PerpTradeSignatureBundle::new(buyer_sig, seller_sig)?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TradeSignatureStatus {
    pub buyer_signature_present: bool,
    pub seller_signature_present: bool,
    pub calldata_ready: bool,
    pub missing_signatures: bool,
}

impl From<&StoredTradeSignatures> for TradeSignatureStatus {
    fn from(value: &StoredTradeSignatures) -> Self {
        Self {
            buyer_signature_present: value.buyer_signature_present(),
            seller_signature_present: value.seller_signature_present(),
            calldata_ready: value.calldata_ready(),
            missing_signatures: value.missing_signatures(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpTradeDomain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: AccountId,
}

impl PerpTradeDomain {
    /// Legacy backward-compat constructor. Returns a V1 domain
    /// (version = "1"). Callsites that predate the versioned config
    /// still use this; new code SHOULD prefer
    /// [`PerpTradeDomain::new_v1`] / [`PerpTradeDomain::new_v2`] /
    /// [`PerpTradeDomain::for_version`] for grep-ability.
    pub fn new(chain_id: u64, verifying_contract: AccountId) -> Self {
        Self::new_v1(chain_id, verifying_contract)
    }

    /// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — canonical V1 domain.
    /// `verifying_contract` MUST be the V1 PerpMatchingEngine address.
    /// Pairs with [`perp_trade_v1_digest`] / the 10-field
    /// [`PERP_TRADE_V1_TYPE`].
    pub fn new_v1(chain_id: u64, verifying_contract: AccountId) -> Self {
        Self {
            name: "DeOptV2-PerpMatchingEngine".to_string(),
            version: "1".to_string(),
            chain_id,
            verifying_contract,
        }
    }

    /// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — canonical V2 domain.
    /// `verifying_contract` MUST be the V2 PerpMatchingEngineV2
    /// address (distinct from V1). Pairs with [`perp_trade_v2_digest`]
    /// / the 12-field [`PERP_TRADE_TYPE`]. The name/version pair
    /// matches
    /// `PerpMatchingEngineV2` `EIP712("DeOptV2-PerpMatchingEngine", "2")`
    /// exactly; any drift means backend-generated V2 signatures will
    /// NOT verify on the deployed V2 PME.
    pub fn new_v2(chain_id: u64, verifying_contract: AccountId) -> Self {
        Self {
            name: "DeOptV2-PerpMatchingEngine".to_string(),
            version: "2".to_string(),
            chain_id,
            verifying_contract,
        }
    }

    /// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — version-aware
    /// dispatcher. Callers holding a
    /// [`PerpsProtocolVersion`] (typically read from a persisted
    /// [`crate::execution::intent::ExecutionIntent::protocol_version`]
    /// or the runtime active-version config) use this so digest
    /// reconstruction cannot silently cross versions.
    pub fn for_version(
        version: PerpsProtocolVersion,
        chain_id: u64,
        verifying_contract: AccountId,
    ) -> Self {
        match version {
            PerpsProtocolVersion::V1 => Self::new_v1(chain_id, verifying_contract),
            PerpsProtocolVersion::V2 => Self::new_v2(chain_id, verifying_contract),
        }
    }
}

/// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — canonical Perps
/// settlement protocol version tag. Persisted as `TEXT` on
/// `execution_intents.protocol_version` with the exact wire strings
/// [`PerpsProtocolVersion::as_persisted_str`]. Also drives EIP-712
/// domain selection ([`PerpTradeDomain::for_version`]) and trade
/// typehash selection ([`perp_trade_digest_for_version`]).
///
/// A persisted intent's version is IMMUTABLE post-cosign: it fixes
/// the exact digest the trader signed. Changing the runtime
/// `PERPS_ACTIVE_ENGINE_VERSION` MUST NOT retarget an already-signed
/// intent — that would silently invalidate the trader's consent.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerpsProtocolVersion {
    V1,
    V2,
}

impl PerpsProtocolVersion {
    /// Persisted wire form. NEVER change — this is a DB-durable
    /// value. Any rename breaks reconciliation of pre-existing
    /// intents.
    pub const fn as_persisted_str(self) -> &'static str {
        match self {
            Self::V1 => "perp_v1",
            Self::V2 => "perp_v2",
        }
    }

    /// EIP-712 domain `version` string as it appears in the domain
    /// separator preimage. Deployed V1 PME uses `"1"`, V2 PME uses
    /// `"2"`. Verified against the Solidity source of truth at
    /// `PerpMatchingEngine.sol` / `PerpMatchingEngineV2.sol`
    /// `constructor(...)` `EIP712(...)` call.
    pub const fn domain_version_str(self) -> &'static str {
        match self {
            Self::V1 => "1",
            Self::V2 => "2",
        }
    }

    /// Parse from persisted string or operator env-var. Accepts the
    /// canonical persisted form (`perp_v1` / `perp_v2`) plus short
    /// aliases (`v1` / `v2`, `1` / `2`) for operator ergonomics.
    /// Any other value fails closed with a
    /// [`BackendError::Config`].
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "perp_v1" | "v1" | "1" => Ok(Self::V1),
            "perp_v2" | "v2" | "2" => Ok(Self::V2),
            other => Err(BackendError::Config(format!(
                "unknown Perps protocol version: {other} \
                 (expected one of: perp_v1, perp_v2)"
            ))),
        }
    }
}

impl Default for PerpsProtocolVersion {
    /// Historical default. Every persisted intent that predates the
    /// `execution_intents.protocol_version` column back-fills to V1
    /// (see migration `0064_execution_intents_protocol_version.sql`).
    fn default() -> Self {
        Self::V1
    }
}

impl fmt::Display for PerpsProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_persisted_str())
    }
}

/// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — canonical Solidity
/// `PerpTrade` type string. 12 fields; `maxExecutionPrice1e8` and
/// `minExecutionPrice1e8` are inserted between `executionPrice1e8`
/// and `buyerIsMaker`. Byte-frozen against the Solidity source of
/// truth — the keccak256 of this string is
/// `PERP_TRADE_TYPEHASH_HEX` below.
pub const PERP_TRADE_TYPE: &str = "PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,uint128 maxExecutionPrice1e8,uint128 minExecutionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";

/// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — the on-chain
/// `TRADE_TYPEHASH` locked by the Solidity side. Any drift here means
/// signatures the backend previews will NOT verify on-chain. Pinned
/// by `perp_trade_typehash_matches_locked_value` below.
pub const PERP_TRADE_TYPEHASH_HEX: &str =
    "0x9ccd368c748c5e85df8e96f94ac1d47316abde07a2d78c4f1b10b91cb98942c3";

/// Returns the runtime-computed EIP-712 typehash for `PerpTrade`.
/// Prefer the constant `PERP_TRADE_TYPEHASH_HEX` when you need the
/// hex form; this helper is here so integration tests can prove the
/// runtime keccak matches the pinned constant byte-for-byte.
pub fn perp_trade_typehash() -> [u8; 32] {
    keccak256(PERP_TRADE_TYPE.as_bytes())
}

pub fn perp_trade_digest(payload: &PerpTradePayload, domain: &PerpTradeDomain) -> Result<String> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = perp_trade_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(hex_0x(&keccak256(&encoded)))
}

/// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — raw 32-byte digest for
/// the V2 12-field PerpTrade. `domain` MUST be constructed via
/// [`PerpTradeDomain::new_v2`] (version = "2"); a V1-versioned
/// domain is refused with a [`BackendError::Config`] to prevent
/// silent cross-version replay.
pub fn perp_trade_v2_digest_bytes(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
) -> Result<[u8; 32]> {
    if domain.version != PerpsProtocolVersion::V2.domain_version_str() {
        return Err(BackendError::Config(format!(
            "perp_trade_v2_digest_bytes requires PerpTradeDomain::new_v2 \
             (domain.version = \"2\"); got version = \"{}\"",
            domain.version
        )));
    }
    let domain_separator = domain_separator(domain)?;
    let trade_hash = perp_trade_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(keccak256(&encoded))
}

/// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — hex form of
/// [`perp_trade_v2_digest_bytes`]. Same wire-lock guardrail applies.
pub fn perp_trade_v2_digest(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
) -> Result<String> {
    Ok(hex_0x(&perp_trade_v2_digest_bytes(payload, domain)?))
}

/// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — version-aware digest
/// dispatcher. Callers holding a persisted
/// [`PerpsProtocolVersion`] (from an
/// [`crate::execution::intent::ExecutionIntent`]) use this so the
/// digest reconstruction path cannot silently cross versions.
///
/// The `domain` MUST have the matching version string; a mismatch
/// (`version = V1 && domain.version = "2"`, or vice versa) fails
/// closed with a [`BackendError::Config`].
pub fn perp_trade_digest_for_version(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
    version: PerpsProtocolVersion,
) -> Result<String> {
    if domain.version != version.domain_version_str() {
        return Err(BackendError::Config(format!(
            "perp_trade_digest_for_version({}) requires domain.version = \"{}\"; \
             got version = \"{}\"",
            version.as_persisted_str(),
            version.domain_version_str(),
            domain.version
        )));
    }
    match version {
        PerpsProtocolVersion::V1 => perp_trade_v1_digest(payload, domain),
        PerpsProtocolVersion::V2 => perp_trade_v2_digest(payload, domain),
    }
}

/// PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1 — raw 32-byte version-aware
/// digest dispatcher. See [`perp_trade_digest_for_version`].
pub fn perp_trade_digest_bytes_for_version(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
    version: PerpsProtocolVersion,
) -> Result<[u8; 32]> {
    if domain.version != version.domain_version_str() {
        return Err(BackendError::Config(format!(
            "perp_trade_digest_bytes_for_version({}) requires domain.version = \"{}\"; \
             got version = \"{}\"",
            version.as_persisted_str(),
            version.domain_version_str(),
            domain.version
        )));
    }
    match version {
        PerpsProtocolVersion::V1 => perp_trade_v1_digest_bytes(payload, domain),
        PerpsProtocolVersion::V2 => perp_trade_v2_digest_bytes(payload, domain),
    }
}

// ================================================================
// PERPS_BASE_SEPOLIA_CLOSED_TEST_COSIGN_ROUTE_V1
// ================================================================
//
// The DEPLOYED Base Sepolia PME at 0x774d96…F165 uses a 10-field
// PerpTrade (NO maxExecutionPrice1e8 / minExecutionPrice1e8). The
// 12-field `PERP_TRADE_TYPE` above corresponds to a WIP V2 PME that is
// NOT yet deployed. Signatures produced against the 12-field typehash
// WILL NOT verify on the deployed V1 PME.
//
// The helpers below (`_v1` suffix) compute the DEPLOYED-V1 EIP-712
// digest. The co-sign closed-test route uses these exclusively.
// Do not swap them without a governance-timelock PME upgrade.

/// DEPLOYED V1 `PerpTrade` EIP-712 type string. 10 fields; no
/// max/min execution-price bounds. Frozen against the deployed
/// bytecode at 0x774d96E5739bffadEE91508b4D3D74F5BE29F165 —
/// `TRADE_TYPEHASH()` on-chain returns
/// [`PERP_TRADE_V1_TYPEHASH_HEX`].
pub const PERP_TRADE_V1_TYPE: &str = "PerpTrade(bytes32 intentId,address buyer,address seller,uint256 marketId,uint128 sizeDelta1e8,uint128 executionPrice1e8,bool buyerIsMaker,uint256 buyerNonce,uint256 sellerNonce,uint256 deadline)";

/// keccak256 of [`PERP_TRADE_V1_TYPE`]. Verified live against the
/// deployed PME at Base Sepolia block ≥ 46 900 000. Any drift here
/// means the co-sign route emits signatures that will NOT verify
/// on-chain — locked by
/// `perp_trade_v1_typehash_matches_deployed_value` regression test.
pub const PERP_TRADE_V1_TYPEHASH_HEX: &str =
    "0xfb345c17e97266a4c9efdc53b5baf04e3df8166f6fce15dc415758759d2e8293";

/// Runtime-computed 10-field typehash. Test-only; production code
/// should prefer the const to avoid re-hashing on every call.
pub fn perp_trade_v1_typehash() -> [u8; 32] {
    keccak256(PERP_TRADE_V1_TYPE.as_bytes())
}

/// EIP-712 structHash for the DEPLOYED 10-field `PerpTrade`. Encodes
/// ONLY the 10 fields — `max_execution_price_1e8` and
/// `min_execution_price_1e8` on the Rust payload are IGNORED (they
/// have no on-chain counterpart in the V1 struct).
fn perp_trade_v1_hash(payload: &PerpTradePayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let buyer = parse_evm_address(&payload.buyer)?;
    let seller = parse_evm_address(&payload.seller)?;
    let mut encoded = Vec::with_capacity(11 * 32);
    encoded.extend_from_slice(&perp_trade_v1_typehash());
    encoded.extend_from_slice(payload.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&buyer));
    encoded.extend_from_slice(&encode_address(&seller));
    encoded.extend_from_slice(&encode_u128(payload.market_id));
    encoded.extend_from_slice(&encode_u128(payload.size_delta_1e8));
    encoded.extend_from_slice(&encode_u128(payload.execution_price_1e8));
    encoded.extend_from_slice(&encode_bool(payload.buyer_is_maker));
    encoded.extend_from_slice(&encode_u128(payload.buyer_nonce));
    encoded.extend_from_slice(&encode_u128(payload.seller_nonce));
    encoded.extend_from_slice(&encode_u128(payload.deadline));
    Ok(keccak256(&encoded))
}

/// Full EIP-712 digest for the DEPLOYED V1 `PerpTrade`:
/// `keccak256(0x1901 || domainSeparator || structHash_v1)`. This is
/// the 32-byte value that Trader A + Trader B MUST sign with
/// their EOA keys.
///
/// Returned as a `0x…` hex string (66 chars) for interop with the
/// existing signer bin / typed-data preview APIs.
pub fn perp_trade_v1_digest(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
) -> Result<String> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = perp_trade_v1_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(hex_0x(&keccak256(&encoded)))
}

/// Returns the raw 32-byte V1 digest (no hex prefix). Same value as
/// [`perp_trade_v1_digest`] but decoded — used by the ECDSA
/// signer/recover path where a `[u8; 32]` prehash is required.
pub fn perp_trade_v1_digest_bytes(
    payload: &PerpTradePayload,
    domain: &PerpTradeDomain,
) -> Result<[u8; 32]> {
    let domain_separator = domain_separator(domain)?;
    let trade_hash = perp_trade_v1_hash(payload)?;
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&trade_hash);
    Ok(keccak256(&encoded))
}

fn validate_signature_hex(signature: &str) -> Result<()> {
    decode_signature(signature).map(|_| ())
}

fn decode_signature(signature: &str) -> Result<Vec<u8>> {
    let Some(hex) = signature.strip_prefix("0x") else {
        return Err(BackendError::MalformedSignature);
    };
    if hex.len() != 130 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::MalformedSignature);
    }

    let mut bytes = vec![0u8; 65];
    decode_hex_to_slice(hex, &mut bytes).map_err(|_| BackendError::MalformedSignature)?;
    Ok(bytes)
}

fn domain_separator(domain: &PerpTradeDomain) -> Result<[u8; 32]> {
    let verifying_contract = parse_evm_address(&domain.verifying_contract)?;
    let mut encoded = Vec::with_capacity(160);
    encoded.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.name.as_bytes()));
    encoded.extend_from_slice(&keccak256(domain.version.as_bytes()));
    encoded.extend_from_slice(&encode_u64(domain.chain_id));
    encoded.extend_from_slice(&encode_address(&verifying_contract));
    Ok(keccak256(&encoded))
}

fn perp_trade_hash(payload: &PerpTradePayload) -> Result<[u8; 32]> {
    payload.validate()?;
    let buyer = parse_evm_address(&payload.buyer)?;
    let seller = parse_evm_address(&payload.seller)?;

    // PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — the encoded body
    // includes `maxExecutionPrice1e8` + `minExecutionPrice1e8` in the
    // SAME POSITION as the Solidity struct (between
    // `executionPrice1e8` and `buyerIsMaker`). Field count grows from
    // 10 → 12; capacity bumped accordingly.
    let mut encoded = Vec::with_capacity(416);
    encoded.extend_from_slice(&perp_trade_typehash());
    encoded.extend_from_slice(payload.intent_id.as_slice());
    encoded.extend_from_slice(&encode_address(&buyer));
    encoded.extend_from_slice(&encode_address(&seller));
    encoded.extend_from_slice(&encode_u128(payload.market_id));
    encoded.extend_from_slice(&encode_u128(payload.size_delta_1e8));
    encoded.extend_from_slice(&encode_u128(payload.execution_price_1e8));
    encoded.extend_from_slice(&encode_u128(payload.max_execution_price_1e8));
    encoded.extend_from_slice(&encode_u128(payload.min_execution_price_1e8));
    encoded.extend_from_slice(&encode_bool(payload.buyer_is_maker));
    encoded.extend_from_slice(&encode_u128(payload.buyer_nonce));
    encoded.extend_from_slice(&encode_u128(payload.seller_nonce));
    encoded.extend_from_slice(&encode_u128(payload.deadline));
    Ok(keccak256(&encoded))
}

fn encode_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

fn encode_bool(value: bool) -> [u8; 32] {
    encode_u8(u8::from(value))
}

fn encode_u8(value: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[31] = value;
    word
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn hex_0x(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + bytes.len() * 2);
    encoded.push_str("0x");
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex_to_slice(hex: &str, out: &mut [u8]) -> std::result::Result<(), ()> {
    if hex.len() != out.len() * 2 {
        return Err(());
    }

    for (index, byte) in out.iter_mut().enumerate() {
        let high = decode_hex_nibble(hex.as_bytes()[index * 2])?;
        let low = decode_hex_nibble(hex.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }

    Ok(())
}

fn decode_hex_nibble(byte: u8) -> std::result::Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perp_trade_payload_validates_addresses() {
        let payload = valid_payload();

        assert_eq!(payload.market_id, 1);
        assert_eq!(
            intent_id_to_hex_bytes32("00000000-0000-0000-0000-000000000001").unwrap(),
            hex_0x(payload.intent_id.as_slice())
        );
    }

    #[test]
    fn backend_intent_id_maps_deterministically_to_bytes32() {
        let intent_id = "550e8400-e29b-41d4-a716-446655440000";

        let first = intent_id_to_b256(intent_id).unwrap();
        let second = intent_id_to_b256(intent_id).unwrap();
        let hex = intent_id_to_hex_bytes32(intent_id).unwrap();

        assert_eq!(first, second);
        assert_eq!(hex.len(), 66);
        assert!(hex.starts_with("0x"));
        assert_ne!(first, B256::ZERO);
    }

    #[test]
    fn different_backend_intent_ids_map_to_different_bytes32_values() {
        let first = intent_id_to_b256("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let second = intent_id_to_b256("550e8400-e29b-41d4-a716-446655440001").unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn zero_perp_trade_intent_id_is_rejected() {
        let error = PerpTradePayload::new(
            B256::ZERO,
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            0, // max_execution_price_1e8 — strict (legacy V1 shape)
            0, // min_execution_price_1e8 — strict
            true,
            11,
            12,
            123,
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::InvalidPerpTradeIntentId));
    }

    #[test]
    fn invalid_buyer_address_is_rejected() {
        let error = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("buyer"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            0, // max_execution_price_1e8 — strict (legacy V1 shape)
            0, // min_execution_price_1e8 — strict
            true,
            11,
            12,
            123,
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::MalformedAccountAddress));
    }

    #[test]
    fn invalid_seller_address_is_rejected() {
        let error = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("seller"),
            1,
            10,
            100,
            0, // max_execution_price_1e8 — strict (legacy V1 shape)
            0, // min_execution_price_1e8 — strict
            true,
            11,
            12,
            123,
        )
        .unwrap_err();

        assert!(matches!(error, BackendError::MalformedAccountAddress));
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let error = PerpTradeSignatureBundle::new("0x1234", &signature_hex(0xbb)).unwrap_err();

        assert!(matches!(error, BackendError::MalformedSignature));
    }

    #[test]
    fn stored_signatures_report_calldata_readiness() {
        let mut signatures = StoredTradeSignatures::default();
        signatures.upsert(Some(signature_hex(0xaa)), None).unwrap();
        assert!(signatures.buyer_signature_present());
        assert!(!signatures.seller_signature_present());
        assert!(!signatures.calldata_ready());

        signatures.upsert(None, Some(signature_hex(0xbb))).unwrap();
        assert!(signatures.calldata_ready());
        assert!(signatures.bundle().unwrap().is_some());
    }

    #[test]
    fn perp_trade_digest_is_eip712_shape() {
        let digest = perp_trade_digest(
            &valid_payload(),
            &PerpTradeDomain::new(
                84532,
                AccountId::new("0x0000000000000000000000000000000000000009"),
            ),
        )
        .unwrap();

        assert_eq!(digest.len(), 66);
        assert!(digest.starts_with("0x"));
    }

    #[test]
    fn perp_trade_digest_changes_when_intent_id_changes() {
        let domain = PerpTradeDomain::new(
            84532,
            AccountId::new("0x0000000000000000000000000000000000000009"),
        );
        let first = perp_trade_digest(&valid_payload(), &domain).unwrap();
        let second = perp_trade_digest(
            &PerpTradePayload::new(
                intent_id_to_b256("00000000-0000-0000-0000-000000000002").unwrap(),
                AccountId::new("0x0000000000000000000000000000000000000001"),
                AccountId::new("0x0000000000000000000000000000000000000002"),
                1,
                10,
                100,
                0, // max_execution_price_1e8 — strict (legacy V1 shape)
                0, // min_execution_price_1e8 — strict
                true,
                11,
                12,
                123,
            )
            .unwrap(),
            &domain,
        )
        .unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn perp_trade_digest_is_deterministic() {
        let domain = PerpTradeDomain::new(
            84532,
            AccountId::new("0x0000000000000000000000000000000000000009"),
        );

        assert_eq!(
            perp_trade_digest(&valid_payload(), &domain).unwrap(),
            perp_trade_digest(&valid_payload(), &domain).unwrap()
        );
    }

    /// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — wire-lock. Freezes
    /// the byte value of `PERP_TRADE_TYPEHASH_HEX` against the Solidity
    /// source of truth (`PerpMatchingEngine.TRADE_TYPEHASH`). Any drift
    /// here means backend-generated signature previews will NOT verify
    /// on-chain. This test is the earliest place the ripple surfaces.
    #[test]
    fn perp_trade_typehash_matches_locked_value() {
        let expected = "0x9ccd368c748c5e85df8e96f94ac1d47316abde07a2d78c4f1b10b91cb98942c3";
        assert_eq!(PERP_TRADE_TYPEHASH_HEX, expected);
        let computed = perp_trade_typehash();
        let mut hex = String::with_capacity(66);
        hex.push_str("0x");
        for byte in computed {
            hex.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(hex.as_str(), expected);
    }

    /// PERPS-PRICING-AND-EXECUTION-SAFETY-CORE-V1 — user-bound envelope
    /// fail-closed. A payload whose executionPrice exceeds the buyer's
    /// max MUST be rejected by validate_shape(); silent passthrough
    /// would let the matcher forge a fill outside the user's tolerance.
    #[test]
    fn validate_shape_rejects_execution_price_above_max_bound() {
        // `new()` runs `validate()` internally; the error surfaces
        // at construction time before any state is captured.
        let err = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            200,
            150, // max: 150; execution: 200 → out of band
            0,
            true,
            11,
            12,
            123,
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpsUserBoundAboveLimit(_)));
    }

    /// Symmetric: executionPrice below seller's min bound.
    #[test]
    fn validate_shape_rejects_execution_price_below_min_bound() {
        let err = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            80,
            0,
            100, // min: 100; execution: 80 → out of band
            true,
            11,
            12,
            123,
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::PerpsUserBoundBelowLimit(_)));
    }

    /// Both bounds set + executionPrice inside → passes. Confirms the
    /// legacy-strict path (both 0) and the new bounded path both work.
    #[test]
    fn validate_shape_accepts_execution_price_inside_bounds() {
        let payload = PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            110, // max: 110
            90,  // min: 90 — execution 100 sits inside
            true,
            11,
            12,
            123,
        )
        .unwrap();
        payload.validate().unwrap();
    }

    fn valid_payload() -> PerpTradePayload {
        PerpTradePayload::new(
            intent_id_to_b256("00000000-0000-0000-0000-000000000001").unwrap(),
            AccountId::new("0x0000000000000000000000000000000000000001"),
            AccountId::new("0x0000000000000000000000000000000000000002"),
            1,
            10,
            100,
            0, // max_execution_price_1e8 — strict (legacy V1 shape)
            0, // min_execution_price_1e8 — strict
            true,
            11,
            12,
            123,
        )
        .unwrap()
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }
}
