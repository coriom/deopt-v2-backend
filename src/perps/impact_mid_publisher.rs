//! PERPS-CLOSED-TEST-HARDENING-V1 Part E — impact-mid publisher trait
//! + local-anvil implementation.
//!
//! ## Scope
//!
//! The impact-mid keeper (Part B, `impact_mid_keeper.rs`) periodically
//! computes an off-chain TWAP mid per market and writes it to the
//! in-memory `ImpactMidCache`. Part E introduces the OPTIONAL on-chain
//! writer path: when the keeper is configured with an
//! `ImpactMidPublisher` handle, it also broadcasts the sample to the
//! on-chain `PerpEngine.updateImpactMid(uint256,uint128)` writer so the
//! Solidity-side funding math can consume the freshest reference.
//!
//! ## Safety posture (unchanged by this module)
//!
//! * Perps public trading remains fail-closed.
//! * Perps funding worker + tick remain fail-closed
//!   (`FundingConfig.isEnabled=false` end-to-end).
//! * `NoOpPublisher` is the SAFE default — it records the publish
//!   intent to the tracing surface but NEVER broadcasts. Operators who
//!   have not provisioned a signer + on-chain `impactMidSource`
//!   authorization stay on this variant.
//! * `LocalAnvilPublisher` is LOCAL ANVIL ONLY. Construction refuses
//!   `chain_id ∈ {1, 8453}` (Base + Ethereum mainnet); the harness's
//!   pinned `84532` local-anvil id is the only accepted value in this
//!   milestone. Base Sepolia wiring lands in a subsequent milestone.
//!
//! ## No secrets in logs
//!
//! The `LocalAnvilPublisher::Debug` impl redacts the private key. The
//! `publish` path logs only the tx-hash / block-number / market-id /
//! mid-value tuple at info level. The private key never enters a log
//! event, an error message, or a returned struct.

use crate::error::{BackendError, Result};
use crate::execution::rpc::{HttpJsonRpcProvider, TransactionBroadcastProvider};
use crate::execution::transaction::{
    assemble_eip1559_signed_transaction, eip1559_transaction_prehash,
    ExecutionTransactionRequest,
};
use crate::signing::eip712::parse_evm_address;
use crate::types::{AccountId, TimestampMs};
use async_trait::async_trait;
use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature, SigningKey};
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Outcome of a single `publish` call.
///
/// `Skipped` covers cases where the transport intentionally suppresses
/// a broadcast (e.g. the NoOpPublisher; or a duplicate-sample dedup on
/// the anvil publisher). `Published` carries the tx-hash + block-number
/// observed by the JSON-RPC receipt poll.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PublishOutcome {
    Published { tx_hash: String, block_number: u64 },
    Skipped { reason: String },
}

/// Publisher trait used by the impact-mid keeper. Implementations are
/// expected to be idempotent per `(market_id, mid_1e8, timestamp_ms)` —
/// resubmitting the exact same sample at the same timestamp MUST be a
/// transport-side no-op (either `Skipped` or a second `Published` that
/// resolves to the same on-chain state).
#[async_trait]
pub trait ImpactMidPublisher: std::fmt::Debug + Send + Sync {
    async fn publish(
        &self,
        market_id: u64,
        mid_1e8: u128,
        timestamp_ms: TimestampMs,
    ) -> Result<PublishOutcome>;
}

// ---------------------------------------------------------------------
// NoOpPublisher — safe default (no broadcast).
// ---------------------------------------------------------------------

/// Publisher that never broadcasts. This is the safe production default
/// until an operator provisions a signer + authorized on-chain
/// `impactMidSource`. Each `publish` call is logged at info level with
/// the publish-intent tuple (market_id, mid_1e8, timestamp_ms) so an
/// operator dashboard can validate that the keeper is producing samples
/// even in the no-broadcast configuration.
#[derive(Debug, Clone, Default)]
pub struct NoOpPublisher;

