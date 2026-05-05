use crate::error::Result;
use crate::execution::revert::{DecodedRevertError, RevertDiagnostics};
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::execution::{
    build_perp_execution_call_from_intent, ExecutionConfig, ExecutionIntent, ExecutionIntentStatus,
    PreparedExecutionCall, StoredTradeSignatures,
};
use crate::types::{now_ms, TimestampMs};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SimulationResult {
    pub simulation_id: Uuid,
    pub intent_id: Uuid,
    pub status: ExecutionIntentStatus,
    pub block_number: Option<u64>,
    pub error: Option<String>,
    pub revert_data: Option<String>,
    pub revert_selector: Option<String>,
    pub decoded_error: Option<DecodedRevertError>,
    pub created_at_ms: TimestampMs,
}

impl SimulationResult {
    pub fn is_ok(&self) -> bool {
        self.status == ExecutionIntentStatus::SimulationOk
    }
}

pub async fn simulate_execution_intent<P>(
    provider: &P,
    config: &ExecutionConfig,
    intent: &ExecutionIntent,
    signatures: &StoredTradeSignatures,
) -> Result<SimulationResult>
where
    P: EthCallProvider,
{
    let call = build_perp_execution_call_from_intent(
        intent,
        &config.perp_matching_engine_address,
        signatures,
    )?;
    simulate_prepared_call(provider, &config.executor_from_address, call).await
}

pub async fn simulate_prepared_call<P>(
    provider: &P,
    from: &crate::types::AccountId,
    call: PreparedExecutionCall,
) -> Result<SimulationResult>
where
    P: EthCallProvider,
{
    if call.missing_signatures {
        return Err(crate::error::BackendError::MissingTradeSignatures);
    }

    let request = EthCallRequest {
        from: from.clone(),
        to: call.target.clone(),
        data: call.calldata,
        value: call.value,
    };
    let created_at_ms = now_ms();
    match provider.eth_call(request).await {
        Ok(success) => Ok(SimulationResult {
            simulation_id: Uuid::new_v4(),
            intent_id: call.intent_id,
            status: ExecutionIntentStatus::SimulationOk,
            block_number: success.block_number,
            error: None,
            revert_data: None,
            revert_selector: None,
            decoded_error: None,
            created_at_ms,
        }),
        Err(error) => {
            let diagnostics = diagnostics_from_backend_error(&error);
            Ok(SimulationResult {
                simulation_id: Uuid::new_v4(),
                intent_id: call.intent_id,
                status: ExecutionIntentStatus::SimulationFailed,
                block_number: None,
                error: Some(error.to_string()),
                revert_data: diagnostics.revert_data,
                revert_selector: diagnostics.revert_selector,
                decoded_error: Some(diagnostics.decoded_error),
                created_at_ms,
            })
        }
    }
}

