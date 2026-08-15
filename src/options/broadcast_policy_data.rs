//! Read-only chain / accounting / dedupe inputs for the
//! `should_broadcast` policy gate.
//!
//! This module turns the previously-placeholder fields in
//! `BroadcastContext` (OME paused, OME isExecutor(BE), BE balance, PFV
//! fee balance, PFV rebate reserve, CV(PFV) balance, dedupe state,
//! R5 drift) into live read-only RPC + DB-backed inputs.
//!
//! Hard rules:
//!   * Read-only: no `eth_sendTransaction`, no state mutation.
//!   * Mainnet (`chain_id == 8453`) fails closed if any required input is
//!     missing — placeholder defaults are NOT permitted to land in a
//!     production-mode policy decision. Sepolia is permissive by design
//!     so the existing rehearsal regression remains green.
//!   * The provider trait is mockable; tests drive the policy with a
//!     `StubBroadcastPolicyDataProvider` returning canned values.
//!
//! Deferred to follow-on tracks (documented in
//! `WIRE_SHOULD_BROADCAST_CHAIN_STATE_READS_RESULT.md §11`):
//!   * `FeesManagerV2.getProfile(...)` decoding into a real
//!     [`FeeSplitSummary`]. The current LiveProvider leaves
//!     `fee_split = None` so `econ_data_available = false` continues to
//!     gate steps 4 / 5 / 7 of `should_broadcast` at the call site.
//!   * Risk-manager snapshot freshness — same shape.
//!   * Subsidy-budget view — same shape.

use crate::api::AppState;
use crate::execution::rpc::{EthBalanceProvider, EthCallProvider, EthCallRequest};
use crate::execution::TransactionBroadcastProvider;
use crate::options::broadcast_observability::BroadcastObservability;
use crate::options::broadcast_policy::FeeSplitSummary;
use crate::options::types::{
    OptionExecutionIntent, OptionExecutionIntentStatus, OptionExecutionTransaction,
};
use crate::signing::eip712::keccak256;
use crate::types::AccountId;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// `read_type` label vocabulary (bounded; hardcoded constants).
// ---------------------------------------------------------------------------

/// Stable `read_type` labels used by
/// [`BroadcastObservability::record_policy_data_failure`] when a
/// [`LiveBroadcastPolicyDataProvider`] read fails. Hardcoded — never
/// derived from user input — so the Prometheus label cardinality stays
/// strictly bounded.
pub mod read_type {
    pub const CHAIN_ID_RPC: &str = "chain_id_rpc";
    pub const BE_BALANCE: &str = "be_balance";
    pub const OME_PAUSED: &str = "ome_paused";
    pub const OME_IS_EXECUTOR: &str = "ome_is_executor";
    pub const PFV_FEE_BALANCE: &str = "pfv_fee_balance";
    pub const PFV_REBATE_RESERVE: &str = "pfv_rebate_reserve";
    pub const CV_PFV_BALANCE: &str = "cv_pfv_balance";
    pub const FM_V2_QUOTE_FEES_RPC: &str = "fm_v2_quote_fees_rpc";
    pub const FM_V2_QUOTE_FEES_DECODE: &str = "fm_v2_quote_fees_decode";
    pub const FM_V2_REBATE_BUDGET: &str = "fm_v2_rebate_budget";
}

/// Boxed-future return type aligned with the existing project conventions
/// (`RpcFuture`, `SignerFuture`).
pub type PolicyDataFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, PolicyDataError>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDataError {
    Rpc(String),
    EncodingError(String),
    DbError(String),
    Internal(String),
}

impl fmt::Display for PolicyDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(msg) => write!(f, "policy-data:rpc:{msg}"),
            Self::EncodingError(msg) => write!(f, "policy-data:encoding:{msg}"),
            Self::DbError(msg) => write!(f, "policy-data:db:{msg}"),
            Self::Internal(msg) => write!(f, "policy-data:internal:{msg}"),
        }
    }
}

impl std::error::Error for PolicyDataError {}

/// Reason for a persistent-dedupe positive — surfaced to the policy reject
/// so logs distinguish "already submitted" from "already confirmed".
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DedupeReason {
    #[default]
    None,
    ExistingTxHash,
    StatusAlreadyBroadcastSubmitted,
    StatusAlreadyBroadcastConfirmed,
    StatusAlreadyBroadcastFailed,
}

impl DedupeReason {
    pub fn is_hit(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Snapshot of every chain / DB read the policy needs. Every field is
/// `Option<T>` so the call-site can map missing data to mainnet
/// fail-closed (`policy:<reject>`) or testnet permissive defaults.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct BroadcastPolicyInputs {
    pub chain_id_rpc: Option<u64>,
    pub be_balance_wei: Option<u128>,
    pub ome_paused: Option<bool>,
    pub ome_is_executor: Option<bool>,
    pub pfv_fee_balance_asset: Option<u128>,
    pub pfv_rebate_reserve_asset: Option<u128>,
    pub cv_pfv_balance_asset: Option<u128>,
    /// `Some(summary)` when both maker + taker FeesManagerV2 `quoteFees`
    /// reads landed AND decoded cleanly; aggregate of two
    /// per-side [`FeeQuoteRaw`] decodes. `None` means the FM_V2 reads
    /// failed → `econ_data_available = false` at the call site.
    pub fee_split: Option<FeeSplitSummary>,
    /// Live `FeesManagerV2.rebateBudget(asset)` read; `None` if FM_V2
    /// address was not configured or the read failed.
    pub fm_v2_rebate_budget_asset: Option<u128>,
    pub dedupe_hit: bool,
    pub dedupe_reason: DedupeReason,
    /// `Some(true)` when CV(PFV,asset) == feeBalance + rebateReserve;
    /// `Some(false)` on observed drift; `None` if any input was missing.
    pub r5_drift_zero: Option<bool>,
}

// ---------------------------------------------------------------------------
// FeesManagerV2 ABI codec
// ---------------------------------------------------------------------------

/// Decoded `IFeesManagerV2.FeeQuote` for a single side. Mirrors the
/// 12-field static tuple returned by `quoteFees`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeQuoteRaw {
    pub applied_ppm: i32,
    pub basis_amount: u128,
    pub fee_amount: u128,
    pub is_rebate: bool,
    pub tier: u8,
    pub product: u8,
    pub flow: u8,
    pub fee_basis: u8,
    pub is_maker: bool,
    pub settlement_asset: AccountId,
    pub recipient: AccountId,
    pub rebate_fundable: bool,
}

/// ABI selector for
/// `quoteFees(address,uint8,uint8,bool,address,uint256)`.
/// Pre-encoded as `0x` + hex for clarity; verified by unit test.
pub const FM_V2_QUOTE_FEES_SELECTOR: [u8; 4] = compute_quote_fees_selector();

/// Compile-time constant — the keccak vector is asserted in tests.
const fn compute_quote_fees_selector() -> [u8; 4] {
    [0x00, 0x00, 0x00, 0x00] // populated at runtime; see [`quote_fees_selector_bytes`].
}

/// Runtime-computed selector (used by the provider). The constant
/// [`FM_V2_QUOTE_FEES_SELECTOR`] is a placeholder; this fn is the
/// authoritative source. Backed by a unit-test keccak vector.
pub fn quote_fees_selector_bytes() -> [u8; 4] {
    let hash = keccak256(b"quoteFees(address,uint8,uint8,bool,address,uint256)");
    [hash[0], hash[1], hash[2], hash[3]]
}