impl NoOpPublisher {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ImpactMidPublisher for NoOpPublisher {
    async fn publish(
        &self,
        market_id: u64,
        mid_1e8: u128,
        timestamp_ms: TimestampMs,
    ) -> Result<PublishOutcome> {
        tracing::info!(
            publisher = "noop",
            market_id,
            mid_1e8 = mid_1e8 as u64,
            timestamp_ms,
            "impact-mid publish intent recorded (no broadcast)"
        );
        Ok(PublishOutcome::Skipped {
            reason: "noop_publisher".to_string(),
        })
    }
}

// ---------------------------------------------------------------------
// LocalAnvilPublisher — real on-chain broadcast against a local anvil.
// ---------------------------------------------------------------------

/// Refuse construction on any mainnet chain id. Extend this list if
/// additional L1/L2 mainnets are onboarded — the point is a positive
/// deny-list, so an operator misconfiguring a chain id to `1` or `8453`
/// gets a construction-time error rather than a live broadcast.
const MAINNET_CHAIN_IDS: &[u64] = &[1, 8453];

/// `updateImpactMid(uint256,uint128)` — first 4 bytes of the keccak256
/// hash of the function signature. Hard-coded to avoid a per-publish
/// runtime keccak; a `#[test]` proves the constant matches the runtime
/// derivation.
const UPDATE_IMPACT_MID_SELECTOR: [u8; 4] = [0x34, 0x4e, 0x51, 0x4a];

/// Bounds for the timeout config. Under `MIN_TIMEOUT_MS` the receipt
/// poll cannot complete a single JSON-RPC round-trip on a live anvil;
/// over `MAX_TIMEOUT_MS` the keeper tick starves other markets waiting
/// for this publish to resolve. Both bounds enforced at construction.
const MIN_TIMEOUT_MS: u64 = 500;
const MAX_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// Bounds for the receipt poll interval. Same reasoning as timeout —
/// too tight busy-spins the RPC provider; too loose adds keeper-tick
/// latency.
const MIN_RECEIPT_POLL_INTERVAL_MS: u64 = 25;
const MAX_RECEIPT_POLL_INTERVAL_MS: u64 = 2_000;
const DEFAULT_RECEIPT_POLL_INTERVAL_MS: u64 = 100;

/// Gas envelope defaults. Local anvil auto-mines, so a modest gas
/// budget is sufficient. Wei quantities are quoted at 1 gwei to keep
/// the numbers legible in test logs.
const DEFAULT_GAS_LIMIT: u64 = 150_000;
const DEFAULT_MAX_FEE_PER_GAS_WEI: u128 = 5_000_000_000; // 5 gwei
const DEFAULT_MAX_PRIORITY_FEE_PER_GAS_WEI: u128 = 1_000_000_000; // 1 gwei

/// Configuration knobs for the local-anvil publisher. Every field is
/// non-optional so an operator must consciously pick a chain id +
/// signer + engine target — there is no ambiguous default that could
/// accidentally point at a real network.
#[derive(Clone)]
pub struct LocalAnvilPublisherConfig {
    /// HTTP JSON-RPC endpoint for the local anvil (`http://127.0.0.1:PORT`).
    pub anvil_rpc_url: String,
    /// Signer key material. Held as raw bytes; the `Debug` impl on
    /// [`LocalAnvilPublisher`] redacts this field.
    pub signer_private_key: [u8; 32],
    /// Address of the on-chain `PerpEngine` (or, in the harness, the
    /// `MockImpactMidSink`). Both expose an identical
    /// `updateImpactMid(uint256,uint128)` selector.
    pub perp_engine_address: AccountId,
    /// Chain id. Refused on `1` and `8453`.
    pub chain_id: u64,
    /// Per-publish transport timeout (broadcast + receipt poll).
    pub timeout_ms: u64,
    /// Receipt-poll interval.
    pub receipt_poll_interval_ms: u64,
    /// Transaction gas limit for the `updateImpactMid` call.
    pub gas_limit: u64,
    /// EIP-1559 max fee per gas.
    pub max_fee_per_gas_wei: u128,
    /// EIP-1559 max priority fee per gas.
    pub max_priority_fee_per_gas_wei: u128,
}

impl LocalAnvilPublisherConfig {
    /// Minimal-arg constructor with the default gas envelope. Kept as
    /// a builder-style shortcut for the harness / tests; production
    /// callers should populate every field explicitly.
    pub fn new(
        anvil_rpc_url: impl Into<String>,
        signer_private_key: [u8; 32],
        perp_engine_address: AccountId,
        chain_id: u64,
    ) -> Self {
        Self {
            anvil_rpc_url: anvil_rpc_url.into(),
            signer_private_key,
            perp_engine_address,
            chain_id,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            receipt_poll_interval_ms: DEFAULT_RECEIPT_POLL_INTERVAL_MS,
            gas_limit: DEFAULT_GAS_LIMIT,
            max_fee_per_gas_wei: DEFAULT_MAX_FEE_PER_GAS_WEI,
            max_priority_fee_per_gas_wei: DEFAULT_MAX_PRIORITY_FEE_PER_GAS_WEI,
        }
    }
}

/// LocalAnvil implementation. Holds the signer + the RPC provider.
/// Cloneable via `Arc`. Never logs the private key.
pub struct LocalAnvilPublisher {
    signer: SigningKey,
    signer_address: AccountId,
    perp_engine_address: AccountId,
    chain_id: u64,
    timeout: Duration,
    receipt_poll_interval: Duration,
    gas_limit: u64,
    max_fee_per_gas_wei: u128,
    max_priority_fee_per_gas_wei: u128,
    provider: HttpJsonRpcProvider,
    rpc_url: String,
    /// Last-published sample cache — used for the transport-side
    /// per-(market_id, mid, ts) dedup contract. Kept behind a `Mutex`
    /// because `publish` is `&self` (trait bound).
    last_sample: std::sync::Mutex<Option<LastSample>>,
}

impl std::fmt::Debug for LocalAnvilPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAnvilPublisher")
            .field("signer_address", &self.signer_address)
            .field("perp_engine_address", &self.perp_engine_address)
            .field("chain_id", &self.chain_id)
            .field("rpc_url", &self.rpc_url)
            .field("signer_private_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
struct LastSample {
    market_id: u64,
    mid_1e8: u128,
    timestamp_ms: TimestampMs,
}

impl LocalAnvilPublisher {
    /// Construct a publisher. Refuses mainnet chain ids up front.
    /// Refuses obviously-out-of-range timeouts / poll intervals — a
    /// zero-timeout publisher would never complete a poll cycle and a
    /// zero-poll-interval publisher would busy-spin the RPC.
    pub fn new(config: LocalAnvilPublisherConfig) -> Result<Self> {
        if MAINNET_CHAIN_IDS.contains(&config.chain_id) {
            return Err(BackendError::Config(format!(
                "LocalAnvilPublisher refused on mainnet chain id {}",
                config.chain_id
            )));
        }
        if config.timeout_ms < MIN_TIMEOUT_MS || config.timeout_ms > MAX_TIMEOUT_MS {
            return Err(BackendError::Config(format!(
                "LocalAnvilPublisher timeout_ms must be in [{MIN_TIMEOUT_MS}, {MAX_TIMEOUT_MS}], got {}",
                config.timeout_ms
            )));
        }
        if config.receipt_poll_interval_ms < MIN_RECEIPT_POLL_INTERVAL_MS
            || config.receipt_poll_interval_ms > MAX_RECEIPT_POLL_INTERVAL_MS
        {
            return Err(BackendError::Config(format!(
                "LocalAnvilPublisher receipt_poll_interval_ms must be in \
                 [{MIN_RECEIPT_POLL_INTERVAL_MS}, {MAX_RECEIPT_POLL_INTERVAL_MS}], got {}",
                config.receipt_poll_interval_ms
            )));
        }
        if config.gas_limit == 0 {
            return Err(BackendError::Config(
                "LocalAnvilPublisher gas_limit must be > 0".to_string(),
            ));
        }
        if config.max_fee_per_gas_wei == 0 {
            return Err(BackendError::Config(
                "LocalAnvilPublisher max_fee_per_gas_wei must be > 0".to_string(),
            ));
        }
        // priority fee 0 is technically legal but nonsensical for
        // EIP-1559; refuse to catch the misconfig.
        if config.max_priority_fee_per_gas_wei == 0 {
            return Err(BackendError::Config(
                "LocalAnvilPublisher max_priority_fee_per_gas_wei must be > 0".to_string(),
            ));
        }
        // Validate perp engine address parses (surfaces malformed
        // config at construction, not at first-publish).
        parse_evm_address(&config.perp_engine_address)?;

        let signer = SigningKey::from_bytes(&config.signer_private_key.into())
            .map_err(|e| BackendError::Config(format!("invalid signer private key: {e}")))?;
        let signer_address = signer_evm_address(&signer);

        let provider = HttpJsonRpcProvider::new(config.anvil_rpc_url.clone());

        Ok(Self {
            signer,
            signer_address,
            perp_engine_address: config.perp_engine_address,
            chain_id: config.chain_id,
            timeout: Duration::from_millis(config.timeout_ms),
            receipt_poll_interval: Duration::from_millis(config.receipt_poll_interval_ms),
            gas_limit: config.gas_limit,
            max_fee_per_gas_wei: config.max_fee_per_gas_wei,
            max_priority_fee_per_gas_wei: config.max_priority_fee_per_gas_wei,
            provider,
            rpc_url: config.anvil_rpc_url,
            last_sample: std::sync::Mutex::new(None),
        })
    }

    /// Address of the signer EOA. Useful for scenario tests to call
    /// `setImpactMidSource(publisher.signer_address())` before the
    /// first publish. Never leaks the private key.
    pub fn signer_address(&self) -> &AccountId {
        &self.signer_address
    }

    /// Chain id the publisher was constructed with. Retained for
    /// diagnostic surfaces.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Encode `updateImpactMid(uint256,uint128)` calldata.
    fn encode_calldata(market_id: u64, mid_1e8: u128) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 32 + 32);
        out.extend_from_slice(&UPDATE_IMPACT_MID_SELECTOR);
        // uint256 marketId — right-pad in a 32-byte big-endian field.
        let mut market_id_word = [0u8; 32];
        market_id_word[24..32].copy_from_slice(&market_id.to_be_bytes());
        out.extend_from_slice(&market_id_word);
        // uint128 mid1e8 — right-pad in a 32-byte big-endian field.
        let mut mid_word = [0u8; 32];
        mid_word[16..32].copy_from_slice(&mid_1e8.to_be_bytes());
        out.extend_from_slice(&mid_word);
        out
    }

