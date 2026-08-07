//! Read-only pre-broadcast simulation for Hybrid V2 execution plans
//! (Part I of `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! ## Frozen safety
//!
//! The simulator is strictly read-only: it emits `eth_chainId`,
//! `eth_getBlockByNumber(latest)`, `eth_call`, and `eth_estimateGas`
//! only, via the narrowed [`ExecutionRpcClient`] trait. That trait has
//! no send / sign / personal method, so the simulator cannot broadcast
//! by construction (compile-time firewall — see
//! `src/hybrid_v2/execution/rpc.rs`).
//!
//! Behavior contract:
//! 1. Read `chain_id()` and verify it equals `plan.chain_id`. Fail
//!    with `ChainMismatch` otherwise.
//! 2. Snapshot head block number + hash.
//! 3. Bind `eth_call { to = plan.target, data = plan.calldata,
//!    from = executor_address, value = plan.value_wei }` to the
//!    snapshotted block number. NO state override.
//! 4. On success → `estimate_gas` at the same block; return
//!    `SimulationOutcome { eth_call_ok = true, ... }`.
//! 5. On revert → decode via `DecodedRevert::decode`; return
//!    `eth_call_ok = false` with the decoded revert.
//! 6. Never retries a revert (revert is deterministic for the same
//!    plan + block). Transport failures allowed one retry.
//! 7. Wall-clock deadline enforced via `tokio::time::timeout` per
//!    round-trip.

use crate::hybrid_v2::execution::plan::ExecutionPlan;
use crate::hybrid_v2::execution::rpc::{
    BlockTag, DecodedRevert, EthCallOutcome, EthCallRequest, ExecutionRpcClient, ExecutionRpcError,
};
use std::time::Duration;
use thiserror::Error;

/// Read-only pre-broadcast simulator. Borrows its RPC client so the
/// same client can be shared across many simulations.
pub struct ExecutionSimulator<'a> {
    pub rpc: &'a dyn ExecutionRpcClient,
    pub executor_address: [u8; 20],
    /// Per-round-trip deadline, applied via `tokio::time::timeout`.
    /// The trait's own timeout applies too; whichever fires first wins.
    pub deadline_ms: u64,
}

/// Successful simulation result. Populated even on revert — the caller
/// records the decoded revert alongside the block context so operators
/// can grep for a deterministic reason.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationOutcome {
    pub simulation_block_number: u64,
    pub simulation_block_hash: [u8; 32],
    pub gas_estimate: u64,
    pub eth_call_ok: bool,
    pub revert: Option<DecodedRevert>,
    pub simulated_at_ms: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SimulationError {
    #[error("chain id mismatch: expected {expected} actual {actual}")]
    ChainMismatch { expected: u64, actual: u64 },
    #[error("rpc failure: {0}")]
    RpcFailure(String),
    #[error("malformed response: {0}")]
    MalformedResponse(String),
    #[error("head block moved between chain-id probe and eth_call")]
    StaleBlock,
    #[error("simulation timed out")]
    Timeout,
}

impl<'a> ExecutionSimulator<'a> {
    pub fn new(rpc: &'a dyn ExecutionRpcClient, executor_address: [u8; 20]) -> Self {
        Self {
            rpc,
            executor_address,
            deadline_ms: 2_500,
        }
    }