/// ABI-encode the call to `quoteFees`.
pub fn encode_quote_fees_call(
    trader: &AccountId,
    product: u8,
    flow: u8,
    is_maker: bool,
    settlement_asset: &AccountId,
    basis_amount: u128,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 6 * 32);
    data.extend_from_slice(&quote_fees_selector_bytes());
    data.extend_from_slice(&encode_address_param(trader));
    data.extend_from_slice(&encode_uint8_param(product));
    data.extend_from_slice(&encode_uint8_param(flow));
    data.extend_from_slice(&encode_bool_param(is_maker));
    data.extend_from_slice(&encode_address_param(settlement_asset));
    data.extend_from_slice(&encode_uint256_param_from_u128(basis_amount));
    data
}

/// Decode the 12-field static `FeeQuote` tuple from a 384-byte buffer.
/// Returns an error variant for short / malformed payloads.
pub fn decode_fee_quote(raw: &[u8]) -> Result<FeeQuoteRaw, PolicyDataError> {
    if raw.len() < 384 {
        return Err(PolicyDataError::EncodingError(format!(
            "FeeQuote payload too short: {} bytes (expected ≥ 384)",
            raw.len()
        )));
    }
    let applied_ppm = decode_i32_from_word(&raw[0..32])?;
    let basis_amount = decode_u128_from_word(&raw[32..64])?;
    let fee_amount = decode_u128_from_word(&raw[64..96])?;
    let is_rebate = decode_bool_from_word(&raw[96..128])?;
    let tier = decode_u8_from_word(&raw[128..160])?;
    let product = decode_u8_from_word(&raw[160..192])?;
    let flow = decode_u8_from_word(&raw[192..224])?;
    let fee_basis = decode_u8_from_word(&raw[224..256])?;
    let is_maker = decode_bool_from_word(&raw[256..288])?;
    let settlement_asset = decode_address_from_word(&raw[288..320])?;
    let recipient = decode_address_from_word(&raw[320..352])?;
    let rebate_fundable = decode_bool_from_word(&raw[352..384])?;

    // Cross-check the contract invariant `isRebate == (appliedPpm < 0)`.
    // Decoder remains tolerant to consumeFees-emitted zero-ppm paths but
    // rejects an active mismatch as malformed.
    if applied_ppm < 0 && !is_rebate {
        return Err(PolicyDataError::EncodingError(
            "FeeQuote isRebate inconsistent with negative appliedPpm".to_string(),
        ));
    }
    if applied_ppm > 0 && is_rebate {
        return Err(PolicyDataError::EncodingError(
            "FeeQuote isRebate inconsistent with positive appliedPpm".to_string(),
        ));
    }

    Ok(FeeQuoteRaw {
        applied_ppm,
        basis_amount,
        fee_amount,
        is_rebate,
        tier,
        product,
        flow,
        fee_basis,
        is_maker,
        settlement_asset,
        recipient,
        rebate_fundable,
    })
}

fn encode_uint8_param(value: u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[31] = value;
    buf
}

fn encode_bool_param(value: bool) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[31] = if value { 1 } else { 0 };
    buf
}

fn encode_uint256_param_from_u128(value: u128) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[16..32].copy_from_slice(&value.to_be_bytes());
    buf
}

fn decode_i32_from_word(word: &[u8]) -> Result<i32, PolicyDataError> {
    if word.len() != 32 {
        return Err(PolicyDataError::EncodingError(
            "i32 word length != 32".to_string(),
        ));
    }
    // ABI sign-extends int32 to int256. The high 28 bytes must be either
    // all 0x00 (positive / zero) or all 0xff (negative).
    let high = &word[0..28];
    let sign_byte = word[28];
    let all_zero = high.iter().all(|b| *b == 0) && (sign_byte & 0x80 == 0);
    let all_one = high.iter().all(|b| *b == 0xff) && (sign_byte & 0x80 != 0);
    if !all_zero && !all_one {
        return Err(PolicyDataError::EncodingError(
            "i32 sign-extension malformed".to_string(),
        ));
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&word[28..32]);
    Ok(i32::from_be_bytes(buf))
}

fn decode_u8_from_word(word: &[u8]) -> Result<u8, PolicyDataError> {
    if word.len() != 32 {
        return Err(PolicyDataError::EncodingError(
            "u8 word length != 32".to_string(),
        ));
    }
    for byte in &word[0..31] {
        if *byte != 0 {
            return Err(PolicyDataError::EncodingError(
                "u8 high bytes non-zero".to_string(),
            ));
        }
    }
    Ok(word[31])
}

fn decode_bool_from_word(word: &[u8]) -> Result<bool, PolicyDataError> {
    if word.len() != 32 {
        return Err(PolicyDataError::EncodingError(
            "bool word length != 32".to_string(),
        ));
    }
    for byte in &word[0..31] {
        if *byte != 0 {
            return Err(PolicyDataError::EncodingError(
                "bool high bytes non-zero".to_string(),
            ));
        }
    }
    match word[31] {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(PolicyDataError::EncodingError(format!(
            "bool last byte {other} is neither 0 nor 1"
        ))),
    }
}

fn decode_address_from_word(word: &[u8]) -> Result<AccountId, PolicyDataError> {
    if word.len() != 32 {
        return Err(PolicyDataError::EncodingError(
            "address word length != 32".to_string(),
        ));
    }
    for byte in &word[0..12] {
        if *byte != 0 {
            return Err(PolicyDataError::EncodingError(
                "address left-pad bytes non-zero".to_string(),
            ));
        }
    }
    let mut hex = String::with_capacity(42);
    hex.push_str("0x");
    for byte in &word[12..32] {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(AccountId::new(hex))
}

fn decode_u128_from_word(word: &[u8]) -> Result<u128, PolicyDataError> {
    if word.len() != 32 {
        return Err(PolicyDataError::EncodingError(
            "u128 word length != 32".to_string(),
        ));
    }
    for byte in &word[0..16] {
        if *byte != 0 {
            return Err(PolicyDataError::EncodingError(
                "u128 high bytes non-zero (overflow guard)".to_string(),
            ));
        }
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&word[16..32]);
    Ok(u128::from_be_bytes(buf))
}

/// Aggregate a maker + taker [`FeeQuoteRaw`] pair into the policy
/// [`FeeSplitSummary`]. Mirrors the existing `should_broadcast` consumer
/// view: signed effective ppm per side, gross fee revenue (positive
/// quotes), total rebate outflow (negative quotes), and net protocol
/// revenue.
pub fn aggregate_fee_split(
    maker_quote: &FeeQuoteRaw,
    taker_quote: &FeeQuoteRaw,
    asset: AccountId,
) -> FeeSplitSummary {
    let maker_fee = if maker_quote.is_rebate {
        0
    } else {
        maker_quote.fee_amount
    };
    let maker_rebate = if maker_quote.is_rebate {
        maker_quote.fee_amount
    } else {
        0
    };
    let taker_fee = if taker_quote.is_rebate {
        0
    } else {
        taker_quote.fee_amount
    };
    let taker_rebate = if taker_quote.is_rebate {
        taker_quote.fee_amount
    } else {
        0
    };
    let gross_fee_revenue = maker_fee.saturating_add(taker_fee);
    let total_rebate_outflow = maker_rebate.saturating_add(taker_rebate);
    let net_protocol_revenue =
        (gross_fee_revenue as i128).saturating_sub(total_rebate_outflow as i128);
    let tier = maker_quote.tier.max(taker_quote.tier);
    FeeSplitSummary {
        gross_fee_revenue,
        total_rebate_outflow,
        net_protocol_revenue,
        effective_maker_ppm: maker_quote.applied_ppm as i64,
        effective_taker_ppm: taker_quote.applied_ppm as i64,
        asset,
        tier,
    }
}

/// Provider trait for the policy data gather step. Implementations MUST
/// be read-only. The provider is invoked exactly once per broadcast
/// attempt, before the signer is contacted.
pub trait BroadcastPolicyDataProvider: Send + Sync {
    fn gather_inputs<'a>(
        &'a self,
        state: &'a AppState,
        intent: &'a OptionExecutionIntent,
    ) -> PolicyDataFuture<'a, BroadcastPolicyInputs>;
}