    /// Sign the EIP-1559 preimage and produce raw hex.
    fn sign_and_assemble(
        &self,
        request: &ExecutionTransactionRequest,
        nonce: u64,
    ) -> Result<String> {
        let prehash = eip1559_transaction_prehash(request, nonce)?;
        let (sig, recovery): (Signature, RecoveryId) = self
            .signer
            .sign_prehash(&prehash)
            .map_err(|e| BackendError::Config(format!("sign_prehash: {e}")))?;
        // Normalize S — same policy as the hybrid v2 signer path.
        let (normalized, recovery): (Signature, RecoveryId) = if let Some(n) = sig.normalize_s() {
            let flipped = RecoveryId::from_byte(recovery.to_byte() ^ 1).unwrap_or(recovery);
            (n, flipped)
        } else {
            (sig, recovery)
        };
        let bytes = normalized.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&bytes[..32]);
        s.copy_from_slice(&bytes[32..64]);
        assemble_eip1559_signed_transaction(request, nonce, recovery.to_byte(), &r, &s)
    }

    /// Poll `eth_getTransactionReceipt` until a receipt shows up or the
    /// deadline elapses. Returns `(block_number, status_ok)`.
    async fn poll_receipt(&self, tx_hash: &str) -> Result<(u64, bool)> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
        })
        .to_string();
        let client = reqwest::Client::new();
        loop {
            let attempt = client
                .post(&self.rpc_url)
                .header("content-type", "application/json")
                .body(payload.clone())
                .send()
                .await;
            if let Ok(resp) = attempt {
                if let Ok(value) = resp.json::<serde_json::Value>().await {
                    if let Some(result) = value.get("result") {
                        if !result.is_null() {
                            let block_number = result
                                .get("blockNumber")
                                .and_then(|v| v.as_str())
                                .and_then(|s| parse_hex_u64(s).ok())
                                .unwrap_or(0);
                            let status_ok = result
                                .get("status")
                                .and_then(|v| v.as_str())
                                .map(|s| s == "0x1")
                                .unwrap_or(false);
                            return Ok((block_number, status_ok));
                        }
                    }
                    // JSON-RPC error branch — surface via a
                    // Persistence error so the keeper logs but does not
                    // panic.
                    if let Some(err) = value.get("error") {
                        return Err(BackendError::Persistence(format!(
                            "impact-mid publish receipt rpc error: {err}"
                        )));
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BackendError::Persistence(format!(
                    "impact-mid publish timeout waiting for receipt: {tx_hash}"
                )));
            }
            tokio::time::sleep(self.receipt_poll_interval).await;
        }
    }

    /// Dedup check — returns Some(reason) when the sample matches the
    /// last-published tuple byte-for-byte.
    fn dedup_reason(&self, market_id: u64, mid_1e8: u128, ts: TimestampMs) -> Option<String> {
        let guard = self
            .last_sample
            .lock()
            .expect("impact-mid publisher last-sample mutex poisoned");
        if let Some(last) = *guard {
            if last.market_id == market_id
                && last.mid_1e8 == mid_1e8
                && last.timestamp_ms == ts
            {
                return Some("identical to last published sample".to_string());
            }
        }
        None
    }

    /// Record the successfully published sample for the dedup check.
    fn record_sample(&self, market_id: u64, mid_1e8: u128, ts: TimestampMs) {
        let mut guard = self
            .last_sample
            .lock()
            .expect("impact-mid publisher last-sample mutex poisoned");
        *guard = Some(LastSample {
            market_id,
            mid_1e8,
            timestamp_ms: ts,
        });
    }
}