fn diagnostics_from_backend_error(error: &crate::error::BackendError) -> RevertDiagnostics {
    match error {
        crate::error::BackendError::SimulationReverted(diagnostics) => diagnostics.as_ref().clone(),
        other => RevertDiagnostics::missing(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BackendError;
    use crate::execution::rpc::{EthCallSuccess, RpcFuture};
    use crate::execution::{PerpTradeSignatureBundle, StoredTradeSignatures};
    use crate::types::{AccountId, OrderId};

    #[derive(Clone)]
    enum MockOutcome {
        Success,
        Failure(&'static str),
        Revert(RevertDiagnostics),
    }

    #[derive(Clone)]
    struct MockProvider {
        outcome: MockOutcome,
    }

    impl EthCallProvider for MockProvider {
        fn eth_call(&self, _request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let outcome = self.outcome.clone();
            Box::pin(async move {
                match outcome {
                    MockOutcome::Success => Ok(EthCallSuccess {
                        block_number: Some(123),
                        output: Vec::new(),
                    }),
                    MockOutcome::Failure(message) => {
                        Err(BackendError::Simulation(message.to_string()))
                    }
                    MockOutcome::Revert(diagnostics) => {
                        Err(BackendError::SimulationReverted(Box::new(diagnostics)))
                    }
                }
            })
        }
    }

    #[tokio::test]
    async fn simulator_maps_success_to_simulation_ok() {
        let result = simulate_execution_intent(
            &MockProvider {
                outcome: MockOutcome::Success,
            },
            &config(),
            &intent(),
            &signatures(),
        )
        .await
        .unwrap();

        assert_eq!(result.status, ExecutionIntentStatus::SimulationOk);
        assert_eq!(result.block_number, Some(123));
        assert_eq!(result.error, None);
        assert_eq!(result.revert_data, None);
        assert_eq!(result.revert_selector, None);
        assert_eq!(result.decoded_error, None);
    }

    #[tokio::test]
    async fn simulator_maps_failure_to_simulation_failed() {
        let result = simulate_execution_intent(
            &MockProvider {
                outcome: MockOutcome::Failure("execution reverted"),
            },
            &config(),
            &intent(),
            &signatures(),
        )
        .await
        .unwrap();

        assert_eq!(result.status, ExecutionIntentStatus::SimulationFailed);
        assert!(result.error.unwrap().contains("execution reverted"));
        assert_eq!(
            result.decoded_error.unwrap().kind,
            "missing_revert_data".to_string()
        );
    }

    #[tokio::test]
    async fn simulator_maps_revert_diagnostics_to_simulation_failed() {
        let result = simulate_execution_intent(
            &MockProvider {
                outcome: MockOutcome::Revert(RevertDiagnostics {
                    raw_error: "execution reverted".to_string(),
                    revert_data: Some("0x12345678".to_string()),
                    revert_selector: Some("0x12345678".to_string()),
                    decoded_error: crate::execution::revert::DecodedRevertError {
                        kind: "unknown_custom_error".to_string(),
                        name: None,
                        selector: Some("0x12345678".to_string()),
                        args: None,
                        decoded: None,
                    },
                }),
            },
            &config(),
            &intent(),
            &signatures(),
        )
        .await
        .unwrap();

        assert_eq!(result.status, ExecutionIntentStatus::SimulationFailed);
        assert_eq!(result.revert_data.as_deref(), Some("0x12345678"));
        assert_eq!(result.revert_selector.as_deref(), Some("0x12345678"));
        assert_eq!(
            result.decoded_error.unwrap().kind,
            "unknown_custom_error".to_string()
        );
    }

    #[tokio::test]
    async fn simulator_rejects_missing_signatures_before_rpc() {
        let result = simulate_execution_intent(
            &MockProvider {
                outcome: MockOutcome::Success,
            },
            &config(),
            &intent(),
            &StoredTradeSignatures::default(),
        )
        .await
        .unwrap_err();

        assert!(matches!(result, BackendError::MissingTradeSignatures));
    }

    fn config() -> ExecutionConfig {
        ExecutionConfig {
            execution_enabled: false,
            dry_run: true,
            poll_interval_ms: 1_000,
            max_batch_size: 10,
            real_broadcast_enabled: false,
            executor_private_key: None,
            executor_chain_id: 84532,
            max_gas_limit: 1_000_000,
            max_fee_per_gas_wei: None,
            max_priority_fee_per_gas_wei: None,
            require_simulation_ok: true,
            simulation_enabled: true,
            simulation_requires_persistence: false,
            rpc_url: Some("https://example.invalid".to_string()),
            executor_from_address: AccountId::new("0x0000000000000000000000000000000000000000"),
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000009",
            ),
            perp_engine_address: AccountId::new("0x0000000000000000000000000000000000000000"),
        }
    }

    fn intent() -> ExecutionIntent {
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
            status: ExecutionIntentStatus::CalldataReady,
        }
    }

    fn signatures() -> StoredTradeSignatures {
        let bundle =
            PerpTradeSignatureBundle::new(&signature_hex(0xaa), &signature_hex(0xbb)).unwrap();
        StoredTradeSignatures {
            buyer_sig: Some(hex_0x(&bundle.buyer_sig)),
            seller_sig: Some(hex_0x(&bundle.seller_sig)),
        }
    }

    fn signature_hex(byte: u8) -> String {
        let mut signature = String::from("0x");
        for _ in 0..65 {
            signature.push_str(&format!("{byte:02x}"));
        }
        signature
    }

    fn hex_0x(bytes: &[u8]) -> String {
        let mut encoded = String::from("0x");
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        encoded
    }
}