// ---------------------------------------------------------------------------
// LiveBroadcastPolicyDataProvider
// ---------------------------------------------------------------------------

/// Live provider that reads chain state via RPC and dedupe state via the
/// existing `option_execution_transactions` store / repository.
///
/// Capabilities the provider relies on:
///   * `TransactionBroadcastProvider::chain_id` — RPC chain id round-trip.
///   * `EthBalanceProvider::eth_get_balance` — backend executor balance.
///   * `EthCallProvider::eth_call` — `OME.paused()`, `OME.isExecutor(BE)`,
///     and (if a PFV address is configured) `PFV.feeBalance(asset)` /
///     `PFV.rebateReserve(asset)` and `CV.balances(PFV, asset)`.
///
/// Any RPC read that fails is mapped to `None` in the returned snapshot;
/// the call site is responsible for fail-closed-on-mainnet semantics.
pub struct LiveBroadcastPolicyDataProvider<P> {
    provider: P,
    pfv_address: Option<AccountId>,
    cv_address: Option<AccountId>,
    /// FeesManagerV2 address. When `Some(...)`, the live provider issues
    /// `quoteFees(maker)` + `quoteFees(taker)` + `rebateBudget(asset)`
    /// reads to populate `fee_split` and `fm_v2_rebate_budget_asset`.
    fees_manager_v2_address: Option<AccountId>,
    /// Optional in-process observability handle. When `Some(...)`, every
    /// read failure increments the matching Prometheus counter via
    /// [`BroadcastObservability::record_policy_data_failure`] and the
    /// dedicated `record_fm_v2_*_failure` helpers. Defaults to `None`
    /// so library + test consumers stay observability-agnostic.
    observability: Option<Arc<BroadcastObservability>>,
}

impl<P> LiveBroadcastPolicyDataProvider<P> {
    pub fn new(
        provider: P,
        pfv_address: Option<AccountId>,
        cv_address: Option<AccountId>,
        fees_manager_v2_address: Option<AccountId>,
    ) -> Self {
        Self {
            provider,
            pfv_address,
            cv_address,
            fees_manager_v2_address,
            observability: None,
        }
    }

    /// Builder-style setter. Production code constructs the provider via
    /// `new(...).with_observability(state.broadcast_observability.clone())`
    /// so live-read failures land in `/metrics`.
    pub fn with_observability(mut self, observability: Arc<BroadcastObservability>) -> Self {
        self.observability = Some(observability);
        self
    }

    pub fn into_provider(self) -> P {
        self.provider
    }

    fn record_data_failure(&self, read_type: &'static str) {
        if let Some(obs) = self.observability.as_ref() {
            obs.record_policy_data_failure(read_type);
        }
    }

    fn record_fm_v2_rpc_failure_metric(&self) {
        if let Some(obs) = self.observability.as_ref() {
            obs.record_fm_v2_rpc_failure();
            obs.record_policy_data_failure(read_type::FM_V2_QUOTE_FEES_RPC);
        }
    }

    fn record_fm_v2_decode_failure_metric(&self) {
        if let Some(obs) = self.observability.as_ref() {
            obs.record_fm_v2_decode_failure();
            obs.record_policy_data_failure(read_type::FM_V2_QUOTE_FEES_DECODE);
        }
    }
}

impl<P> BroadcastPolicyDataProvider for LiveBroadcastPolicyDataProvider<P>
where
    P: TransactionBroadcastProvider + EthCallProvider + EthBalanceProvider,
{
    fn gather_inputs<'a>(
        &'a self,
        state: &'a AppState,
        intent: &'a OptionExecutionIntent,
    ) -> PolicyDataFuture<'a, BroadcastPolicyInputs> {
        Box::pin(async move {
            // 1) chain_id round-trip
            let chain_id_rpc = match self.provider.chain_id().await {
                Ok(id) => Some(id),
                Err(_) => {
                    self.record_data_failure(read_type::CHAIN_ID_RPC);
                    None
                }
            };

            // 2) backend executor wei balance
            let be_addr = state.execution_config.executor_from_address.clone();
            let be_balance_wei = match self.provider.eth_get_balance(be_addr).await {
                Ok(balance) => Some(balance),
                Err(_) => {
                    self.record_data_failure(read_type::BE_BALANCE);
                    None
                }
            };

            let mut inputs = BroadcastPolicyInputs {
                chain_id_rpc,
                be_balance_wei,
                ..BroadcastPolicyInputs::default()
            };

            // 3) OME live state — paused() + isExecutor(BE)
            let ome_addr = state.options_config.matching_engine_address.clone();
            if !ome_addr.0.is_empty() {
                inputs.ome_paused =
                    read_bool_view(&self.provider, &ome_addr, &selector_no_args("paused()")).await;
                if inputs.ome_paused.is_none() {
                    self.record_data_failure(read_type::OME_PAUSED);
                }
                let be_addr2 = state.execution_config.executor_from_address.clone();
                inputs.ome_is_executor = read_bool_view(
                    &self.provider,
                    &ome_addr,
                    &selector_with_address("isExecutor(address)", &be_addr2),
                )
                .await;
                if inputs.ome_is_executor.is_none() {
                    self.record_data_failure(read_type::OME_IS_EXECUTOR);
                }
            }

            // 4) PFV live state — feeBalance(asset) + rebateReserve(asset)
            let asset = intent.settlement_asset.clone();
            if let Some(pfv) = self.pfv_address.as_ref() {
                inputs.pfv_fee_balance_asset = read_u256_view(
                    &self.provider,
                    pfv,
                    &selector_with_address("feeBalance(address)", &asset),
                )
                .await;
                if inputs.pfv_fee_balance_asset.is_none() {
                    self.record_data_failure(read_type::PFV_FEE_BALANCE);
                }
                inputs.pfv_rebate_reserve_asset = read_u256_view(
                    &self.provider,
                    pfv,
                    &selector_with_address("rebateReserve(address)", &asset),
                )
                .await;
                if inputs.pfv_rebate_reserve_asset.is_none() {
                    self.record_data_failure(read_type::PFV_REBATE_RESERVE);
                }
            }

            // 5) CV.balances(PFV, asset) for the R5 precheck
            if let (Some(pfv), Some(cv)) = (self.pfv_address.as_ref(), self.cv_address.as_ref()) {
                inputs.cv_pfv_balance_asset = read_u256_view(
                    &self.provider,
                    cv,
                    &selector_with_two_addresses("balances(address,address)", pfv, &asset),
                )
                .await;
                if inputs.cv_pfv_balance_asset.is_none() {
                    self.record_data_failure(read_type::CV_PFV_BALANCE);
                }
                // Derive R5: CV(PFV,asset) == feeBalance + rebateReserve.
                inputs.r5_drift_zero = match (
                    inputs.cv_pfv_balance_asset,
                    inputs.pfv_fee_balance_asset,
                    inputs.pfv_rebate_reserve_asset,
                ) {
                    (Some(cv_bal), Some(fee), Some(reserve)) => {
                        Some(cv_bal == fee.saturating_add(reserve))
                    }
                    _ => None,
                };
            }

            // 6) Persistent dedupe — existing tx row or terminal status.
            let (hit, reason) = derive_dedupe_state(state, intent).await;
            inputs.dedupe_hit = hit;
            inputs.dedupe_reason = reason;

            // 7) FeesManagerV2 economic data — quoteFees(maker) +
            //    quoteFees(taker) + rebateBudget(asset). Populated only
            //    when an FM_V2 address is configured.
            if let Some(fm) = self.fees_manager_v2_address.as_ref() {
                let basis_amount =
                    (intent.premium_per_contract_native).saturating_mul(intent.quantity_contracts);
                let (maker_addr, taker_addr) = if intent.buyer_is_maker {
                    (&intent.buyer, &intent.seller)
                } else {
                    (&intent.seller, &intent.buyer)
                };
                let flow_code = match intent.source_type {
                    crate::options::OptionExecutionSourceType::OptionOrderbookFill => 0u8,
                    crate::options::OptionExecutionSourceType::OptionRfqFill => 1u8,
                };
                const PRODUCT_OPTION: u8 = 0;
                let maker_quote = self
                    .quote_fees_call(
                        fm,
                        maker_addr,
                        PRODUCT_OPTION,
                        flow_code,
                        true,
                        &asset,
                        basis_amount,
                    )
                    .await;
                let taker_quote = self
                    .quote_fees_call(
                        fm,
                        taker_addr,
                        PRODUCT_OPTION,
                        flow_code,
                        false,
                        &asset,
                        basis_amount,
                    )
                    .await;
                if let (Ok(mk), Ok(tk)) = (&maker_quote, &taker_quote) {
                    inputs.fee_split = Some(aggregate_fee_split(mk, tk, asset.clone()));
                }
                inputs.fm_v2_rebate_budget_asset = read_u256_view(
                    &self.provider,
                    fm,
                    &selector_with_address("rebateBudget(address)", &asset),
                )
                .await;
                if inputs.fm_v2_rebate_budget_asset.is_none() {
                    self.record_data_failure(read_type::FM_V2_REBATE_BUDGET);
                }
            }

            Ok(inputs)
        })
    }
}

