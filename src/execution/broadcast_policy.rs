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
use crate::execution::executor::{
    ExecutionIntentRepository, PreparedBroadcastRow, PreparedTransactionRecord,
};
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
/// * `intentFilled(bytes32)` → `0x209905a5`
pub const PME_IS_EXECUTOR_SELECTOR: [u8; 4] = [0xde, 0xbf, 0xda, 0x30];
pub const PME_PAUSED_SELECTOR: [u8; 4] = [0x5c, 0x97, 0x5a, 0xbb];
pub const PME_INTENT_FILLED_SELECTOR: [u8; 4] = [0x20, 0x99, 0x05, 0xa5];

/// Event topic0 for
/// `TradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)`
/// emitted by the PerpMatchingEngine on the pre-matched settlement
/// path. The intent_id is topic[1] (indexed).
pub const PME_TRADE_EXECUTED_TOPIC0: [u8; 32] = [
    0x50, 0x18, 0xa0, 0xa7, 0x3d, 0x56, 0xc0, 0x0e, 0x01, 0x81, 0x56, 0x36, 0xcf, 0x5e, 0x02, 0x9f,
    0xd7, 0xed, 0x94, 0x40, 0xd4, 0x2b, 0x3e, 0xea, 0x0e, 0x75, 0x40, 0x4d, 0xfe, 0xdb, 0x3f, 0x80,
];

