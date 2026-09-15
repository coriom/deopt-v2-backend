//! PERPS-BASE-SEPOLIA-BACKEND-BROADCAST-WORKER-V1 — real Base Sepolia
//! closed-test broadcast path for matched Perps execution intents.
//!
//! Composes existing primitives (`tx_builder`, `transaction`, `rpc`,
//! `ExecutorSigner`) into a fail-closed, crash-safe pipeline:
//!
//! 1. Preflight the on-chain PerpMatchingEngine state (chain-id,
//!    isExecutor(from), paused()) via `eth_call`.
//! 2. Build the canonical `ExecutionTransactionRequest` from the intent
//!    + signature pair using [`build_execution_transaction_request`].
//! 3. Acquire the executor pending nonce from the configured RPC via
//!    [`TransactionBroadcastProvider::transaction_count`].
//! 4. Compute the EIP-1559 prehash, sign, assemble the raw signed
//!    envelope, and derive the canonical transaction hash.
//! 5. Persist `(intent_id, tx_hash, nonce, raw)` to the repository
//!    BEFORE calling `eth_sendRawTransaction`. Crash between persist
//!    and send: on restart, [`reconcile_submitted`] observes the tx
//!    hash and cross-checks the chain.
//! 6. Send the raw tx and update status to `Submitted`.
//! 7. Poll [`transaction_receipt`]. On `status == 1` transition to
//!    `Confirmed`; on `status == 0` transition to `Failed` (durable
//!    rejection — no auto-resign; upstream must reset the intent
//!    explicitly).
//!
//! ## Fail-closed default
//!
//! [`BroadcastPolicy::broadcast_intent`] fails closed unless every
//! preflight predicate passes. In particular a signer whose address
//! does not match `config.executor_from_address` short-circuits before
//! the transaction is ever built. See [`GateMatrix`] for the full
//! table.
//!
//! ## Idempotency / crash-safety
//!
//! * Prior tx_hash present in repository → refuse to rebroadcast
//!   (`ensure_no_submitted_transaction`) and instead trigger
//!   reconciliation.
//! * `record_submitted_transaction` MUST land before the RPC call.
//! * Crash windows are documented in
//!   `docs/PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_WORKER_V1.md`.
//!
//! ## Scope
//!
//! Closed-test only. Public Perps route remains fail-closed; funding
//! remains OFF; collateral activation unchanged. This module owns
//! settlement broadcast only — market/config governance stays with
//! the ProtocolTimelock + OPS_MULTISIG governance path.

use crate::confirmation::ConfirmationReceipt;
use crate::error::{BackendError, Result};
use crate::execution::executor::ExecutionIntentRepository;
use crate::execution::intent::{ExecutionIntent, ExecutionIntentStatus};
use crate::execution::perp_trade::StoredTradeSignatures;
use crate::execution::rpc::{
    EthCallProvider, EthCallRequest, TransactionBroadcastProvider, TransactionReceiptProvider,
};
use crate::execution::signer::{ExecutorSigner, RecoverableSignature};
use crate::execution::transaction::{
    assemble_eip1559_signed_transaction, build_execution_transaction_request,
    derive_signed_transaction_hash, eip1559_transaction_prehash, ensure_no_submitted_transaction,
    ExecutionTransactionRequest,
};
use crate::execution::ExecutionConfig;
use crate::signing::eip712::parse_evm_address;
use crate::types::{now_ms, AccountId};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use uuid::Uuid;

/// Function selectors for the on-chain PerpMatchingEngine view surface.
///
/// * `isExecutor(address)` → `0xdebfda30`
/// * `paused()` → `0x5c975abb`
pub const PME_IS_EXECUTOR_SELECTOR: [u8; 4] = [0xde, 0xbf, 0xda, 0x30];
pub const PME_PAUSED_SELECTOR: [u8; 4] = [0x5c, 0x97, 0x5a, 0xbb];

/// Result of a PME on-chain preflight read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PmeState {
    pub is_executor: bool,
    pub paused: bool,
}

/// Fresh on-chain read of the PerpMatchingEngine executor bit and
/// pause flag for `from` against `pme`. Two `eth_call` roundtrips.
pub async fn preflight_pme_state<R>(rpc: &R, from: &AccountId, pme: &AccountId) -> Result<PmeState>
where
    R: EthCallProvider,
{
    let from_bytes = parse_evm_address(from)?;
    let mut is_executor_data = Vec::with_capacity(4 + 32);
    is_executor_data.extend_from_slice(&PME_IS_EXECUTOR_SELECTOR);
    // ABI-encode a single address argument: left-pad 12 zero bytes + 20-byte address.
    is_executor_data.extend_from_slice(&[0u8; 12]);
    is_executor_data.extend_from_slice(&from_bytes);
    let is_executor_out = rpc
        .eth_call(EthCallRequest {
            from: from.clone(),
            to: pme.clone(),
            data: is_executor_data,
            value: 0,
            gas_limit: None,
        })
        .await?;
    let is_executor = decode_bool_return(&is_executor_out.output, "isExecutor")?;

    let paused_data = PME_PAUSED_SELECTOR.to_vec();
    let paused_out = rpc
        .eth_call(EthCallRequest {
            from: from.clone(),
            to: pme.clone(),
            data: paused_data,
            value: 0,
            gas_limit: None,
        })
        .await?;
    let paused = decode_bool_return(&paused_out.output, "paused")?;

    Ok(PmeState {
        is_executor,
        paused,
    })
}