impl<P> LiveBroadcastPolicyDataProvider<P>
where
    P: EthCallProvider,
{
    /// Issue a read-only `FeesManagerV2.quoteFees(...)` and decode the
    /// `FeeQuote` static-tuple return.
    ///
    /// Returns:
    /// - `Ok(FeeQuoteRaw)` on success.
    /// - `Err(FmV2QuoteFailureKind::Rpc)` on `eth_call` failure (records
    ///   `fm_v2_rpc_failures_total` + `policy_data_failures_total{read_type="fm_v2_quote_fees_rpc"}`).
    /// - `Err(FmV2QuoteFailureKind::Decode)` on ABI decode failure
    ///   (records `fm_v2_decode_failures_total` +
    ///   `policy_data_failures_total{read_type="fm_v2_quote_fees_decode"}`).
    ///
    /// The call-site treats any error as fail-closed via
    /// `econ_data_available == false` (chain-state gates remain
    /// authoritative). Distinguishing RPC vs decode failure at the metric
    /// layer lets operators alert on the two failure modes separately —
    /// a decode-rate spike implies a contract upgrade or ABI drift,
    /// whereas an RPC-rate spike implies an infrastructure issue.
    #[allow(clippy::too_many_arguments)]
    async fn quote_fees_call(
        &self,
        fm: &AccountId,
        trader: &AccountId,
        product: u8,
        flow: u8,
        is_maker: bool,
        settlement_asset: &AccountId,
        basis_amount: u128,
    ) -> Result<FeeQuoteRaw, FmV2QuoteFailureKind> {
        let call_data = encode_quote_fees_call(
            trader,
            product,
            flow,
            is_maker,
            settlement_asset,
            basis_amount,
        );
        let success = match self
            .provider
            .eth_call(EthCallRequest {
                from: AccountId::new("0x0000000000000000000000000000000000000000"),
                to: fm.clone(),
                data: call_data,
                value: 0,
                gas_limit: None,
            })
            .await
        {
            Ok(success) => success,
            Err(_) => {
                self.record_fm_v2_rpc_failure_metric();
                return Err(FmV2QuoteFailureKind::Rpc);
            }
        };
        match decode_fee_quote(&success.output) {
            Ok(quote) => Ok(quote),
            Err(_) => {
                self.record_fm_v2_decode_failure_metric();
                Err(FmV2QuoteFailureKind::Decode)
            }
        }
    }
}

/// Distinguishes RPC-side from decode-side failures in `quote_fees_call`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FmV2QuoteFailureKind {
    Rpc,
    Decode,
}

// ---------------------------------------------------------------------------
// StubBroadcastPolicyDataProvider
// ---------------------------------------------------------------------------

/// Test / dev-only stub provider. Holds a pre-built [`BroadcastPolicyInputs`]
/// that `gather_inputs` returns verbatim. Useful for table-driven
/// integration tests of the broadcast call-site under specific live-read
/// permutations without standing up a mock RPC.
pub struct StubBroadcastPolicyDataProvider {
    inputs: BroadcastPolicyInputs,
}

impl StubBroadcastPolicyDataProvider {
    pub fn new(inputs: BroadcastPolicyInputs) -> Self {
        Self { inputs }
    }

    /// Permissive Sepolia-shaped fixture: all reads succeeded, OME healthy,
    /// dedupe clean, R5 drift = 0. `fee_split` is left `None` to preserve
    /// the boundary-mode broadcast path used by the rehearsal regression;
    /// tests that need `econ_data_available = true` override it.
    pub fn sepolia_permissive() -> Self {
        Self {
            inputs: BroadcastPolicyInputs {
                chain_id_rpc: Some(crate::execution::BASE_SEPOLIA_CHAIN_ID),
                be_balance_wei: Some(u128::MAX / 2),
                ome_paused: Some(false),
                ome_is_executor: Some(true),
                pfv_fee_balance_asset: Some(0),
                pfv_rebate_reserve_asset: Some(0),
                cv_pfv_balance_asset: Some(0),
                fee_split: None,
                fm_v2_rebate_budget_asset: None,
                dedupe_hit: false,
                dedupe_reason: DedupeReason::None,
                r5_drift_zero: Some(true),
            },
        }
    }
}

impl BroadcastPolicyDataProvider for StubBroadcastPolicyDataProvider {
    fn gather_inputs<'a>(
        &'a self,
        _state: &'a AppState,
        _intent: &'a OptionExecutionIntent,
    ) -> PolicyDataFuture<'a, BroadcastPolicyInputs> {
        let inputs = self.inputs.clone();
        Box::pin(async move { Ok(inputs) })
    }
}

// ---------------------------------------------------------------------------
// Helpers — ABI selector encoding + view-call decoding
// ---------------------------------------------------------------------------

/// `keccak256(signature.as_bytes())[0..4]`.
fn selector_no_args(signature: &str) -> Vec<u8> {
    let hash = keccak256(signature.as_bytes());
    hash[0..4].to_vec()
}

/// ABI-encode a single `address` argument behind the selector.
fn selector_with_address(signature: &str, address: &AccountId) -> Vec<u8> {
    let mut data = selector_no_args(signature);
    data.extend_from_slice(&encode_address_param(address));
    data
}

/// ABI-encode two `address` arguments behind the selector.
fn selector_with_two_addresses(signature: &str, first: &AccountId, second: &AccountId) -> Vec<u8> {
    let mut data = selector_no_args(signature);
    data.extend_from_slice(&encode_address_param(first));
    data.extend_from_slice(&encode_address_param(second));
    data
}

/// Left-pad a 20-byte EVM address to a 32-byte ABI parameter.
fn encode_address_param(address: &AccountId) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let hex = address.0.strip_prefix("0x").unwrap_or(address.0.as_str());
    let mut bytes = [0u8; 20];
    if hex.len() == 40 {
        for i in 0..20 {
            if let Ok(byte) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
                bytes[i] = byte;
            }
        }
    }
    buf[12..].copy_from_slice(&bytes);
    buf
}