    pub async fn simulate(
        &self,
        plan: &ExecutionPlan,
    ) -> Result<SimulationOutcome, SimulationError> {
        // 1. chain_id check.
        let observed_chain = self.timed(self.rpc.chain_id()).await??;
        if observed_chain != plan.chain_id {
            return Err(SimulationError::ChainMismatch {
                expected: plan.chain_id,
                actual: observed_chain,
            });
        }

        // 2. head block snapshot.
        let block_number = self.timed(self.rpc.head_block_number()).await??;
        let block_hash = self.timed(self.rpc.head_block_hash()).await??;

        // 3. eth_call bound to that block.
        let req = EthCallRequest {
            from: Some(self.executor_address),
            to: plan.target,
            data: plan.calldata.clone(),
            value_wei: plan.value_wei,
            gas: None,
        };

        // Transport retries only — a revert is not retried.
        let outcome = {
            let mut last_transport_err: Option<ExecutionRpcError> = None;
            let mut attempt = 0u8;
            loop {
                match self
                    .timed(
                        self.rpc
                            .eth_call(req.clone(), BlockTag::Number(block_number)),
                    )
                    .await?
                {
                    Ok(v) => break Ok::<_, SimulationError>(v),
                    Err(err) if is_transport(&err) && attempt == 0 => {
                        last_transport_err = Some(err);
                        attempt += 1;
                        continue;
                    }
                    Err(err) => {
                        return Err(classify_rpc_error(err.clone(), last_transport_err));
                    }
                }
            }
        }?;

        let now_ms = wall_ms();
        match outcome {
            EthCallOutcome::Success(_) => {
                let gas = self
                    .timed(
                        self.rpc
                            .estimate_gas(req.clone(), BlockTag::Number(block_number)),
                    )
                    .await??;
                Ok(SimulationOutcome {
                    simulation_block_number: block_number,
                    simulation_block_hash: block_hash,
                    gas_estimate: gas,
                    eth_call_ok: true,
                    revert: None,
                    simulated_at_ms: now_ms,
                })
            }
            EthCallOutcome::Revert { raw_data, decoded } => Ok(SimulationOutcome {
                simulation_block_number: block_number,
                simulation_block_hash: block_hash,
                // gas_estimate is 0 on revert — the plan will not
                // proceed and downstream ignores this field when
                // eth_call_ok=false. Storing 0 is an unambiguous
                // sentinel for the persistence layer.
                gas_estimate: 0,
                eth_call_ok: false,
                revert: decoded.or_else(|| DecodedRevert::decode(&raw_data)),
                simulated_at_ms: now_ms,
            }),
            EthCallOutcome::Panic { code } => Ok(SimulationOutcome {
                simulation_block_number: block_number,
                simulation_block_hash: block_hash,
                gas_estimate: 0,
                eth_call_ok: false,
                revert: Some(DecodedRevert::Panic { code }),
                simulated_at_ms: now_ms,
            }),
            EthCallOutcome::MalformedResponse(msg) => Err(SimulationError::MalformedResponse(msg)),
        }
    }

    async fn timed<F, T>(&self, fut: F) -> Result<T, SimulationError>
    where
        F: std::future::Future<Output = T>,
    {
        match tokio::time::timeout(Duration::from_millis(self.deadline_ms), fut).await {
            Ok(v) => Ok(v),
            Err(_) => Err(SimulationError::Timeout),
        }
    }
}

fn classify_rpc_error(
    err: ExecutionRpcError,
    _prior_transport: Option<ExecutionRpcError>,
) -> SimulationError {
    match err {
        ExecutionRpcError::ChainMismatch { expected, actual } => {
            SimulationError::ChainMismatch { expected, actual }
        }
        ExecutionRpcError::Malformed(msg) => SimulationError::MalformedResponse(msg),
        ExecutionRpcError::Timeout => SimulationError::Timeout,
        other => SimulationError::RpcFailure(other.to_string()),
    }
}

fn is_transport(err: &ExecutionRpcError) -> bool {
    matches!(
        err,
        ExecutionRpcError::Transport(_)
            | ExecutionRpcError::RateLimited
            | ExecutionRpcError::ServerError { .. }
    )
}

impl From<ExecutionRpcError> for SimulationError {
    fn from(e: ExecutionRpcError) -> Self {
        classify_rpc_error(e, None)
    }
}