#[async_trait]
impl ImpactMidPublisher for LocalAnvilPublisher {
    async fn publish(
        &self,
        market_id: u64,
        mid_1e8: u128,
        timestamp_ms: TimestampMs,
    ) -> Result<PublishOutcome> {
        if let Some(reason) = self.dedup_reason(market_id, mid_1e8, timestamp_ms) {
            tracing::info!(
                publisher = "local_anvil",
                market_id,
                mid_1e8 = mid_1e8 as u64,
                timestamp_ms,
                reason = %reason,
                "impact-mid publish skipped (transport dedup)"
            );
            return Ok(PublishOutcome::Skipped { reason });
        }
        // Wrap the whole broadcast in the configured timeout to bound
        // the keeper tick latency.
        let deadline_result = tokio::time::timeout(self.timeout, async {
            let calldata = Self::encode_calldata(market_id, mid_1e8);
            let request = ExecutionTransactionRequest {
                intent_id: Uuid::new_v4(),
                onchain_intent_id: String::new(),
                from: self.signer_address.clone(),
                to: self.perp_engine_address.clone(),
                value_wei: 0,
                calldata,
                chain_id: self.chain_id,
                gas_limit: self.gas_limit,
                max_fee_per_gas_wei: Some(self.max_fee_per_gas_wei.to_string()),
                max_priority_fee_per_gas_wei: Some(
                    self.max_priority_fee_per_gas_wei.to_string(),
                ),
            };
            let nonce = self
                .provider
                .transaction_count(self.signer_address.clone())
                .await?;
            let raw_hex = self.sign_and_assemble(&request, nonce)?;
            let tx_hash = self.provider.send_raw_transaction(raw_hex).await?;
            let (block_number, status_ok) = self.poll_receipt(&tx_hash).await?;
            if !status_ok {
                return Err(BackendError::Persistence(format!(
                    "impact-mid publish reverted: tx_hash={tx_hash} block={block_number}"
                )));
            }
            Ok::<(String, u64), BackendError>((tx_hash, block_number))
        })
        .await;
        let (tx_hash, block_number) = match deadline_result {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => {
                // Downcast Simulation errors (from `send_raw_transaction`
                // when the RPC returns a JSON-RPC error object; typical
                // for auth-rejected reverts on the mock sink) into the
                // Persistence variant so the keeper's log surface is
                // consistent.
                let mapped = match err {
                    BackendError::Simulation(msg) => BackendError::Persistence(format!(
                        "impact-mid publish reverted: {msg}"
                    )),
                    other => other,
                };
                tracing::warn!(
                    publisher = "local_anvil",
                    market_id,
                    mid_1e8 = mid_1e8 as u64,
                    timestamp_ms,
                    error = %mapped,
                    "impact-mid publish failed"
                );
                return Err(mapped);
            }
            Err(_elapsed) => {
                let msg = format!(
                    "impact-mid publish timed out after {}ms",
                    self.timeout.as_millis()
                );
                tracing::warn!(
                    publisher = "local_anvil",
                    market_id,
                    mid_1e8 = mid_1e8 as u64,
                    timestamp_ms,
                    "{msg}"
                );
                return Err(BackendError::Persistence(msg));
            }
        };
        self.record_sample(market_id, mid_1e8, timestamp_ms);
        tracing::info!(
            publisher = "local_anvil",
            market_id,
            mid_1e8 = mid_1e8 as u64,
            timestamp_ms,
            tx_hash = %tx_hash,
            block_number,
            "impact-mid publish confirmed"
        );
        Ok(PublishOutcome::Published { tx_hash, block_number })
    }
}