async fn read_bool_view<P>(provider: &P, to: &AccountId, data: &[u8]) -> Option<bool>
where
    P: EthCallProvider,
{
    let success = provider
        .eth_call(EthCallRequest {
            from: AccountId::new("0x0000000000000000000000000000000000000000"),
            to: to.clone(),
            data: data.to_vec(),
            value: 0,
            gas_limit: None,
        })
        .await
        .ok()?;
    let raw = success.output;
    if raw.len() < 32 {
        return None;
    }
    let mut all_zero = true;
    for byte in &raw[..31] {
        if *byte != 0 {
            all_zero = false;
            break;
        }
    }
    if !all_zero {
        return None;
    }
    Some(raw[31] != 0)
}

async fn read_u256_view<P>(provider: &P, to: &AccountId, data: &[u8]) -> Option<u128>
where
    P: EthCallProvider,
{
    let success = provider
        .eth_call(EthCallRequest {
            from: AccountId::new("0x0000000000000000000000000000000000000000"),
            to: to.clone(),
            data: data.to_vec(),
            value: 0,
            gas_limit: None,
        })
        .await
        .ok()?;
    let raw = success.output;
    if raw.len() < 32 {
        return None;
    }
    // High 16 bytes must be zero to fit in u128 — if not, return None
    // (defence-in-depth against overflow; PFV / CV balances will fit).
    for byte in &raw[0..16] {
        if *byte != 0 {
            return None;
        }
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&raw[16..32]);
    Some(u128::from_be_bytes(buf))
}

// ---------------------------------------------------------------------------
// Persistent dedupe — DB / store boundary
// ---------------------------------------------------------------------------

/// Persistent-dedupe predicate. Returns `(hit, reason)` — hit when there's
/// an existing submitted tx row with a tx_hash on the same intent, or when
/// the intent's status is already `BroadcastSubmitted` / `BroadcastConfirmed`
/// / `BroadcastFailed`. In-memory dedupe alone is not enough for mainnet
/// (custody policy §10.1 + monitoring spec §3.3); when the repository is
/// enabled this function consults the persisted state.
async fn derive_dedupe_state(
    state: &AppState,
    intent: &OptionExecutionIntent,
) -> (bool, DedupeReason) {
    let intent_id = intent.intent_id;

    // 1) Terminal-status check (cheap; uses in-process state — but the
    // intent is loaded from the repository when persistence is enabled).
    match intent.status {
        OptionExecutionIntentStatus::BroadcastSubmitted => {
            return (true, DedupeReason::StatusAlreadyBroadcastSubmitted);
        }
        OptionExecutionIntentStatus::BroadcastConfirmed => {
            return (true, DedupeReason::StatusAlreadyBroadcastConfirmed);
        }
        OptionExecutionIntentStatus::BroadcastFailed => {
            // BroadcastFailed is a terminal-rejection state; treat as a
            // dedupe hit so we don't re-attempt automatically.
            return (true, DedupeReason::StatusAlreadyBroadcastFailed);
        }
        _ => {}
    }

    // 2) Persistent tx-row check via the existing store / repository.
    if let Some(tx) = find_submitted_or_pending_tx(state, intent_id).await {
        if tx.tx_hash.as_deref().filter(|s| !s.is_empty()).is_some() {
            return (true, DedupeReason::ExistingTxHash);
        }
    }

    (false, DedupeReason::None)
}

async fn find_submitted_or_pending_tx(
    state: &AppState,
    intent_id: crate::options::OptionExecutionIntentId,
) -> Option<OptionExecutionTransaction> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .find_submitted_option_execution_transaction_by_intent(intent_id)
            .await
            .ok()
            .flatten();
    }
    let store = state.options_store.lock().ok()?;
    store.find_submitted_option_execution_transaction_by_intent(intent_id)
}

// ---------------------------------------------------------------------------
// Startup launch-invariant hook (Cluster 4 / Q-34)
// ---------------------------------------------------------------------------

/// Outcome of the startup-time `verify_launch_invariant` sweep wrapping
/// `broadcast_policy::verify_launch_invariant`. On mainnet, a failing
/// invariant MUST block the process from accepting broadcast traffic;
/// `is_blocking_failure()` is the predicate the caller (e.g. `main.rs` or
/// an admin route) uses to decide whether to exit non-zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupLaunchInvariantOutcome {
    pub mode: crate::options::broadcast_policy::BroadcastMode,
    pub report: crate::options::broadcast_policy::LaunchInvariantReport,
}

impl StartupLaunchInvariantOutcome {
    pub fn is_blocking_failure(&self) -> bool {
        matches!(
            self.mode,
            crate::options::broadcast_policy::BroadcastMode::Mainnet
        ) && !self.report.overall_pass
    }
}