fn decode_bool_return(bytes: &[u8], call_name: &str) -> Result<bool> {
    if bytes.len() != 32 {
        return Err(BackendError::BroadcastRejected(format!(
            "unexpected {call_name} return length: {}",
            bytes.len()
        )));
    }
    // ABI-encoded bool: 32 bytes, last byte is 0x00 or 0x01.
    for byte in &bytes[..31] {
        if *byte != 0 {
            return Err(BackendError::BroadcastRejected(format!(
                "malformed {call_name} return (non-zero pad byte)"
            )));
        }
    }
    match bytes[31] {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(BackendError::BroadcastRejected(format!(
            "malformed {call_name} return (byte31={other:#x})"
        ))),
    }
}

/// Signer bind that survives the closed-test broadcast preflight
/// (address matches `config.executor_from_address`).
pub trait BroadcastSigner: Send + Sync {
    fn address(&self) -> AccountId;
    fn sign_prehash(&self, prehash: &[u8; 32]) -> Result<RecoverableSignature>;
}

impl BroadcastSigner for ExecutorSigner {
    fn address(&self) -> AccountId {
        ExecutorSigner::address(self).clone()
    }
    fn sign_prehash(&self, prehash: &[u8; 32]) -> Result<RecoverableSignature> {
        ExecutorSigner::sign_prehash(self, prehash)
    }
}

/// Row-level outcome for a single-intent broadcast.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BroadcastOutcome {
    pub intent_id: Uuid,
    pub tx_hash: String,
    pub nonce: u64,
    pub status: ExecutionIntentStatus,
    pub receipt_block_number: Option<u64>,
    pub error: Option<String>,
}

/// Summary of a reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileSummary {
    pub inspected: usize,
    pub confirmed: usize,
    pub failed: usize,
    pub still_pending: usize,
}

/// Gate matrix reported alongside a preflight rejection. Emitted only
/// on the failure path so the operator sees exactly which predicate
/// short-circuited without a stack dive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateMatrix {
    pub execution_enabled: bool,
    pub real_broadcast_enabled: bool,
    pub dry_run: bool,
    pub calldata_ready: bool,
    pub chain_id_ok: bool,
    pub signer_bind_ok: bool,
    pub is_executor: bool,
    pub paused: bool,
    pub already_submitted: bool,
}

/// The closed-test broadcast policy. Composes RPC + signer + config.
///
/// `poll_receipt_interval_ms` / `poll_receipt_max_attempts` bound how
/// long `broadcast_intent` waits for the initial receipt before
/// returning `Submitted` (leaving the intent for the reconciliation
/// pass). Defaults suit Base Sepolia (2s block time; 15 attempts × 2s
/// = 30s upper wait).
pub struct BroadcastPolicy<R, S> {
    pub config: ExecutionConfig,
    pub rpc: R,
    pub signer: Arc<S>,
    pub poll_receipt_interval_ms: u64,
    pub poll_receipt_max_attempts: u32,
}