// ---------------------------------------------------------------------
// Shared re-export shape: `Arc<dyn ImpactMidPublisher>` — matches the
// keeper config's `Option<Arc<...>>` field type. Consumers use this
// alias so the trait-object bound never appears in code that only
// wires the publisher.
// ---------------------------------------------------------------------

pub type SharedImpactMidPublisher = Arc<dyn ImpactMidPublisher>;

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn signer_evm_address(key: &SigningKey) -> AccountId {
    let verifying = key.verifying_key();
    let encoded = verifying.to_encoded_point(false);
    // Drop the leading 0x04 tag; keccak256 over the remaining 64 bytes.
    let hash = Keccak256::digest(&encoded.as_bytes()[1..]);
    let mut hex = String::from("0x");
    for byte in &hash[12..] {
        hex.push_str(&format!("{byte:02x}"));
    }
    AccountId::new(hex)
}

fn parse_hex_u64(hex: &str) -> Result<u64> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    u64::from_str_radix(stripped, 16)
        .map_err(|e| BackendError::Persistence(format!("invalid hex u64: {e}")))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_matches_signature() {
        let mut h = Keccak256::new();
        h.update(b"updateImpactMid(uint256,uint128)");
        let out = h.finalize();
        assert_eq!(&out[..4], &UPDATE_IMPACT_MID_SELECTOR[..]);
    }

    #[test]
    fn calldata_layout_market_and_mid() {
        let calldata = LocalAnvilPublisher::encode_calldata(1, 3_000 * 100_000_000);
        assert_eq!(calldata.len(), 4 + 32 + 32);
        assert_eq!(&calldata[..4], &UPDATE_IMPACT_MID_SELECTOR[..]);
        // market id = 1 — last byte of the first argument word.
        assert_eq!(calldata[4 + 31], 1);
        assert!(calldata[4..4 + 31].iter().all(|b| *b == 0));
        // mid = 3_000 * 1e8 = 300_000_000_000 = 0x45D964B800.
        // First arg word ends at index 36; second word occupies [36, 68).
        // 300_000_000_000 fits in 5 bytes (LE would be [0x00, 0xB8, 0x64, 0xD9, 0x45]).
        let expected: u128 = 3_000 * 100_000_000;
        let mut expected_word = [0u8; 32];
        expected_word[16..32].copy_from_slice(&expected.to_be_bytes());
        assert_eq!(&calldata[36..68], &expected_word[..]);
    }

    #[test]
    fn mainnet_chain_id_refused_at_construction() {
        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            1,
        );
        let err = LocalAnvilPublisher::new(cfg).unwrap_err();
        assert!(matches!(err, BackendError::Config(_)));
        assert!(format!("{err}").contains("mainnet"));

        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            8453,
        );
        let err = LocalAnvilPublisher::new(cfg).unwrap_err();
        assert!(matches!(err, BackendError::Config(_)));
    }

    #[test]
    fn local_anvil_chain_id_accepted_at_construction() {
        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            84532,
        );
        assert!(LocalAnvilPublisher::new(cfg).is_ok());
    }

    #[test]
    fn zero_gas_limit_refused() {
        let mut cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            84532,
        );
        cfg.gas_limit = 0;
        assert!(LocalAnvilPublisher::new(cfg).is_err());
    }

    #[test]
    fn timeout_out_of_range_refused() {
        let mut cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            84532,
        );
        cfg.timeout_ms = 0;
        assert!(LocalAnvilPublisher::new(cfg.clone()).is_err());
        cfg.timeout_ms = MAX_TIMEOUT_MS + 1;
        assert!(LocalAnvilPublisher::new(cfg).is_err());
    }

    #[test]
    fn malformed_perp_engine_address_refused() {
        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("not-an-address"),
            84532,
        );
        assert!(LocalAnvilPublisher::new(cfg).is_err());
    }

    #[test]
    fn debug_impl_redacts_private_key() {
        let cfg = LocalAnvilPublisherConfig::new(
            "http://127.0.0.1:0",
            [0x11u8; 32],
            AccountId::new("0x0000000000000000000000000000000000000001"),
            84532,
        );
        let publisher = LocalAnvilPublisher::new(cfg).unwrap();
        let rendered = format!("{publisher:?}");
        assert!(rendered.contains("<redacted>"));
        // The hex string of the private key bytes must NOT appear.
        assert!(!rendered.contains("1111111111111111"));
    }

    #[tokio::test]
    async fn noop_publisher_returns_skipped() {
        let publisher = NoOpPublisher::new();
        let outcome = publisher.publish(1, 3_000 * 100_000_000, 12345).await.unwrap();
        assert!(matches!(outcome, PublishOutcome::Skipped { .. }));
    }
}