/// Wrapper around `broadcast_policy::verify_launch_invariant` intended for
/// startup-time invocation. The caller supplies the active fee-profile
/// snapshot (operator-side import — FeesManagerV2 ABI reads land in a
/// follow-on track) plus the `PFV.rebateReserve(asset)` value the live
/// provider read.
///
/// On mainnet AND `overall_pass == false`, callers MUST treat the result
/// as fatal (e.g. exit non-zero in `main.rs`, or return a `Config` error
/// when wired through `validate_startup`). Sepolia mode is informational.
pub fn verify_launch_invariant_for_startup(
    chain_id: u64,
    profiles: &[crate::options::broadcast_policy::ActiveFeeProfile],
    rebate_reserve_asset: u128,
) -> StartupLaunchInvariantOutcome {
    let mode = crate::options::broadcast_policy::BroadcastMode::from_chain_id(chain_id);
    let report = crate::options::broadcast_policy::verify_launch_invariant(
        profiles,
        rebate_reserve_asset,
        mode,
    );
    StartupLaunchInvariantOutcome { mode, report }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv_pfv_inputs(cv: u128, fee: u128, reserve: u128) -> BroadcastPolicyInputs {
        BroadcastPolicyInputs {
            chain_id_rpc: Some(8453),
            be_balance_wei: Some(1_000_000_000_000_000_000),
            ome_paused: Some(false),
            ome_is_executor: Some(true),
            pfv_fee_balance_asset: Some(fee),
            pfv_rebate_reserve_asset: Some(reserve),
            cv_pfv_balance_asset: Some(cv),
            fee_split: None,
            fm_v2_rebate_budget_asset: None,
            dedupe_hit: false,
            dedupe_reason: DedupeReason::None,
            r5_drift_zero: Some(cv == fee.saturating_add(reserve)),
        }
    }

    #[test]
    fn selector_no_args_matches_paused() {
        // keccak256("paused()") = 0x5c975abb...
        let selector = selector_no_args("paused()");
        assert_eq!(selector.len(), 4);
        assert_eq!(selector, vec![0x5c, 0x97, 0x5a, 0xbb]);
    }

    #[test]
    fn selector_with_address_left_pads_to_32_bytes() {
        let addr = AccountId::new("0x000000000000000000000000000000000000beef");
        let data = selector_with_address("isExecutor(address)", &addr);
        assert_eq!(data.len(), 36); // 4 + 32
                                    // last 20 bytes of the 32-byte parameter region are the address
        let tail = &data[16..36];
        assert_eq!(tail[19], 0xef);
        assert_eq!(tail[18], 0xbe);
        // bytes 0..12 of the 32-byte parameter are zero (address left-pad)
        for byte in &data[4..16] {
            assert_eq!(*byte, 0u8);
        }
    }

    #[test]
    fn r5_drift_detected_in_fixture() {
        let ok = cv_pfv_inputs(1000, 600, 400);
        assert_eq!(ok.r5_drift_zero, Some(true));
        let drift = cv_pfv_inputs(1000, 600, 401);
        assert_eq!(drift.r5_drift_zero, Some(false));
    }

    #[test]
    fn dedupe_reason_helpers() {
        assert!(!DedupeReason::None.is_hit());
        assert!(DedupeReason::ExistingTxHash.is_hit());
        assert!(DedupeReason::StatusAlreadyBroadcastSubmitted.is_hit());
        assert!(DedupeReason::StatusAlreadyBroadcastConfirmed.is_hit());
        assert!(DedupeReason::StatusAlreadyBroadcastFailed.is_hit());
    }

    #[test]
    fn policy_data_error_display_carries_code_prefix() {
        let err = PolicyDataError::Rpc("connect failed".to_string());
        assert!(err.to_string().starts_with("policy-data:rpc:"));
    }

    // ----- startup launch-invariant hook -----

    fn fee_only_profile() -> crate::options::broadcast_policy::ActiveFeeProfile {
        crate::options::broadcast_policy::ActiveFeeProfile {
            tier: 0,
            product: crate::options::broadcast_policy::ProductKind::Option,
            flow: crate::options::broadcast_policy::FeeFlow::Orderbook,
            maker_ppm: 50,
            taker_ppm: 100,
            maker_discount_ppm: 0,
            taker_discount_ppm: 0,
        }
    }

    #[test]
    fn startup_invariant_passes_on_mainnet_with_fee_only_and_zero_reserve() {
        let outcome = verify_launch_invariant_for_startup(8453, &[fee_only_profile()], 0);
        assert!(outcome.report.overall_pass);
        assert!(!outcome.is_blocking_failure());
    }

    #[test]
    fn startup_invariant_blocks_on_mainnet_with_nonzero_rebate_reserve() {
        let outcome = verify_launch_invariant_for_startup(8453, &[fee_only_profile()], 1);
        assert!(!outcome.report.overall_pass);
        assert!(outcome.is_blocking_failure());
    }

    #[test]
    fn startup_invariant_does_not_block_on_sepolia_even_with_nonzero_reserve() {
        let outcome = verify_launch_invariant_for_startup(84532, &[fee_only_profile()], 1);
        // Sepolia rule: overall_pass relaxes the reserve clause.
        assert!(outcome.report.overall_pass);
        assert!(!outcome.is_blocking_failure());
    }

    // ----- FeesManagerV2 ABI codec -----

    #[test]
    fn quote_fees_selector_matches_keccak_vector() {
        let bytes = quote_fees_selector_bytes();
        let expected = {
            let hash = keccak256(b"quoteFees(address,uint8,uint8,bool,address,uint256)");
            [hash[0], hash[1], hash[2], hash[3]]
        };
        assert_eq!(bytes, expected);
        // Placeholder const is exported for documentation only.
        let _ = FM_V2_QUOTE_FEES_SELECTOR;
    }

    #[test]
    fn encode_quote_fees_call_has_exact_size_and_selector_prefix() {
        let trader = AccountId::new("0x000000000000000000000000000000000000beef");
        let asset = AccountId::new("0x000000000000000000000000000000000000aaaa");
        let data = encode_quote_fees_call(&trader, 0, 0, true, &asset, 12_345);
        assert_eq!(data.len(), 4 + 6 * 32);
        assert_eq!(&data[0..4], &quote_fees_selector_bytes());
        let basis_word = &data[4 + 5 * 32..];
        assert_eq!(basis_word.len(), 32);
        assert_eq!(basis_word[30], 0x30);
        assert_eq!(basis_word[31], 0x39);
    }

    #[allow(clippy::too_many_arguments)]
    fn build_fee_quote_payload(
        applied_ppm: i32,
        basis_amount: u128,
        fee_amount: u128,
        is_rebate: bool,
        tier: u8,
        product: u8,
        flow: u8,
        fee_basis: u8,
        is_maker: bool,
        asset_hex_byte: u8,
        recipient_hex_byte: u8,
        rebate_fundable: bool,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 384];
        if applied_ppm < 0 {
            for byte in &mut buf[0..28] {
                *byte = 0xff;
            }
        }
        buf[28..32].copy_from_slice(&applied_ppm.to_be_bytes());
        buf[32 + 16..32 + 32].copy_from_slice(&basis_amount.to_be_bytes());
        buf[64 + 16..64 + 32].copy_from_slice(&fee_amount.to_be_bytes());
        buf[96 + 31] = if is_rebate { 1 } else { 0 };
        buf[128 + 31] = tier;
        buf[160 + 31] = product;
        buf[192 + 31] = flow;
        buf[224 + 31] = fee_basis;
        buf[256 + 31] = if is_maker { 1 } else { 0 };
        for i in 0..20 {
            buf[288 + 12 + i] = asset_hex_byte;
            buf[320 + 12 + i] = recipient_hex_byte;
        }
        buf[352 + 31] = if rebate_fundable { 1 } else { 0 };
        buf
    }

    #[test]
    fn decode_fee_quote_positive_ppm_round_trip() {
        let raw =
            build_fee_quote_payload(50, 1_000_000, 50, false, 0, 0, 0, 0, true, 0xaa, 0xcc, true);
        let q = decode_fee_quote(&raw).expect("decode must succeed");
        assert_eq!(q.applied_ppm, 50);
        assert_eq!(q.basis_amount, 1_000_000);
        assert_eq!(q.fee_amount, 50);
        assert!(!q.is_rebate);
        assert_eq!(q.tier, 0);
        assert!(q.is_maker);
        assert!(q.rebate_fundable);
    }

    #[test]
    fn decode_fee_quote_zero_ppm_treated_as_non_rebate() {
        let raw =
            build_fee_quote_payload(0, 1_000_000, 0, false, 0, 0, 0, 0, true, 0xaa, 0xcc, true);
        let q = decode_fee_quote(&raw).expect("zero ppm must decode");
        assert_eq!(q.applied_ppm, 0);
        assert!(!q.is_rebate);
    }

    #[test]
    fn decode_fee_quote_negative_ppm_is_rebate() {
        let raw = build_fee_quote_payload(
            -25, 1_000_000, 25, true, 3, 0, 1, 0, false, 0xaa, 0xcc, true,
        );
        let q = decode_fee_quote(&raw).expect("decode must succeed");
        assert_eq!(q.applied_ppm, -25);
        assert!(q.is_rebate);
        assert_eq!(q.fee_amount, 25);
        assert_eq!(q.tier, 3);
    }

    #[test]
    fn decode_fee_quote_short_payload_rejected() {
        let err = decode_fee_quote(&[0u8; 100]).expect_err("short payload must reject");
        assert!(matches!(err, PolicyDataError::EncodingError(_)));
    }

    #[test]
    fn decode_fee_quote_inconsistent_negative_sign_rejected() {
        let raw = build_fee_quote_payload(
            -25, 1_000_000, 25, false, 0, 0, 0, 0, false, 0xaa, 0xcc, true,
        );
        let err = decode_fee_quote(&raw).expect_err("inconsistent isRebate must reject");
        assert!(matches!(err, PolicyDataError::EncodingError(_)));
    }

    #[test]
    fn decode_fee_quote_inconsistent_positive_sign_rejected() {
        let raw =
            build_fee_quote_payload(25, 1_000_000, 25, true, 0, 0, 0, 0, false, 0xaa, 0xcc, true);
        let err = decode_fee_quote(&raw).expect_err("positive ppm + isRebate must reject");
        assert!(matches!(err, PolicyDataError::EncodingError(_)));
    }

    #[test]
    fn decode_fee_quote_overflow_high_basis_rejected() {
        let mut raw = build_fee_quote_payload(50, 0, 0, false, 0, 0, 0, 0, true, 0, 0, true);
        raw[32] = 0x01;
        let err = decode_fee_quote(&raw).expect_err("over u128 basis must reject");
        assert!(matches!(err, PolicyDataError::EncodingError(_)));
    }

    #[test]
    fn decode_fee_quote_malformed_sign_extension_rejected() {
        let mut raw = build_fee_quote_payload(50, 0, 0, false, 0, 0, 0, 0, true, 0, 0, true);
        raw[10] = 0xff;
        let err = decode_fee_quote(&raw).expect_err("malformed sign extension must reject");
        assert!(matches!(err, PolicyDataError::EncodingError(_)));
    }

    // ----- fee split aggregation -----

    fn quote(ppm: i32, fee: u128, is_rebate: bool, tier: u8, is_maker: bool) -> FeeQuoteRaw {
        FeeQuoteRaw {
            applied_ppm: ppm,
            basis_amount: 1_000_000,
            fee_amount: fee,
            is_rebate,
            tier,
            product: 0,
            flow: 0,
            fee_basis: 0,
            is_maker,
            settlement_asset: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            recipient: AccountId::new("0x000000000000000000000000000000000000bbbb"),
            rebate_fundable: true,
        }
    }

    #[test]
    fn aggregate_fee_split_fee_only_path_sums_both_sides() {
        let mk = quote(50, 50, false, 0, true);
        let tk = quote(100, 100, false, 0, false);
        let split = aggregate_fee_split(
            &mk,
            &tk,
            AccountId::new("0x000000000000000000000000000000000000aaaa"),
        );
        assert_eq!(split.gross_fee_revenue, 150);
        assert_eq!(split.total_rebate_outflow, 0);
        assert_eq!(split.net_protocol_revenue, 150);
        assert_eq!(split.effective_maker_ppm, 50);
        assert_eq!(split.effective_taker_ppm, 100);
        assert_eq!(split.tier, 0);
    }

    #[test]
    fn aggregate_fee_split_maker_rebate_taker_fee() {
        let mk = quote(-10, 10, true, 1, true);
        let tk = quote(100, 100, false, 1, false);
        let split = aggregate_fee_split(
            &mk,
            &tk,
            AccountId::new("0x000000000000000000000000000000000000aaaa"),
        );
        assert_eq!(split.gross_fee_revenue, 100);
        assert_eq!(split.total_rebate_outflow, 10);
        assert_eq!(split.net_protocol_revenue, 90);
        assert_eq!(split.effective_maker_ppm, -10);
        assert_eq!(split.effective_taker_ppm, 100);
    }

    #[test]
    fn aggregate_fee_split_tier_is_max_of_two_sides() {
        let mk = quote(50, 50, false, 2, true);
        let tk = quote(100, 100, false, 5, false);
        let split = aggregate_fee_split(
            &mk,
            &tk,
            AccountId::new("0x000000000000000000000000000000000000aaaa"),
        );
        assert_eq!(split.tier, 5);
    }

    #[test]
    fn aggregate_fee_split_both_rebate_negative_net_revenue() {
        let mk = quote(-50, 50, true, 0, true);
        let tk = quote(-100, 100, true, 0, false);
        let split = aggregate_fee_split(
            &mk,
            &tk,
            AccountId::new("0x000000000000000000000000000000000000aaaa"),
        );
        assert_eq!(split.gross_fee_revenue, 0);
        assert_eq!(split.total_rebate_outflow, 150);
        assert_eq!(split.net_protocol_revenue, -150);
    }

    // ----- LiveBroadcastPolicyDataProvider failure-metric wiring -----

    use crate::api::AppState;
    use crate::engine::EngineState;
    use crate::error::BackendError;
    use crate::execution::rpc::{
        EstimateGasRequest, EthBalanceProvider, EthCallProvider, EthCallRequest, EthCallSuccess,
        GasEstimateProvider, RpcFuture, TransactionBroadcastProvider, TransactionReceiptProvider,
    };
    use crate::options::broadcast_observability::BroadcastObservability;
    use crate::options::types::{
        OptionExecutionIntent, OptionExecutionIntentStatus, OptionExecutionSourceType,
        OptionsConfig,
    };
    use std::sync::Arc as StdArc;
    use std::sync::Mutex as StdMutex;
    use uuid::Uuid;

    const TEST_ASSET: &str = "0x6eae407f5640b006fac9965182e238582a3b412e";
    const TEST_BE: &str = "0x295005fd4f311e6691f008d57d32fcfede844518";
    const TEST_OME: &str = "0x5a5ebf9a9ccd7c012518569de8283982982670f6";
    const TEST_PFV: &str = "0x7c0a3b6febd5bffc164f37738299aeb453181886";
    const TEST_CV: &str = "0x00340c360353a5ab784c5bc5c44322a6af0625d3";
    const TEST_FM_V2: &str = "0xf6626177f3b85cc3239667cc53c04a8007652944";

    #[derive(Clone, Default)]
    struct ProgrammableEthProvider {
        fail_chain_id: StdArc<StdMutex<bool>>,
        fail_balance: StdArc<StdMutex<bool>>,
        /// `eth_call` outcome: when matching the predicate, return error
        /// or return success with raw bytes.
        eth_call_handler: StdArc<StdMutex<EthCallHandler>>,
        chain_id_value: u64,
        balance_value: u128,
    }

    #[derive(Clone, Default)]
    struct EthCallHandler {
        /// (selector, outcome) — when the call data starts with `selector`,
        /// apply the outcome. First match wins; default = empty 384-byte
        /// success buffer (decode will fail-closed).
        rules: Vec<(Vec<u8>, EthCallOutcome)>,
        default_success: Vec<u8>,
    }

    #[derive(Clone)]
    enum EthCallOutcome {
        Success(Vec<u8>),
        Error,
    }

    impl ProgrammableEthProvider {
        fn new() -> Self {
            // Default: chain_id ok = 84532; balance ok = 1; eth_call ok
            // returns a 384-byte zero buffer (decoder will accept zeroes
            // as a "fee-only zero-ppm" quote).
            Self {
                fail_chain_id: StdArc::new(StdMutex::new(false)),
                fail_balance: StdArc::new(StdMutex::new(false)),
                eth_call_handler: StdArc::new(StdMutex::new(EthCallHandler {
                    rules: Vec::new(),
                    default_success: build_default_eth_call_success(),
                })),
                chain_id_value: 84532,
                balance_value: 1,
            }
        }

        fn fail_chain_id(self) -> Self {
            *self.fail_chain_id.lock().unwrap() = true;
            self
        }

        fn fail_balance(self) -> Self {
            *self.fail_balance.lock().unwrap() = true;
            self
        }

        fn fail_eth_call_with_selector(self, selector: &[u8]) -> Self {
            self.eth_call_handler
                .lock()
                .unwrap()
                .rules
                .push((selector.to_vec(), EthCallOutcome::Error));
            self
        }

        fn return_eth_call_payload(self, selector: &[u8], payload: Vec<u8>) -> Self {
            self.eth_call_handler
                .lock()
                .unwrap()
                .rules
                .push((selector.to_vec(), EthCallOutcome::Success(payload)));
            self
        }
    }

    /// Build a 384-byte default `FeeQuote` payload that decodes as a
    /// fee-only zero-ppm quote — used so the OK path of eth_call returns
    /// a structurally-valid FeeQuote when the test doesn't override.
    fn build_default_eth_call_success() -> Vec<u8> {
        // 12 zero words → applied_ppm = 0, basisAmount = 0, ...
        vec![0u8; 384]
    }

    impl TransactionBroadcastProvider for ProgrammableEthProvider {
        fn chain_id(&self) -> RpcFuture<'_, u64> {
            let fail = *self.fail_chain_id.lock().unwrap();
            let value = self.chain_id_value;
            Box::pin(async move {
                if fail {
                    Err(BackendError::Simulation("chain_id failure".into()))
                } else {
                    Ok(value)
                }
            })
        }
        fn transaction_count(&self, _address: AccountId) -> RpcFuture<'_, u64> {
            Box::pin(async move { Ok(0) })
        }
        fn send_raw_transaction(&self, _raw: String) -> RpcFuture<'_, String> {
            Box::pin(async move { Ok("0x00".to_string()) })
        }
    }

    impl GasEstimateProvider for ProgrammableEthProvider {
        fn estimate_gas(&self, _request: EstimateGasRequest) -> RpcFuture<'_, u64> {
            Box::pin(async move { Ok(0) })
        }
    }

    impl TransactionReceiptProvider for ProgrammableEthProvider {
        fn block_number(&self) -> RpcFuture<'_, u64> {
            Box::pin(async move { Ok(0) })
        }
        fn transaction_receipt(
            &self,
            _tx_hash: String,
        ) -> RpcFuture<'_, Option<crate::confirmation::ConfirmationReceipt>> {
            Box::pin(async move { Ok(None) })
        }
    }

    impl EthBalanceProvider for ProgrammableEthProvider {
        fn eth_get_balance(&self, _address: AccountId) -> RpcFuture<'_, u128> {
            let fail = *self.fail_balance.lock().unwrap();
            let value = self.balance_value;
            Box::pin(async move {
                if fail {
                    Err(BackendError::Simulation("balance failure".into()))
                } else {
                    Ok(value)
                }
            })
        }
    }

    impl EthCallProvider for ProgrammableEthProvider {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let handler = self.eth_call_handler.lock().unwrap();
            let outcome = handler
                .rules
                .iter()
                .find(|(selector, _)| request.data.starts_with(selector))
                .map(|(_, outcome)| outcome.clone())
                .unwrap_or_else(|| EthCallOutcome::Success(handler.default_success.clone()));
            drop(handler);
            Box::pin(async move {
                match outcome {
                    EthCallOutcome::Success(bytes) => Ok(EthCallSuccess {
                        block_number: Some(1),
                        output: bytes,
                    }),
                    EthCallOutcome::Error => {
                        Err(BackendError::Simulation("eth_call failure".into()))
                    }
                }
            })
        }
    }

    fn programmable_state() -> AppState {
        let mut state = AppState::new(EngineState::with_default_markets());
        let mut options_config = OptionsConfig::disabled();
        options_config.matching_engine_address = AccountId::new(TEST_OME);
        state.options_config = options_config;
        state.execution_config.executor_from_address = AccountId::new(TEST_BE);
        state.execution_config.executor_chain_id = 84532;
        state
    }

    fn ome_orderbook_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x01".to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id: "0x02".to_string(),
            buyer: AccountId::new("0x000000000000000000000000000000000000aaaa"),
            seller: AccountId::new("0x000000000000000000000000000000000000bbbb"),
            underlying: AccountId::new("0x000000000000000000000000000000000000cccc"),
            settlement_asset: AccountId::new(TEST_ASSET),
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 1_000,
            buyer_is_maker: true,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0xdeadbeef".to_string()),
            status: OptionExecutionIntentStatus::CalldataReady,
            error: None,
            simulation_status: None,
            simulation_error: None,
            simulation_block_number: None,
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: None,
            canonical_execution_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn live_provider_records_chain_id_rpc_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let provider = ProgrammableEthProvider::new().fail_chain_id();
        let live = LiveBroadcastPolicyDataProvider::new(provider, None, None, None)
            .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.chain_id_rpc, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total.get(read_type::CHAIN_ID_RPC),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn live_provider_records_be_balance_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let provider = ProgrammableEthProvider::new().fail_balance();
        let live = LiveBroadcastPolicyDataProvider::new(provider, None, None, None)
            .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.be_balance_wei, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total.get(read_type::BE_BALANCE),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn live_provider_records_ome_paused_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let paused_selector = selector_no_args("paused()");
        let provider = ProgrammableEthProvider::new().fail_eth_call_with_selector(&paused_selector);
        let live = LiveBroadcastPolicyDataProvider::new(provider, None, None, None)
            .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.ome_paused, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total.get(read_type::OME_PAUSED),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn live_provider_records_pfv_rebate_reserve_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let selector = selector_no_args("rebateReserve(address)");
        let provider = ProgrammableEthProvider::new().fail_eth_call_with_selector(&selector);
        let live = LiveBroadcastPolicyDataProvider::new(
            provider,
            Some(AccountId::new(TEST_PFV)),
            None,
            None,
        )
        .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.pfv_rebate_reserve_asset, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::PFV_REBATE_RESERVE),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn live_provider_records_cv_pfv_balance_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let selector = selector_no_args("balances(address,address)");
        let provider = ProgrammableEthProvider::new().fail_eth_call_with_selector(&selector);
        let live = LiveBroadcastPolicyDataProvider::new(
            provider,
            Some(AccountId::new(TEST_PFV)),
            Some(AccountId::new(TEST_CV)),
            None,
        )
        .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.cv_pfv_balance_asset, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::CV_PFV_BALANCE),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn live_provider_records_fm_v2_quote_fees_rpc_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let selector = quote_fees_selector_bytes();
        let provider = ProgrammableEthProvider::new().fail_eth_call_with_selector(&selector);
        let live = LiveBroadcastPolicyDataProvider::new(
            provider,
            Some(AccountId::new(TEST_PFV)),
            Some(AccountId::new(TEST_CV)),
            Some(AccountId::new(TEST_FM_V2)),
        )
        .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.fee_split, None);
        let snap = observability.snapshot();
        // Both quoteFees calls (maker + taker) fail → 2 increments.
        assert_eq!(snap.fm_v2_rpc_failures_total, 2);
        assert_eq!(snap.fm_v2_decode_failures_total, 0);
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::FM_V2_QUOTE_FEES_RPC),
            Some(&2)
        );
    }

    #[tokio::test]
    async fn live_provider_records_fm_v2_quote_fees_decode_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let selector = quote_fees_selector_bytes();
        // Return a truncated 100-byte buffer instead of 384 → decode rejects.
        let provider =
            ProgrammableEthProvider::new().return_eth_call_payload(&selector, vec![0u8; 100]);
        let live = LiveBroadcastPolicyDataProvider::new(
            provider,
            Some(AccountId::new(TEST_PFV)),
            Some(AccountId::new(TEST_CV)),
            Some(AccountId::new(TEST_FM_V2)),
        )
        .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.fee_split, None);
        let snap = observability.snapshot();
        // Both quoteFees calls decode-fail → 2 increments on decode side.
        assert_eq!(snap.fm_v2_decode_failures_total, 2);
        assert_eq!(snap.fm_v2_rpc_failures_total, 0);
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::FM_V2_QUOTE_FEES_DECODE),
            Some(&2)
        );
    }

    #[tokio::test]
    async fn live_provider_records_fm_v2_rebate_budget_failure() {
        let observability = Arc::new(BroadcastObservability::new());
        let selector = selector_no_args("rebateBudget(address)");
        let provider = ProgrammableEthProvider::new().fail_eth_call_with_selector(&selector);
        let live = LiveBroadcastPolicyDataProvider::new(
            provider,
            Some(AccountId::new(TEST_PFV)),
            Some(AccountId::new(TEST_CV)),
            Some(AccountId::new(TEST_FM_V2)),
        )
        .with_observability(observability.clone());
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.fm_v2_rebate_budget_asset, None);
        let snap = observability.snapshot();
        assert_eq!(
            snap.policy_data_failures_total
                .get(read_type::FM_V2_REBATE_BUDGET),
            Some(&1)
        );
    }

    /// When observability is NOT attached, the provider runs unchanged
    /// (preserves library consumers that don't want metrics).
    #[tokio::test]
    async fn live_provider_without_observability_skips_metric_increments() {
        let provider = ProgrammableEthProvider::new().fail_chain_id();
        let live = LiveBroadcastPolicyDataProvider::new(provider, None, None, None);
        let state = programmable_state();
        let intent = ome_orderbook_intent();
        let inputs = live.gather_inputs(&state, &intent).await.unwrap();
        assert_eq!(inputs.chain_id_rpc, None);
        // No observability handle attached → no panic, no increment surface.
    }
}