impl<R, S> BroadcastPolicy<R, S>
where
    R: EthCallProvider
        + TransactionBroadcastProvider
        + TransactionReceiptProvider
        + Clone
        + Send
        + Sync,
    S: BroadcastSigner + 'static,
{
    pub fn new(config: ExecutionConfig, rpc: R, signer: Arc<S>) -> Self {
        Self {
            config,
            rpc,
            signer,
            poll_receipt_interval_ms: 2_000,
            poll_receipt_max_attempts: 15,
        }
    }

    /// Preflight the runtime configuration. Returns Ok(()) only if
    /// every gate predicate passes. See [`GateMatrix`].
    pub fn preflight_static(&self) -> Result<()> {
        if !self.config.execution_enabled {
            return Err(BackendError::BroadcastRejected(
                "executor.execution_enabled = false".to_string(),
            ));
        }
        if !self.config.real_broadcast_enabled {
            return Err(BackendError::BroadcastRejected(
                "executor.real_broadcast_enabled = false (fail-closed default)".to_string(),
            ));
        }
        if self.config.dry_run {
            return Err(BackendError::BroadcastRejected(
                "executor.dry_run = true — real broadcast refused".to_string(),
            ));
        }
        if self.config.rpc_url.is_none() {
            return Err(BackendError::BroadcastRejected(
                "RPC_URL is not configured".to_string(),
            ));
        }
        // Signer address must equal configured executor_from_address.
        let signer_addr = self.signer.address();
        if !addresses_equal(&signer_addr, &self.config.executor_from_address) {
            return Err(BackendError::BroadcastRejected(format!(
                "signer address ({}) does not match EXECUTOR_FROM_ADDRESS ({})",
                signer_addr.0, self.config.executor_from_address.0
            )));
        }
        Ok(())
    }

    /// Perform the full broadcast pipeline for a single intent. Fails
    /// closed on any preflight predicate.
    pub async fn broadcast_intent<Repo>(
        &self,
        repository: &Repo,
        intent: &ExecutionIntent,
        signatures: &StoredTradeSignatures,
    ) -> Result<BroadcastOutcome>
    where
        Repo: ExecutionIntentRepository,
    {
        self.preflight_static()?;

        // Idempotency guard — if an intent already has a submitted
        // tx_hash we refuse to rebroadcast. Reconciliation is the
        // supported recovery path.
        let prior_hash = repository.get_submitted_tx_hash(intent.intent_id).await?;
        ensure_no_submitted_transaction(prior_hash.is_some())?;

        // Chain identity guard — fresh eth_chainId roundtrip against
        // the broadcast RPC. Refuses a mainnet RPC pointed at a
        // testnet config, and vice versa.
        let rpc_chain_id = self.rpc.chain_id().await?;
        if rpc_chain_id != self.config.executor_chain_id {
            return Err(BackendError::BroadcastRejected(format!(
                "RPC chain_id ({rpc_chain_id}) does not match EXECUTOR_CHAIN_ID ({})",
                self.config.executor_chain_id
            )));
        }

        // PME on-chain state guard.
        let pme_state = preflight_pme_state(
            &self.rpc,
            &self.config.executor_from_address,
            &self.config.perp_matching_engine_address,
        )
        .await?;
        if !pme_state.is_executor {
            return Err(BackendError::BroadcastRejected(format!(
                "PME.isExecutor({}) == false — refuse to broadcast",
                self.config.executor_from_address.0
            )));
        }
        if pme_state.paused {
            return Err(BackendError::BroadcastRejected(
                "PME.paused() == true — refuse to broadcast".to_string(),
            ));
        }

        // Build the canonical execution request. Validates simulation
        // status (if required), signatures, and calldata shape.
        let request = build_execution_transaction_request(&self.config, intent, signatures)?;

        // Acquire executor pending nonce.
        let nonce = self
            .rpc
            .transaction_count(self.config.executor_from_address.clone())
            .await?;

        // Sign, assemble raw, derive canonical tx hash.
        let raw_hex = sign_transaction(&request, nonce, self.signer.as_ref())?;
        let tx_hash = derive_signed_transaction_hash(&raw_hex)?;

        // Persist BEFORE send. On crash between here and RPC send the
        // reconciliation pass observes the tx_hash and cross-checks the
        // chain — if no tx by that hash on chain, the DB row must be
        // manually reset (this is the safe direction).
        repository
            .record_submitted_transaction(
                intent.intent_id,
                tx_hash.clone(),
                nonce,
                raw_hex.clone(),
                now_ms(),
            )
            .await?;

        // Send. On send failure classify and surface the class to the
        // caller; the tx_hash remains persisted so the operator can
        // decide whether to reconcile or reset.
        let send_result = self.rpc.send_raw_transaction(raw_hex.clone()).await;
        let submitted_tx_hash = match send_result {
            Ok(hash) => hash,
            Err(error) => {
                let error_msg = error.to_string();
                let class = classify_send_error(&error_msg);
                match class {
                    SendErrorClass::AlreadyKnown | SendErrorClass::NonceTooLow => {
                        // Idempotent replay — the network already
                        // observed our tx. Fall through and rely on
                        // receipt polling for confirmation.
                        info!(
                            intent_id = %intent.intent_id,
                            class = ?class,
                            error = %error_msg,
                            "eth_sendRawTransaction returned idempotent class; falling through to receipt poll"
                        );
                        tx_hash.clone()
                    }
                    _ => {
                        // Unclassified transport / execution error.
                        // Surface as durable failure — the operator
                        // must inspect the tx_hash before any reset.
                        warn!(
                            intent_id = %intent.intent_id,
                            class = ?class,
                            error = %error_msg,
                            "eth_sendRawTransaction failed; marking intent Failed"
                        );
                        repository
                            .mark_intent_failed(
                                intent.intent_id,
                                format!("send_error: {error_msg}"),
                                now_ms(),
                            )
                            .await?;
                        return Ok(BroadcastOutcome {
                            intent_id: intent.intent_id,
                            tx_hash,
                            nonce,
                            status: ExecutionIntentStatus::Failed,
                            receipt_block_number: None,
                            error: Some(error_msg),
                        });
                    }
                }
            }
        };
        assert_tx_hashes_match(&tx_hash, &submitted_tx_hash)?;

        // Success post-send: mark Submitted. Optimistic mid-state so
        // an early crash / poll timeout leaves a reconcile-able row.
        repository
            .update_execution_intent_status(
                intent.intent_id,
                ExecutionIntentStatus::Submitted,
                now_ms(),
            )
            .await?;

        // Poll for receipt (bounded). If the receipt is not yet mined
        // within the poll budget we return early with Submitted; the
        // reconciler will resolve it later.
        let receipt = self.poll_receipt(&tx_hash).await?;
        match receipt {
            Some(receipt) => {
                self.finalize_receipt(repository, intent.intent_id, &tx_hash, nonce, receipt)
                    .await
            }
            None => Ok(BroadcastOutcome {
                intent_id: intent.intent_id,
                tx_hash,
                nonce,
                status: ExecutionIntentStatus::Submitted,
                receipt_block_number: None,
                error: None,
            }),
        }
    }

    async fn poll_receipt(&self, tx_hash: &str) -> Result<Option<ConfirmationReceipt>> {
        for _ in 0..self.poll_receipt_max_attempts {
            match self.rpc.transaction_receipt(tx_hash.to_string()).await? {
                Some(receipt) => return Ok(Some(receipt)),
                None => sleep(Duration::from_millis(self.poll_receipt_interval_ms)).await,
            }
        }
        Ok(None)
    }

    async fn finalize_receipt<Repo>(
        &self,
        repository: &Repo,
        intent_id: Uuid,
        expected_tx_hash: &str,
        nonce: u64,
        receipt: ConfirmationReceipt,
    ) -> Result<BroadcastOutcome>
    where
        Repo: ExecutionIntentRepository,
    {
        if !receipt.tx_hash.eq_ignore_ascii_case(expected_tx_hash) {
            let error = format!(
                "receipt_tx_hash_mismatch: expected {expected_tx_hash}, got {}",
                receipt.tx_hash
            );
            repository
                .mark_intent_failed(intent_id, error.clone(), now_ms())
                .await?;
            return Ok(BroadcastOutcome {
                intent_id,
                tx_hash: expected_tx_hash.to_string(),
                nonce,
                status: ExecutionIntentStatus::Failed,
                receipt_block_number: receipt.block_number,
                error: Some(error),
            });
        }
        match receipt.status {
            Some(1) => {
                let block = receipt.block_number.unwrap_or_default();
                repository
                    .mark_intent_confirmed(intent_id, block, now_ms())
                    .await?;
                Ok(BroadcastOutcome {
                    intent_id,
                    tx_hash: expected_tx_hash.to_string(),
                    nonce,
                    status: ExecutionIntentStatus::Confirmed,
                    receipt_block_number: Some(block),
                    error: None,
                })
            }
            Some(0) => {
                let error = "receipt.status = 0 (reverted on-chain)".to_string();
                repository
                    .mark_intent_failed(intent_id, error.clone(), now_ms())
                    .await?;
                Ok(BroadcastOutcome {
                    intent_id,
                    tx_hash: expected_tx_hash.to_string(),
                    nonce,
                    status: ExecutionIntentStatus::Failed,
                    receipt_block_number: receipt.block_number,
                    error: Some(error),
                })
            }
            other => {
                let error = format!("receipt.status = {other:?} (unexpected)");
                repository
                    .mark_intent_failed(intent_id, error.clone(), now_ms())
                    .await?;
                Ok(BroadcastOutcome {
                    intent_id,
                    tx_hash: expected_tx_hash.to_string(),
                    nonce,
                    status: ExecutionIntentStatus::Failed,
                    receipt_block_number: receipt.block_number,
                    error: Some(error),
                })
            }
        }
    }

    /// Restart-safe reconciliation. For each intent currently in
    /// `Submitted` status, look up the tx hash on chain and transition
    /// to `Confirmed` or `Failed` accordingly. Intents whose receipts
    /// are not yet mined stay in `Submitted` (idempotent — next pass
    /// re-checks).
    pub async fn reconcile_submitted<Repo>(
        &self,
        repository: &Repo,
        max_intents: u32,
    ) -> Result<ReconcileSummary>
    where
        Repo: ExecutionIntentRepository,
    {
        let mut summary = ReconcileSummary::default();
        let submitted = repository
            .list_submitted_execution_intents(max_intents)
            .await?;
        summary.inspected = submitted.len();
        for intent in &submitted {
            let Some(tx_hash) = repository.get_submitted_tx_hash(intent.intent_id).await? else {
                // Row is Submitted but no tx_hash — an operational
                // anomaly. Leave it alone; manual intervention.
                summary.still_pending += 1;
                continue;
            };
            match self.rpc.transaction_receipt(tx_hash.clone()).await? {
                Some(receipt) => {
                    let outcome = self
                        .finalize_receipt(
                            repository,
                            intent.intent_id,
                            &tx_hash,
                            /*nonce*/ 0,
                            receipt,
                        )
                        .await?;
                    match outcome.status {
                        ExecutionIntentStatus::Confirmed => summary.confirmed += 1,
                        ExecutionIntentStatus::Failed => summary.failed += 1,
                        _ => summary.still_pending += 1,
                    }
                }
                None => summary.still_pending += 1,
            }
        }
        Ok(summary)
    }
}