fn wall_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// -----------------------------------------------------------------
//                          UNIT TESTS
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hybrid_v2::execution::plan::ExecutionPlan;
    use crate::hybrid_v2::execution::rpc::{FeeHistory, KNOWN_CUSTOM_ERROR_SELECTORS};
    use crate::hybrid_v2::execution::CanonicalExecutionId;
    use alloy_primitives::U256;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Minimal in-process fake ExecutionRpcClient for unit tests. Every
    /// method routes through an `Instruction` queue so tests can stage
    /// arbitrary success / revert / transport-failure sequences.
    enum Instruction {
        Ok(EthCallOutcome),
        Err(ExecutionRpcError),
    }

    struct FakeRpc {
        chain_id: u64,
        block_number: u64,
        block_hash: [u8; 32],
        gas: u64,
        // Queue of eth_call outcomes.
        call_queue: Mutex<Vec<Instruction>>,
    }

    impl FakeRpc {
        fn ok(chain_id: u64, outcome: EthCallOutcome) -> Self {
            Self {
                chain_id,
                block_number: 100,
                block_hash: [0xab; 32],
                gas: 90_000,
                call_queue: Mutex::new(vec![Instruction::Ok(outcome)]),
            }
        }
    }

    #[async_trait]
    impl ExecutionRpcClient for FakeRpc {
        async fn chain_id(&self) -> Result<u64, ExecutionRpcError> {
            Ok(self.chain_id)
        }
        async fn head_block_number(&self) -> Result<u64, ExecutionRpcError> {
            Ok(self.block_number)
        }
        async fn head_block_hash(&self) -> Result<[u8; 32], ExecutionRpcError> {
            Ok(self.block_hash)
        }
        async fn transaction_count(
            &self,
            _: [u8; 20],
            _: BlockTag,
        ) -> Result<u64, ExecutionRpcError> {
            Ok(0)
        }
        async fn eth_call(
            &self,
            _req: EthCallRequest,
            _tag: BlockTag,
        ) -> Result<EthCallOutcome, ExecutionRpcError> {
            let mut q = self.call_queue.lock().unwrap();
            if q.is_empty() {
                return Err(ExecutionRpcError::Transport("empty queue".into()));
            }
            match q.remove(0) {
                Instruction::Ok(o) => Ok(o),
                Instruction::Err(e) => Err(e),
            }
        }
        async fn estimate_gas(
            &self,
            _req: EthCallRequest,
            _tag: BlockTag,
        ) -> Result<u64, ExecutionRpcError> {
            Ok(self.gas)
        }
        async fn fee_history(
            &self,
            _n: u64,
            _tag: BlockTag,
            _p: &[f64],
        ) -> Result<FeeHistory, ExecutionRpcError> {
            Ok(FeeHistory {
                oldest_block: 0,
                base_fee_per_gas: vec![],
                reward: vec![],
                gas_used_ratio: vec![],
            })
        }
    }

    fn dummy_plan(chain_id: u64) -> ExecutionPlan {
        ExecutionPlan {
            canonical_execution_id: CanonicalExecutionId(format!("0x{}", "aa".repeat(32))),
            chain_id,
            deployment_id: 1,
            target: [0xcc; 20],
            selector: [0x12, 0x34, 0x56, 0x78],
            calldata: vec![0x12, 0x34, 0x56, 0x78, 0x00, 0x00],
            calldata_hash: [0xdd; 32],
            value_wei: U256::ZERO,
            expected_module_version: "OptionMatchingEngineV2".into(),
            deadline_ms: None,
            plan_hash: [0xee; 32],
        }
    }

    #[tokio::test]
    async fn happy_path_success_simulation() {
        let rpc = FakeRpc::ok(84532, EthCallOutcome::Success(vec![]));
        let sim = ExecutionSimulator::new(&rpc, [0x01; 20]);
        let out = sim.simulate(&dummy_plan(84532)).await.unwrap();
        assert!(out.eth_call_ok);
        assert_eq!(out.gas_estimate, 90_000);
        assert_eq!(out.simulation_block_number, 100);
        assert!(out.revert.is_none());
    }

    #[tokio::test]
    async fn chain_mismatch_rejected() {
        let rpc = FakeRpc::ok(11155111, EthCallOutcome::Success(vec![]));
        let sim = ExecutionSimulator::new(&rpc, [0x01; 20]);
        let err = sim.simulate(&dummy_plan(84532)).await.unwrap_err();
        assert!(matches!(err, SimulationError::ChainMismatch { .. }));
    }

    #[tokio::test]
    async fn revert_decoded_for_each_known_selector() {
        for (sel, name) in KNOWN_CUSTOM_ERROR_SELECTORS {
            let rpc = FakeRpc::ok(
                84532,
                EthCallOutcome::Revert {
                    raw_data: sel.to_vec(),
                    decoded: None,
                },
            );
            let sim = ExecutionSimulator::new(&rpc, [0x01; 20]);
            let out = sim.simulate(&dummy_plan(84532)).await.unwrap();
            assert!(!out.eth_call_ok, "must be revert for {name}");
            match out.revert {
                Some(DecodedRevert::Custom { selector, name: n }) => {
                    assert_eq!(&selector, sel);
                    assert_eq!(n, *name);
                }
                other => panic!("expected Custom revert for {name}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn malformed_response_returns_hard_error() {
        let rpc = FakeRpc {
            chain_id: 84532,
            block_number: 1,
            block_hash: [0; 32],
            gas: 0,
            call_queue: Mutex::new(vec![Instruction::Ok(EthCallOutcome::MalformedResponse(
                "garbage".into(),
            ))]),
        };
        let sim = ExecutionSimulator::new(&rpc, [0x01; 20]);
        let err = sim.simulate(&dummy_plan(84532)).await.unwrap_err();
        assert!(matches!(err, SimulationError::MalformedResponse(_)));
    }

    #[tokio::test]
    async fn transport_failure_retried_once() {
        // First eth_call yields transport failure, second succeeds.
        let rpc = FakeRpc {
            chain_id: 84532,
            block_number: 100,
            block_hash: [0; 32],
            gas: 42,
            call_queue: Mutex::new(vec![
                Instruction::Err(ExecutionRpcError::Transport("blip".into())),
                Instruction::Ok(EthCallOutcome::Success(vec![])),
            ]),
        };
        let sim = ExecutionSimulator::new(&rpc, [0x01; 20]);
        let out = sim.simulate(&dummy_plan(84532)).await.unwrap();
        assert!(out.eth_call_ok);
        assert_eq!(out.gas_estimate, 42);
    }
}