/// Event topic0 for
/// `TradeExecutedFromIntents(bytes32,bytes32,uint256,uint128,uint128,uint256)`
/// emitted by the PerpMatchingEngine on the intent-based settlement
/// path. Kept for future use.
pub const PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0: [u8; 32] = [
    0x56, 0x0e, 0xbd, 0x5f, 0xdf, 0x7a, 0x64, 0xe8, 0x82, 0x68, 0x99, 0xb2, 0x6d, 0xaf, 0xd0, 0xda,
    0x5f, 0x70, 0xaa, 0x61, 0x3c, 0x89, 0xa8, 0x5c, 0xdb, 0x27, 0xbd, 0xd5, 0x24, 0x34, 0x24, 0x40,
];

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
    /// Number of Prepared rows where the reconciler rebroadcast the
    /// byte-identical raw envelope (no new nonce, no new tx_hash).
    pub rebroadcast_attempted: usize,
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
    /// Age threshold (ms) past which a `Prepared` row with no receipt
    /// is eligible for a byte-identical rebroadcast in the reconciler.
    pub rebroadcast_after_ms: u64,
    /// Absolute cap on the number of times the reconciler will
    /// rebroadcast the byte-identical raw envelope. Enforced per
    /// intent via the durable `send_attempts` counter. Deliberately
    /// low (default 8) — beyond this the intent stays in `Prepared`
    /// and requires operator intervention.
    pub rebroadcast_max_attempts: u32,
    /// Expected on-chain event topic0 emitted by the PerpMatchingEngine
    /// on a successful settlement. Verified inside `finalize_receipt`
    /// (when `verify_pme_event` is `true`) via
    /// [`crate::execution::rpc::EthLogsProvider`].
    pub expected_pme_topic0: [u8; 32],
    /// Toggle enabling on-chain event verification in addition to the
    /// standard receipt.status == 1 check. Default true — disable only
    /// for tests where the mock RPC does not surface logs.
    pub verify_pme_event: bool,
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
            rebroadcast_after_ms: 15_000,
            rebroadcast_max_attempts: 8,
            expected_pme_topic0: PME_TRADE_EXECUTED_TOPIC0,
            // Default off — enabled by production wiring once the logs
            // provider is configured. Tests toggle explicitly.
            verify_pme_event: false,
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

        // Idempotency guard — if an intent already has a prepared or
        // submitted tx_hash we refuse to build a NEW transaction.
        // Reconciliation is the supported recovery path and will
        // rebroadcast the byte-identical raw envelope where safe.
        let prior_row = repository.get_prepared_broadcast(intent.intent_id).await?;
        ensure_no_submitted_transaction(prior_row.is_some())?;

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

        // Persist the raw envelope BEFORE send. Transitions the intent
        // to `Prepared`. Crash between here and the RPC call is now
        // safe — the reconciler recovers by rebroadcasting the
        // byte-identical raw_hex (same nonce, same tx_hash) or
        // observing a receipt already produced by an earlier attempt.
        repository
            .record_prepared_transaction(PreparedTransactionRecord {
                intent_id: intent.intent_id,
                chain_id: self.config.executor_chain_id,
                executor_address: self.config.executor_from_address.clone(),
                target_address: self.config.perp_matching_engine_address.clone(),
                tx_hash: tx_hash.clone(),
                nonce,
                raw_tx_hex: raw_hex.clone(),
                prepared_at_ms: now_ms(),
            })
            .await?;
        repository
            .update_execution_intent_status(
                intent.intent_id,
                ExecutionIntentStatus::Prepared,
                now_ms(),
            )
            .await?;

        // Send. Classify errors: deterministic → durable Failed;
        // idempotent-replay / ambiguous → stay in Prepared and rely on
        // receipt polling. See `SendErrorClass` docs for the full
        // taxonomy.
        let _attempts = repository
            .bump_send_attempt(intent.intent_id, now_ms())
            .await?;
        let send_result = self.rpc.send_raw_transaction(raw_hex.clone()).await;
        match send_result {
            Ok(submitted_hash) => {
                assert_tx_hashes_match(&tx_hash, &submitted_hash)?;
            }
            Err(error) => {
                let error_msg = error.to_string();
                let class = classify_send_error(&error_msg);
                match class {
                    SendErrorClass::AlreadyKnown | SendErrorClass::NonceTooLow => {
                        info!(
                            intent_id = %intent.intent_id,
                            class = ?class,
                            error = %error_msg,
                            "send returned idempotent class; falling through to receipt poll"
                        );
                    }
                    SendErrorClass::Ambiguous | SendErrorClass::ReplacementUnderpriced => {
                        // AMBIGUOUS: RPC transport failed after the
                        // node may have accepted the tx. Do NOT mark
                        // Failed. Row stays in `Prepared`; reconciler
                        // will either observe a receipt (Confirmed /
                        // Failed) or, past the retry window, rebroadcast
                        // the byte-identical raw envelope.
                        warn!(
                            intent_id = %intent.intent_id,
                            class = ?class,
                            error = %error_msg,
                            "send returned ambiguous class; leaving row in Prepared for reconciler"
                        );
                        return Ok(BroadcastOutcome {
                            intent_id: intent.intent_id,
                            tx_hash,
                            nonce,
                            status: ExecutionIntentStatus::Prepared,
                            receipt_block_number: None,
                            error: Some(format!("ambiguous_send: {error_msg}")),
                        });
                    }
                    SendErrorClass::DeterministicReject => {
                        warn!(
                            intent_id = %intent.intent_id,
                            class = ?class,
                            error = %error_msg,
                            "send returned deterministic rejection; marking Failed"
                        );
                        repository
                            .mark_intent_failed(
                                intent.intent_id,
                                format!("deterministic_reject: {error_msg}"),
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
        }

        // Post-send: RPC either accepted the tx or returned an
        // idempotent replay class — advance to Submitted.
        repository
            .mark_intent_submitted(intent.intent_id, now_ms())
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
                // Semantic PME event verification. `receipt.status = 1`
                // alone is NOT sufficient — the tx must have emitted an
                // expected PME settlement event from the configured PME
                // address. Without this check a status=1 tx that
                // executed on a non-PME target could be mistaken for a
                // confirmed fill.
                if self.verify_pme_event {
                    if let Err(err) = verify_pme_event_in_receipt(
                        &receipt,
                        &self.config.perp_matching_engine_address,
                        &self.expected_pme_topic0,
                    ) {
                        let error = format!("semantic_event_verification: {err}");
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
                }
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

    /// Restart-safe reconciliation. For each intent whose durable
    /// broadcast has not yet converged (status ∈ {Prepared, Submitted}),
    /// either:
    ///
    /// * observe a receipt and finalize (Confirmed / Failed), or
    /// * for `Prepared` rows whose age exceeds
    ///   [`Self::rebroadcast_after_ms`] and whose `send_attempts` is
    ///   below [`Self::rebroadcast_max_attempts`], rebroadcast the
    ///   BYTE-IDENTICAL raw envelope stored at prepare time. Same
    ///   `raw_tx_hex` → same `keccak(raw_tx)` → same `tx_hash` → same
    ///   nonce → no duplicate economic execution.
    ///
    /// This is idempotent — repeated invocations converge without
    /// producing duplicate transactions or economic fills.
    pub async fn reconcile_unfinalized<Repo>(
        &self,
        repository: &Repo,
        max_intents: u32,
    ) -> Result<ReconcileSummary>
    where
        Repo: ExecutionIntentRepository,
    {
        let mut summary = ReconcileSummary::default();
        let candidates = repository.list_unfinalized_broadcasts(max_intents).await?;
        summary.inspected = candidates.len();
        for intent in &candidates {
            let Some(row) = repository.get_prepared_broadcast(intent.intent_id).await? else {
                summary.still_pending += 1;
                continue;
            };
            // Receipt-first: even a `Prepared` row may have been mined
            // by a prior lifecycle — we always check the chain before
            // rebroadcasting.
            match self.rpc.transaction_receipt(row.tx_hash.clone()).await? {
                Some(receipt) => {
                    let outcome = self
                        .finalize_receipt(
                            repository,
                            intent.intent_id,
                            &row.tx_hash,
                            row.nonce,
                            receipt,
                        )
                        .await?;
                    match outcome.status {
                        ExecutionIntentStatus::Confirmed => summary.confirmed += 1,
                        ExecutionIntentStatus::Failed => summary.failed += 1,
                        _ => summary.still_pending += 1,
                    }
                }
                None => {
                    if row.status == ExecutionIntentStatus::Prepared
                        && row.send_attempts < self.rebroadcast_max_attempts
                    {
                        // BYTE-IDENTICAL rebroadcast. No new nonce,
                        // no new envelope, no risk of a second fill.
                        summary.rebroadcast_attempted += 1;
                        let _ = repository
                            .bump_send_attempt(intent.intent_id, now_ms())
                            .await?;
                        match self.rpc.send_raw_transaction(row.raw_tx_hex.clone()).await {
                            Ok(_) | Err(_) => {
                                // Errors on rebroadcast are classified
                                // downstream by the next tick — we do
                                // not transition state on send errors
                                // here because the raw envelope is
                                // already persisted. Next reconcile
                                // pass re-checks receipt.
                            }
                        }
                    }
                    summary.still_pending += 1;
                }
            }
        }
        Ok(summary)
    }

    /// Backwards-compatible alias for the durable reconciler.
    pub async fn reconcile_submitted<Repo>(
        &self,
        repository: &Repo,
        max_intents: u32,
    ) -> Result<ReconcileSummary>
    where
        Repo: ExecutionIntentRepository,
    {
        self.reconcile_unfinalized(repository, max_intents).await
    }
}

/// Verify that a mined receipt contains at least one log emitted by
/// the configured PerpMatchingEngine with `topic0` matching the
/// canonical settlement event (or the intent-based variant). Returns
/// Ok on success; error message names the specific predicate that
/// failed for operator inspection.
///
/// PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_RUNTIME_WIRING_V1: closes the
/// "receipt.status = 1 but wrong contract executed" attack surface.
/// Even with a status=1 receipt whose `tx_hash` matches, the caller
/// MAY have been tricked into signing a tx that landed on a different
/// contract. This check refuses to mark Confirmed unless the log
/// stream demonstrably includes a PME settlement event.
pub fn verify_pme_event_in_receipt(
    receipt: &ConfirmationReceipt,
    expected_pme: &AccountId,
    expected_topic0: &[u8; 32],
) -> Result<()> {
    if receipt.logs.is_empty() {
        return Err(BackendError::BroadcastRejected(
            "receipt contains no logs — expected PME settlement event missing".to_string(),
        ));
    }
    let pme_lower = expected_pme
        .0
        .trim()
        .trim_start_matches("0x")
        .to_ascii_lowercase();
    let expected_topic_hex = format!("0x{}", hex_encode(expected_topic0));
    let alt_topic_hex = format!("0x{}", hex_encode(&PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0));
    for log in &receipt.logs {
        let addr_lower = log
            .address
            .trim()
            .trim_start_matches("0x")
            .to_ascii_lowercase();
        if addr_lower != pme_lower {
            continue;
        }
        let Some(topic0) = log.topics.first() else {
            continue;
        };
        let topic0_lower = topic0.trim().to_ascii_lowercase();
        if topic0_lower == expected_topic_hex || topic0_lower == alt_topic_hex {
            return Ok(());
        }
    }
    Err(BackendError::BroadcastRejected(format!(
        "no matching PME event in receipt (expected emitter {expected_pme:?} + topic0 {expected_topic_hex})",
    )))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

fn addresses_equal(a: &AccountId, b: &AccountId) -> bool {
    let left = a.0.trim().trim_start_matches("0x").to_ascii_lowercase();
    let right = b.0.trim().trim_start_matches("0x").to_ascii_lowercase();
    left == right
}

/// Validate the "signer identity triad" required at startup:
///
/// * `executor_from_address` — where `EXECUTOR_FROM_ADDRESS` /
///   `HybridV2ExecutionConfig.executor_address` resolve.
/// * `hv2_signer_expected_address` — the address configured via
///   `HV2_SIGNER_EXPECTED_ADDRESS` that the signer bridge asserts
///   against every KMS response.
/// * `hv2_executor_address` — the `HV2_EXECUTOR_ADDRESS` value that
///   feeds `HybridV2ExecutionConfig`.
///
/// All three MUST resolve to the same EOA. If any pair disagrees the
/// caller MUST fail startup — the broadcast worker must never sign
/// with an address that disagrees with the configured executor
/// identity. Case-insensitive; strips `0x` prefix; does not compare
/// checksum casing.
///
/// This function does NOT log the addresses on failure (they are
/// non-secret but noise); it returns them in the error message for
/// operator inspection.
pub fn validate_signer_triad(
    executor_from_address: &AccountId,
    hv2_signer_expected_address: &AccountId,
    hv2_executor_address: &AccountId,
) -> Result<()> {
    if !addresses_equal(executor_from_address, hv2_signer_expected_address) {
        return Err(BackendError::Config(format!(
            "signer identity triad disagreement: EXECUTOR_FROM_ADDRESS ({}) != HV2_SIGNER_EXPECTED_ADDRESS ({})",
            executor_from_address.0, hv2_signer_expected_address.0
        )));
    }
    if !addresses_equal(executor_from_address, hv2_executor_address) {
        return Err(BackendError::Config(format!(
            "signer identity triad disagreement: EXECUTOR_FROM_ADDRESS ({}) != HV2_EXECUTOR_ADDRESS ({})",
            executor_from_address.0, hv2_executor_address.0
        )));
    }
    Ok(())
}

/// Query `PerpMatchingEngine.intentFilled(bytes32 hash)` via
/// `eth_call`. Returns the cumulative fill amount in 1e8 units. A
/// non-zero return means the intent has been (at least partially)
/// filled on-chain; the backend can then refuse to submit a fresh
/// duplicate that would definitionally revert.
///
/// The on-chain contract's per-intent replay guard (`intentFilled`)
/// remains authoritative — this pre-check is an operational
/// optimization to avoid burning gas on guaranteed-revert txs.
pub async fn preflight_intent_filled<R>(
    rpc: &R,
    pme: &AccountId,
    intent_hash: &[u8; 32],
) -> Result<u128>
where
    R: EthCallProvider,
{
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&PME_INTENT_FILLED_SELECTOR);
    data.extend_from_slice(intent_hash);
    let out = rpc
        .eth_call(EthCallRequest {
            from: pme.clone(),
            to: pme.clone(),
            data,
            value: 0,
            gas_limit: None,
        })
        .await?;
    if out.output.len() != 32 {
        return Err(BackendError::BroadcastRejected(format!(
            "unexpected intentFilled return length: {}",
            out.output.len()
        )));
    }
    // Return value is a uint128; the top 16 bytes should be zero.
    let mut u128_bytes = [0u8; 16];
    u128_bytes.copy_from_slice(&out.output[16..32]);
    Ok(u128::from_be_bytes(u128_bytes))
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

/// Classification of `eth_sendRawTransaction` error messages so
/// downstream code can decide between:
/// * durable failure (deterministic reject),
/// * idempotent replay (already-known / nonce-too-low), or
/// * unknown submission outcome (ambiguous transport failure).
///
/// The classifier is deliberately conservative — anything that could
/// mean "the node may have accepted the tx even though the response
/// never made it back to us" is treated as `Ambiguous` so the raw
/// envelope stays persisted and the reconciler can decide.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendErrorClass {
    /// The tx is already in the node's mempool or on-chain — safe to
    /// fall through to receipt polling.
    AlreadyKnown,
    /// The nonce is behind the node's account state — usually because
    /// an earlier attempt succeeded. Safe to fall through to receipt
    /// polling.
    NonceTooLow,
    /// The node reports a replacement collision (same nonce, different
    /// gas). Treated as ambiguous because the earlier submission may
    /// still be pending.
    ReplacementUnderpriced,
    /// Deterministic RPC/protocol rejection — e.g. malformed tx,
    /// invalid signature, "insufficient funds". Never accepted by the
    /// node; safe to mark durable Failed.
    DeterministicReject,
    /// Transport-layer failure that MAY have followed the node's
    /// acceptance — timeouts, connection resets, proxy 502/504,
    /// malformed responses. NEVER mark Failed on this class; the
    /// reconciler will either observe a receipt or rebroadcast the
    /// byte-identical raw envelope.
    Ambiguous,
}

/// Classify an `eth_sendRawTransaction` error message. The
/// classification is used to decide whether the row transitions to
/// `Failed` (deterministic) or stays in `Prepared` (ambiguous /
/// idempotent).
pub fn classify_send_error(message: &str) -> SendErrorClass {
    let m = message.to_ascii_lowercase();
    if m.contains("already known") || m.contains("known transaction") {
        return SendErrorClass::AlreadyKnown;
    }
    if m.contains("nonce too low") {
        return SendErrorClass::NonceTooLow;
    }
    if m.contains("replacement transaction underpriced") || m.contains("replacement underpriced") {
        return SendErrorClass::ReplacementUnderpriced;
    }
    // Deterministic rejections — the tx cannot have been accepted.
    let deterministic = [
        "insufficient funds",
        "invalid signature",
        "invalid sender",
        "invalid tx",
        "invalid transaction",
        "malformed",
        "intrinsic gas too low",
        "gas price too low",
        "exceeds block gas limit",
        "transaction underpriced",
    ];
    for needle in deterministic {
        if m.contains(needle) {
            return SendErrorClass::DeterministicReject;
        }
    }
    // Everything else — including timeouts, resets, 502/504,
    // "connection reset", "eof", "response malformed" — is treated
    // as ambiguous. The reconciler resolves.
    SendErrorClass::Ambiguous
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
        // (tx_hash, nonce, raw_tx_hex, send_attempts, chain_id, executor, target)
        submitted_tx: Arc<Mutex<std::collections::HashMap<Uuid, PreparedBroadcastRow>>>,
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
                .map(|row| row.tx_hash.clone())
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

        fn send_attempts(&self, intent_id: Uuid) -> u32 {
            self.submitted_tx
                .lock()
                .unwrap()
                .get(&intent_id)
                .map(|r| r.send_attempts)
                .unwrap_or(0)
        }

        fn preset_prepared_row(&self, intent_id: Uuid, tx_hash: String, raw_tx_hex: String) {
            let row = PreparedBroadcastRow {
                intent_id,
                chain_id: 84532,
                executor_address: AccountId::new(String::from("0x00")),
                target_address: AccountId::new(String::from(PME)),
                tx_hash,
                nonce: 0,
                raw_tx_hex,
                status: ExecutionIntentStatus::Prepared,
                prepared_at_ms: 0,
                first_submission_at_ms: None,
                last_send_at_ms: None,
                send_attempts: 0,
                receipt_block_number: None,
                receipt_status: None,
                confirmed_at_ms: None,
                failure_class: None,
                failure_reason: None,
                failed_at_ms: None,
            };
            self.submitted_tx.lock().unwrap().insert(intent_id, row);
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
        fn record_prepared_transaction(
            &self,
            record: PreparedTransactionRecord,
        ) -> RepositoryFuture<'_, ()> {
            let row = PreparedBroadcastRow {
                intent_id: record.intent_id,
                chain_id: record.chain_id,
                executor_address: record.executor_address,
                target_address: record.target_address,
                tx_hash: record.tx_hash,
                nonce: record.nonce,
                raw_tx_hex: record.raw_tx_hex,
                status: ExecutionIntentStatus::Prepared,
                prepared_at_ms: record.prepared_at_ms,
                first_submission_at_ms: None,
                last_send_at_ms: None,
                send_attempts: 0,
                receipt_block_number: None,
                receipt_status: None,
                confirmed_at_ms: None,
                failure_class: None,
                failure_reason: None,
                failed_at_ms: None,
            };
            self.submitted_tx
                .lock()
                .unwrap()
                .insert(record.intent_id, row);
            Box::pin(async move { Ok(()) })
        }
        fn bump_send_attempt(
            &self,
            intent_id: Uuid,
            last_send_at_ms: TimestampMs,
        ) -> RepositoryFuture<'_, u32> {
            let attempts = {
                let mut store = self.submitted_tx.lock().unwrap();
                if let Some(row) = store.get_mut(&intent_id) {
                    row.send_attempts = row.send_attempts.saturating_add(1);
                    row.last_send_at_ms = Some(last_send_at_ms);
                    row.send_attempts
                } else {
                    0
                }
            };
            Box::pin(async move { Ok(attempts) })
        }
        fn get_prepared_broadcast(
            &self,
            intent_id: Uuid,
        ) -> RepositoryFuture<'_, Option<PreparedBroadcastRow>> {
            let row = self.submitted_tx.lock().unwrap().get(&intent_id).cloned();
            Box::pin(async move { Ok(row) })
        }
        fn list_unfinalized_broadcasts(
            &self,
            limit: u32,
        ) -> RepositoryFuture<'_, Vec<ExecutionIntent>> {
            let result = {
                let intents = self.intents.lock().unwrap();
                Ok(intents
                    .iter()
                    .filter(|i| {
                        i.status == ExecutionIntentStatus::Submitted
                            || i.status == ExecutionIntentStatus::Prepared
                    })
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
        fn with_send_err(self, e: &str) -> Self {
            *self.send_err.lock().unwrap() = Some(e.to_string());
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
            logs: Vec::new(),
        }
    }

    /// Receipt with a valid PME TradeExecuted event log — for tests
    /// that toggle `verify_pme_event = true`.
    fn happy_receipt_with_pme_event(tx_hash: &str) -> ConfirmationReceipt {
        let mut receipt = happy_receipt(tx_hash);
        receipt.logs.push(crate::confirmation::ReceiptLog {
            address: PME.to_ascii_lowercase(),
            topics: vec![format!("0x{}", hex_encode(&PME_TRADE_EXECUTED_TOPIC0))],
            data: "0x".to_string(),
        });
        receipt
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
            logs: Vec::new(),
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
        repo.preset_prepared_row(
            intent.intent_id,
            tx_hash.clone(),
            "0x02deadbeef".to_string(),
        );

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
        repo.preset_prepared_row(
            intent.intent_id,
            "0xdeadbeef".to_string(),
            "0x02".to_string(),
        );
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
            SendErrorClass::DeterministicReject
        );
        assert_eq!(
            classify_send_error("invalid signature"),
            SendErrorClass::DeterministicReject
        );
        // Transport-layer failures are Ambiguous — do NOT mark Failed.
        assert_eq!(
            classify_send_error("connection reset by peer"),
            SendErrorClass::Ambiguous
        );
        assert_eq!(
            classify_send_error("i/o error: timed out"),
            SendErrorClass::Ambiguous
        );
        assert_eq!(
            classify_send_error("upstream request timeout"),
            SendErrorClass::Ambiguous
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
        // intentFilled(bytes32) → 0x209905a5
        assert_eq!(PME_INTENT_FILLED_SELECTOR, [0x20, 0x99, 0x05, 0xa5]);
        // TradeExecuted event topic0 (first + last 4 bytes)
        assert_eq!(&PME_TRADE_EXECUTED_TOPIC0[..4], &[0x50, 0x18, 0xa0, 0xa7]);
        assert_eq!(&PME_TRADE_EXECUTED_TOPIC0[28..], &[0xfe, 0xdb, 0x3f, 0x80]);
    }

    // ---- durability-hardening tests ----

    // (N) ambiguous send failure → row stays in Prepared; no Failed
    // marking; caller sees Ambiguous outcome.
    #[tokio::test]
    async fn n_ambiguous_send_leaves_prepared() {
        let rpc = MockRpc::new().with_send_err("connection reset by peer");
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let signatures = make_signatures();
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Prepared);
        assert!(outcome.error.as_deref().unwrap().contains("ambiguous"));
        // The row is durably Prepared with a persisted raw_tx and tx_hash.
        assert_eq!(
            repo.status(intent.intent_id),
            ExecutionIntentStatus::Prepared
        );
        let row = repo
            .get_prepared_broadcast(intent.intent_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!row.raw_tx_hex.is_empty());
        assert_eq!(row.tx_hash, outcome.tx_hash);
        // send_attempts should be at least 1 (the ambiguous attempt counts).
        assert!(row.send_attempts >= 1);
    }

    // (O) deterministic reject → durable Failed.
    #[tokio::test]
    async fn o_deterministic_reject_marks_failed() {
        let rpc = MockRpc::new().with_send_err("insufficient funds for gas * price + value");
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let intent = make_intent();
        let signatures = make_signatures();
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Failed);
        assert!(outcome
            .error
            .as_deref()
            .unwrap()
            .contains("insufficient funds"));
        assert_eq!(repo.status(intent.intent_id), ExecutionIntentStatus::Failed);
    }

    // (P) reconcile rebroadcasts byte-identical raw tx for Prepared row
    // that never advanced. Same tx_hash preserved; no new nonce.
    #[tokio::test]
    async fn p_reconcile_rebroadcasts_byte_identical() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        // Simulate a crash: raw envelope + tx_hash persisted, receipt
        // never observed, RPC returns None on receipt polls.
        let rpc = MockRpc::new(); // no receipt
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.rebroadcast_max_attempts = 3;
        let repo = MockRepo::with(
            ExecutionIntent {
                status: ExecutionIntentStatus::Prepared,
                ..intent.clone()
            },
            signatures.clone(),
        );
        repo.preset_prepared_row(intent.intent_id, tx_hash.clone(), raw.clone());

        let summary = policy.reconcile_unfinalized(&repo, 10).await.unwrap();
        assert_eq!(summary.inspected, 1);
        assert_eq!(summary.confirmed, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.still_pending, 1);
        assert_eq!(summary.rebroadcast_attempted, 1);
        // Prepared row is preserved (no new tx built).
        let row = repo
            .get_prepared_broadcast(intent.intent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.tx_hash, tx_hash);
        assert_eq!(row.raw_tx_hex, raw);
        assert_eq!(row.nonce, 0);
    }

    // (Q) reconcile bounded by rebroadcast_max_attempts.
    #[tokio::test]
    async fn q_reconcile_respects_attempt_cap() {
        let rpc = MockRpc::new();
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.rebroadcast_max_attempts = 2;
        let intent = make_intent();
        let repo = MockRepo::with(
            ExecutionIntent {
                status: ExecutionIntentStatus::Prepared,
                ..intent.clone()
            },
            make_signatures(),
        );
        repo.preset_prepared_row(
            intent.intent_id,
            "0xdead".to_string(),
            "0x02deadbeef".to_string(),
        );
        // Manually bump attempts past the cap.
        for _ in 0..3 {
            let _ = repo.bump_send_attempt(intent.intent_id, 0).await.unwrap();
        }
        assert!(repo.send_attempts(intent.intent_id) >= policy.rebroadcast_max_attempts);
        let summary = policy.reconcile_unfinalized(&repo, 10).await.unwrap();
        assert_eq!(summary.rebroadcast_attempted, 0);
        assert_eq!(summary.still_pending, 1);
    }

    // (R) reconcile: prior send succeeded but reconciler had no
    // knowledge → observes receipt → Confirmed. No rebroadcast.
    #[tokio::test]
    async fn r_reconcile_confirms_when_receipt_present() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();
        let rpc = MockRpc::new().with_receipt(happy_receipt(&tx_hash));
        let policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        let repo = MockRepo::with(
            ExecutionIntent {
                status: ExecutionIntentStatus::Prepared,
                ..intent.clone()
            },
            signatures.clone(),
        );
        repo.preset_prepared_row(intent.intent_id, tx_hash.clone(), raw.clone());

        let summary = policy.reconcile_unfinalized(&repo, 10).await.unwrap();
        assert_eq!(summary.confirmed, 1);
        assert_eq!(summary.rebroadcast_attempted, 0);
        assert_eq!(
            repo.status(intent.intent_id),
            ExecutionIntentStatus::Confirmed
        );
    }

    // (S) signer triad validation
    #[test]
    fn s_signer_triad_all_equal_passes() {
        let addr = AccountId::new("0xabc0000000000000000000000000000000000001".to_string());
        validate_signer_triad(&addr, &addr, &addr).unwrap();
    }

    #[test]
    fn s_signer_triad_case_insensitive_passes() {
        let a = AccountId::new("0xABC0000000000000000000000000000000000001".to_string());
        let b = AccountId::new("0xabc0000000000000000000000000000000000001".to_string());
        let c = AccountId::new("0xAbC0000000000000000000000000000000000001".to_string());
        validate_signer_triad(&a, &b, &c).unwrap();
    }

    #[test]
    fn s_signer_triad_mismatch_fails_closed() {
        let a = AccountId::new("0xaaa0000000000000000000000000000000000001".to_string());
        let b = AccountId::new("0xbbb0000000000000000000000000000000000002".to_string());
        let c = AccountId::new("0xaaa0000000000000000000000000000000000001".to_string());
        let err = validate_signer_triad(&a, &b, &c).unwrap_err();
        assert!(matches!(err, BackendError::Config(msg) if msg.contains("triad disagreement")));
    }

    // ---- semantic PME event verification (T series) ----

    // (T1) status=1 + expected event => Confirmed.
    #[tokio::test]
    async fn t1_status1_with_pme_event_confirms() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let rpc = MockRpc::new().with_receipt(happy_receipt_with_pme_event(&tx_hash));
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.verify_pme_event = true;
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Confirmed);
    }

    // (T2) status=1 + no PME event => NOT Confirmed.
    #[tokio::test]
    async fn t2_status1_missing_pme_event_fails() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        // happy_receipt has empty logs — event verification must fail
        let rpc = MockRpc::new().with_receipt(happy_receipt(&tx_hash));
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.verify_pme_event = true;
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Failed);
        assert!(outcome
            .error
            .as_deref()
            .unwrap()
            .contains("semantic_event_verification"));
    }

    // (T3) status=1 + event from wrong emitter => NOT Confirmed.
    #[tokio::test]
    async fn t3_status1_wrong_emitter_fails() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let mut r = happy_receipt(&tx_hash);
        r.logs.push(crate::confirmation::ReceiptLog {
            address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            topics: vec![format!("0x{}", hex_encode(&PME_TRADE_EXECUTED_TOPIC0))],
            data: "0x".to_string(),
        });
        let rpc = MockRpc::new().with_receipt(r);
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.verify_pme_event = true;
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Failed);
    }

    // (T4) status=1 + wrong topic0 => NOT Confirmed.
    #[tokio::test]
    async fn t4_status1_wrong_topic0_fails() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let mut r = happy_receipt(&tx_hash);
        r.logs.push(crate::confirmation::ReceiptLog {
            address: PME.to_ascii_lowercase(),
            topics: vec![
                "0x0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            ],
            data: "0x".to_string(),
        });
        let rpc = MockRpc::new().with_receipt(r);
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.verify_pme_event = true;
        let repo = MockRepo::with(intent.clone(), signatures.clone());

        let outcome = policy
            .broadcast_intent(&repo, &intent, &signatures)
            .await
            .unwrap();
        assert_eq!(outcome.status, ExecutionIntentStatus::Failed);
    }

    // (T5) receipt appears only after restart (reconciler path)
    #[tokio::test]
    async fn t5_reconciler_confirms_when_event_present() {
        let config = make_config(TEST_KEY_ADDRESS, true, false);
        let signer = make_signer();
        let intent = make_intent();
        let signatures = make_signatures();
        let request = build_execution_transaction_request(&config, &intent, &signatures).unwrap();
        let raw = sign_transaction(&request, 0, signer.as_ref()).unwrap();
        let tx_hash = derive_signed_transaction_hash(&raw).unwrap();

        let rpc = MockRpc::new().with_receipt(happy_receipt_with_pme_event(&tx_hash));
        let mut policy = make_policy_with(rpc, TEST_KEY_ADDRESS);
        policy.verify_pme_event = true;
        let repo = MockRepo::with(
            ExecutionIntent {
                status: ExecutionIntentStatus::Prepared,
                ..intent.clone()
            },
            signatures.clone(),
        );
        repo.preset_prepared_row(intent.intent_id, tx_hash.clone(), raw.clone());

        let summary = policy.reconcile_unfinalized(&repo, 10).await.unwrap();
        assert_eq!(summary.confirmed, 1);
        assert_eq!(
            repo.status(intent.intent_id),
            ExecutionIntentStatus::Confirmed
        );
    }

    #[test]
    fn s_signer_triad_hv2_executor_mismatch_fails() {
        let a = AccountId::new("0xaaa0000000000000000000000000000000000001".to_string());
        let b = AccountId::new("0xaaa0000000000000000000000000000000000001".to_string());
        let c = AccountId::new("0xccc0000000000000000000000000000000000003".to_string());
        let err = validate_signer_triad(&a, &b, &c).unwrap_err();
        assert!(matches!(err, BackendError::Config(msg) if msg.contains("HV2_EXECUTOR_ADDRESS")));
    }
}