fn addresses_equal(a: &AccountId, b: &AccountId) -> bool {
    let left = a.0.trim().trim_start_matches("0x").to_ascii_lowercase();
    let right = b.0.trim().trim_start_matches("0x").to_ascii_lowercase();
    left == right
}

fn sign_transaction<S: BroadcastSigner + ?Sized>(
    request: &ExecutionTransactionRequest,
    nonce: u64,
    signer: &S,
) -> Result<String> {
    let prehash = eip1559_transaction_prehash(request, nonce)?;
    let signature = signer.sign_prehash(&prehash)?;
    assemble_eip1559_signed_transaction(
        request,
        nonce,
        signature.y_parity,
        &signature.r,
        &signature.s,
    )
}

fn assert_tx_hashes_match(derived: &str, submitted: &str) -> Result<()> {
    if derived.eq_ignore_ascii_case(submitted) {
        Ok(())
    } else {
        Err(BackendError::BroadcastRejected(format!(
            "eth_sendRawTransaction returned tx_hash={submitted} but envelope derived {derived}"
        )))
    }
}

/// Coarse classification of `eth_sendRawTransaction` error messages
/// so we can distinguish idempotent replays from real transport /
/// execution failures. Deliberately conservative — anything not
/// clearly matching a known idempotent class is treated as `Other`
/// and surfaced as a durable failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendErrorClass {
    AlreadyKnown,
    NonceTooLow,
    ReplacementUnderpriced,
    Other,
}

pub fn classify_send_error(message: &str) -> SendErrorClass {
    let m = message.to_ascii_lowercase();
    if m.contains("already known") || m.contains("known transaction") {
        SendErrorClass::AlreadyKnown
    } else if m.contains("nonce too low") {
        SendErrorClass::NonceTooLow
    } else if m.contains("replacement transaction underpriced")
        || m.contains("replacement underpriced")
    {
        SendErrorClass::ReplacementUnderpriced
    } else {
        SendErrorClass::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirmation::ConfirmationReceipt;
    use crate::execution::executor::RepositoryFuture;
    use crate::execution::intent::ExecutionIntent;
    use crate::execution::perp_trade::{PerpTradeSignatureBundle, StoredTradeSignatures};
    use crate::execution::rpc::{EthCallSuccess, RpcFuture};
    use crate::execution::PrivateKeySecret;
    use crate::types::{OrderId, TimestampMs};
    use std::sync::Mutex;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    // Address derived from TEST_KEY (lower-case; the signer-bind check
    // is case-insensitive).
    const TEST_KEY_ADDRESS: &str = "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23";
    const PME: &str = "0x774d96E5739bffadEE91508b4D3D74F5BE29F165";

    // ---- fixture builders ----

    fn make_intent() -> ExecutionIntent {
        ExecutionIntent {
            intent_id: Uuid::from_u128(1),
            market_id: 1,
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            price_1e8: 300_000_000_000,
            size_1e8: 100_000_000,
            buy_order_id: OrderId(Uuid::from_u128(2)),
            sell_order_id: OrderId(Uuid::from_u128(3)),
            buyer_is_maker: Some(false),
            buyer_nonce: Some(11),
            seller_nonce: Some(12),
            deadline_ms: Some(4_102_444_800),
            created_at_ms: 123,
            status: ExecutionIntentStatus::SimulationOk,
        }
    }

    fn make_signatures() -> StoredTradeSignatures {
        use crate::execution::transaction::hex_0x;
        let bundle =
            PerpTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap();
        StoredTradeSignatures {
            buyer_sig: Some(hex_0x(&bundle.buyer_sig)),
            seller_sig: Some(hex_0x(&bundle.seller_sig)),
        }
    }

    fn signature_hex(byte: u8) -> String {
        let mut s = String::from("0x");
        for _ in 0..65 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }

    fn make_config(from: &str, real_broadcast: bool, dry_run: bool) -> ExecutionConfig {
        ExecutionConfig {
            execution_enabled: true,
            dry_run,
            poll_interval_ms: 100,
            max_batch_size: 10,
            real_broadcast_enabled: real_broadcast,
            executor_private_key: Some(PrivateKeySecret::new(TEST_KEY.to_string())),
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: Some("1000000000".to_string()),
            max_priority_fee_per_gas_wei: Some("100000000".to_string()),
            require_simulation_ok: true,
            simulation_enabled: false,
            simulation_requires_persistence: false,
            rpc_url: Some("http://mock.invalid".to_string()),
            executor_from_address: AccountId::new(from.to_string()),
            perp_matching_engine_address: AccountId::new(PME.to_string()),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            old_perp_engine_address: None,
            backend_signer_mode: crate::execution::SignerBackendKind::LocalDev,
            backend_signer_endpoint: None,
            executor_allow_local_signer: true,
            backend_signer_provider: None,
            backend_signer_timeout_ms: 2500,
        }
    }

    fn make_signer() -> Arc<ExecutorSigner> {
        Arc::new(
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap(),
        )
    }

    // ---- mock repository ----

    #[derive(Clone, Default)]
    struct MockRepo {
        intents: Arc<Mutex<Vec<ExecutionIntent>>>,
        signatures: Arc<Mutex<Option<StoredTradeSignatures>>>,
        submitted_tx: Arc<Mutex<std::collections::HashMap<Uuid, (String, u64, String)>>>,
        confirmed: Arc<Mutex<Vec<(Uuid, u64)>>>,
        failed: Arc<Mutex<Vec<(Uuid, String)>>>,
    }

    impl MockRepo {
        fn with(intent: ExecutionIntent, signatures: StoredTradeSignatures) -> Self {
            Self {
                intents: Arc::new(Mutex::new(vec![intent])),
                signatures: Arc::new(Mutex::new(Some(signatures))),
                ..Default::default()
            }
        }

        fn submitted_tx_hash(&self, intent_id: Uuid) -> Option<String> {
            self.submitted_tx
                .lock()
                .unwrap()
                .get(&intent_id)
                .map(|(hash, _, _)| hash.clone())
        }

        fn status(&self, intent_id: Uuid) -> ExecutionIntentStatus {
            self.intents
                .lock()
                .unwrap()
                .iter()
                .find(|i| i.intent_id == intent_id)
                .map(|i| i.status)
                .unwrap()
        }

        fn preset_submitted(&self, intent_id: Uuid, tx_hash: String) {
            self.submitted_tx
                .lock()
                .unwrap()
                .insert(intent_id, (tx_hash, 0, String::new()));
        }
    }

    impl ExecutionIntentRepository for MockRepo {
        fn list_pending_execution_intents(
            &self,
            limit: u32,
        ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
            let result = {
                let intents = self.intents.lock().unwrap();
                Ok(intents
                    .iter()
                    .filter(|i| i.status == ExecutionIntentStatus::Pending)
                    .take(limit as usize)
                    .cloned()
                    .collect())
            };
            Box::pin(async move { result })
        }
        fn update_execution_intent_status(
            &self,
            intent_id: Uuid,
            status: ExecutionIntentStatus,
            _updated_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, ()> {
            let result = {
                let mut intents = self.intents.lock().unwrap();
                if let Some(i) = intents.iter_mut().find(|i| i.intent_id == intent_id) {
                    i.status = status;
                    Ok(())
                } else {
                    Err(BackendError::Persistence("intent not found".to_string()))
                }
            };
            Box::pin(async move { result })
        }
        fn get_execution_intent_signatures(
            &self,
            _intent_id: Uuid,
        ) -> RepositoryFuture<'_, StoredTradeSignatures> {
            let signatures = self.signatures.lock().unwrap().clone().unwrap_or_default();
            Box::pin(async move { Ok(signatures) })
        }
        fn record_submitted_transaction(
            &self,
            intent_id: Uuid,
            tx_hash: String,
            nonce: u64,
            raw_tx_hex: String,
            _submitted_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, ()> {
            self.submitted_tx
                .lock()
                .unwrap()
                .insert(intent_id, (tx_hash, nonce, raw_tx_hex));
            Box::pin(async move { Ok(()) })
        }
        fn get_submitted_tx_hash(&self, intent_id: Uuid) -> RepositoryFuture<'_, Option<String>> {
            let hash = self.submitted_tx_hash(intent_id);
            Box::pin(async move { Ok(hash) })
        }
        fn list_submitted_execution_intents(
            &self,
            limit: u32,
        ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
            let result = {
                let intents = self.intents.lock().unwrap();
                Ok(intents
                    .iter()
                    .filter(|i| i.status == ExecutionIntentStatus::Submitted)
                    .take(limit as usize)
                    .cloned()
                    .collect())
            };
            Box::pin(async move { result })
        }
        fn mark_intent_confirmed(
            &self,
            intent_id: Uuid,
            receipt_block_number: u64,
            _confirmed_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, ()> {
            self.confirmed
                .lock()
                .unwrap()
                .push((intent_id, receipt_block_number));
            let mut intents = self.intents.lock().unwrap();
            if let Some(i) = intents.iter_mut().find(|i| i.intent_id == intent_id) {
                i.status = ExecutionIntentStatus::Confirmed;
            }
            Box::pin(async move { Ok(()) })
        }
        fn mark_intent_failed(
            &self,
            intent_id: Uuid,
            reason: String,
            _failed_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, ()> {
            self.failed.lock().unwrap().push((intent_id, reason));
            let mut intents = self.intents.lock().unwrap();
            if let Some(i) = intents.iter_mut().find(|i| i.intent_id == intent_id) {
                i.status = ExecutionIntentStatus::Failed;
            }
            Box::pin(async move { Ok(()) })
        }
    }

    // ---- mock RPC ----

    #[derive(Clone, Default)]
    struct MockRpc {
        chain_id: Arc<Mutex<u64>>,
        pending_nonce: Arc<Mutex<u64>>,
        is_executor: Arc<Mutex<bool>>,
        paused: Arc<Mutex<bool>>,
        send_err: Arc<Mutex<Option<String>>>,
        receipt: Arc<Mutex<Option<ConfirmationReceipt>>>,
        sent_raw: Arc<Mutex<Vec<String>>>,
    }

    impl MockRpc {
        fn new() -> Self {
            Self {
                chain_id: Arc::new(Mutex::new(84532)),
                pending_nonce: Arc::new(Mutex::new(0)),
                is_executor: Arc::new(Mutex::new(true)),
                paused: Arc::new(Mutex::new(false)),
                send_err: Arc::new(Mutex::new(None)),
                receipt: Arc::new(Mutex::new(None)),
                sent_raw: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn with_chain(self, id: u64) -> Self {
            *self.chain_id.lock().unwrap() = id;
            self
        }
        fn with_is_executor(self, v: bool) -> Self {
            *self.is_executor.lock().unwrap() = v;
            self
        }
        fn with_paused(self, v: bool) -> Self {
            *self.paused.lock().unwrap() = v;
            self
        }
        fn with_receipt(self, r: ConfirmationReceipt) -> Self {
            *self.receipt.lock().unwrap() = Some(r);
            self
        }
    }

    impl EthCallProvider for MockRpc {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let selector = &request.data.get(..4).unwrap_or(&[0u8; 4]).to_vec();
            let is_executor = *self.is_executor.lock().unwrap();
            let paused = *self.paused.lock().unwrap();
            let out = if selector.as_slice() == PME_IS_EXECUTOR_SELECTOR {
                bool_return(is_executor)
            } else if selector.as_slice() == PME_PAUSED_SELECTOR {
                bool_return(paused)
            } else {
                vec![0u8; 32]
            };
            Box::pin(async move {
                Ok(EthCallSuccess {
                    block_number: Some(1),
                    output: out,
                })
            })
        }
    }

    impl TransactionBroadcastProvider for MockRpc {
        fn chain_id(&self) -> RpcFuture<'_, u64> {
            let v = *self.chain_id.lock().unwrap();
            Box::pin(async move { Ok(v) })
        }
        fn transaction_count(&self, _address: AccountId) -> RpcFuture<'_, u64> {
            let v = *self.pending_nonce.lock().unwrap();
            Box::pin(async move { Ok(v) })
        }
        fn send_raw_transaction(&self, raw_transaction: String) -> RpcFuture<'_, String> {
            self.sent_raw.lock().unwrap().push(raw_transaction.clone());
            let err = self.send_err.lock().unwrap().clone();
            let hash = derive_signed_transaction_hash(&raw_transaction);
            Box::pin(async move {
                if let Some(e) = err {
                    return Err(BackendError::Simulation(e));
                }
                hash
            })
        }
    }

    impl TransactionReceiptProvider for MockRpc {
        fn block_number(&self) -> RpcFuture<'_, u64> {
            Box::pin(async { Ok(1) })
        }
        fn transaction_receipt(
            &self,
            _tx_hash: String,
        ) -> RpcFuture<'_, Option<ConfirmationReceipt>> {
            let r = self.receipt.lock().unwrap().clone();
            Box::pin(async move { Ok(r) })
        }
    }

    fn bool_return(v: bool) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        if v {
            out[31] = 1;
        }
        out
    }

    fn happy_receipt(tx_hash: &str) -> ConfirmationReceipt {
        ConfirmationReceipt {
            tx_hash: tx_hash.to_string(),
            status: Some(1),
            block_number: Some(42),
            gas_used: Some(400_000),
            effective_gas_price: Some("6000000".to_string()),
            cumulative_gas_used: Some(400_000),
            block_hash: Some("0xabc".to_string()),
            transaction_index: Some(0),
        }
    }

    fn reverted_receipt(tx_hash: &str) -> ConfirmationReceipt {
        ConfirmationReceipt {
            tx_hash: tx_hash.to_string(),
            status: Some(0),
            block_number: Some(42),
            gas_used: Some(21_000),
            effective_gas_price: Some("6000000".to_string()),
            cumulative_gas_used: Some(21_000),
            block_hash: Some("0xabc".to_string()),
            transaction_index: Some(0),
        }
    }

    fn make_policy_with(rpc: MockRpc, from: &str) -> BroadcastPolicy<MockRpc, ExecutorSigner> {
        let config = make_config(from, true, false);
        let signer = make_signer();
        let mut policy = BroadcastPolicy::new(config, rpc, signer);
        // Speed up test polling.
        policy.poll_receipt_interval_ms = 1;
        policy.poll_receipt_max_attempts = 3;
        policy
    }

    // ============ TEST MATRIX ============

    // (A) dry-run behavior remains unchanged — no attempt to broadcast.
    // Covered by the pre-existing dry-run test in executor.rs; here we
    // additionally prove the policy rejects when dry_run=true.
    #[tokio::test]
    async fn a_dry_run_config_refuses_broadcast() {
        let rpc = MockRpc::new();
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.config.dry_run = true;
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("dry_run")));
    }

    // (B) real mode missing RPC_URL => fail closed
    #[tokio::test]
    async fn b_missing_rpc_url_fails_closed() {
        let rpc = MockRpc::new();
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.config.rpc_url = None;
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("RPC_URL")));
    }

    // (C) signer address != EXECUTOR_FROM_ADDRESS => fail closed
    #[tokio::test]
    async fn c_signer_address_mismatch_fails_closed() {
        let rpc = MockRpc::new();
        let policy = make_policy_with(rpc, "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("signer address"))
        );
    }

    // (E) wrong chainId => fail closed
    #[tokio::test]
    async fn e_wrong_chain_id_fails_closed() {
        let rpc = MockRpc::new().with_chain(8453); // mainnet chain id
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("chain_id")));
    }

    // (F) PME reports configured executor unauthorized => fail closed
    #[tokio::test]
    async fn f_pme_unauthorized_executor_fails_closed() {
        let rpc = MockRpc::new().with_is_executor(false);
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("isExecutor")));
    }

    // (G) PME paused => fail closed
    #[tokio::test]
    async fn g_pme_paused_fails_closed() {
        let rpc = MockRpc::new().with_paused(true);
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("paused")));
    }

    // (H) valid mocked broadcast — happy path
    #[tokio::test]
    async fn h_happy_path_broadcast_confirms() {
        // First we need to know the derived tx hash to seed the mocked
        // receipt. Sign once to derive it.
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let rpc = MockRpc::new().with_receipt(happy_receipt(&tx_hash));
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Confirmed);
        assert_eq!(outcome.receipt_block_number, Some(42));
        assert_eq!(outcome.tx_hash, tx_hash);
        assert_eq!(
            repo.status(intent.intent_id),
            ExecutionIntentStatus::Confirmed
        );
        assert!(repo.submitted_tx_hash(intent.intent_id).is_some());
    }

    // (I) reverted receipt => durable failure
    #[tokio::test]
    async fn i_reverted_receipt_marks_failed() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let rpc = MockRpc::new().with_receipt(reverted_receipt(&tx_hash));
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Failed);
        assert_eq!(repo.status(intent.intent_id), ExecutionIntentStatus::Failed);
    }

    // (J) restart after tx submission => reconcile instead of rebroadcast
    #[tokio::test]
    async fn j_reconcile_confirms_prior_submitted() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = ExecutionIntent {
            status: ExecutionIntentStatus::Submitted,
            ..make_intent()
        };
        let signatures = make_signatures();
        let request =
            build_execution_transaction_request(&config, &make_intent(), &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let rpc = MockRpc::new().with_receipt(happy_receipt(&tx_hash));
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let repo = MockRepo::with(intent.clone(), signatures.clone());
        repo.preset_submitted(intent.intent_id, tx_hash.clone());

        let summary = policy.reconcile_submitted(&repo, 10).await.unwrap();
        assert_eq!(summary.inspected, 1);
        assert_eq!(summary.confirmed, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(
            repo.status(intent.intent_id),
            ExecutionIntentStatus::Confirmed
        );
    }

    // (K) duplicate execution candidate => no duplicate broadcast
    #[tokio::test]
    async fn k_idempotency_refuses_rebroadcast_when_tx_hash_present() {
        let rpc = MockRpc::new();
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        repo.preset_submitted(intent.intent_id, "0xdeadbeef".to_string());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::BroadcastRejected(msg) if msg.contains("already has a submitted"))
        );
    }

    // (L) nonce-too-low / already-known classification
    #[test]
    fn l_send_error_classification() {
        assert_eq!(
            classify_send_error("nonce too low"),
            SendErrorClass::NonceTooLow
        );
        assert_eq!(
            classify_send_error("already known"),
            SendErrorClass::AlreadyKnown
        );
        assert_eq!(
            classify_send_error("known transaction: 0xabc"),
            SendErrorClass::AlreadyKnown
        );
        assert_eq!(
            classify_send_error("replacement transaction underpriced"),
            SendErrorClass::ReplacementUnderpriced
        );
        assert_eq!(
            classify_send_error("insufficient funds"),
            SendErrorClass::Other
        );
    }

    // (M) old deployer executor cannot be used by runtime config — the
    // config's executor_from_address is enforced by the signer bind
    // check (test C). Additionally, if the operator configures the
    // deployer address AND the on-chain isExecutor(deployer) is false
    // (post-rotation state), preflight rejects.
    #[tokio::test]
    async fn m_deployer_address_rejected_when_pme_not_authorizing() {
        // Simulate: config points at deployer address, signer address
        // does NOT match (real security bind check), on-chain PME says
        // isExecutor(deployer) = false. Signer-bind fires first.
        let rpc = MockRpc::new().with_is_executor(false);
        let policy = make_policy_with(rpc, "0xc35F7A8A103A9A4464adfaa76B9B514093D23C27");
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), make_signatures());
        let err = policy
            .broadcast_intent(&repo, &intent, &make_signatures())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::BroadcastRejected(_)));
    }

    // (D) missing calldata_ready (no signatures) => fail closed
    #[tokio::test]
    async fn d_missing_signatures_fails_closed() {
        let rpc = MockRpc::new();
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let repo = MockRepo::with(intent.clone(), StoredTradeSignatures::default());
        let err = policy
            .broadcast_intent(&repo, &intent, &StoredTradeSignatures::default())
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::MissingTradeSignatures));
    }

    // Preflight helper (isolated test) — encoded selectors match cast.
    #[test]
    fn selectors_match_precomputed() {
        // isExecutor(address) → 0xdebfda30
        assert_eq!(PME_IS_EXECUTOR_SELECTOR, [0xde, 0xbf, 0xda, 0x30]);
        // paused() → 0x5c975abb
        assert_eq!(PME_PAUSED_SELECTOR, [0x5c, 0x97, 0x5a, 0xbb]);
    }
}
