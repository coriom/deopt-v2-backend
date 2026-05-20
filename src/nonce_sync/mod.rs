use crate::error::{BackendError, Result};
use crate::execution::{EthCallProvider, EthCallRequest, RpcFuture};
use crate::signing::eip712::{keccak256, parse_evm_address};
use crate::types::AccountId;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerpNonceSyncConfig {
    pub enabled: bool,
    pub require_rpc: bool,
    pub strict: bool,
    pub rpc_url: Option<String>,
    pub perp_matching_engine_address: AccountId,
}

impl PerpNonceSyncConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_rpc: true,
            strict: true,
            rpc_url: None,
            perp_matching_engine_address: AccountId::new(
                "0x0000000000000000000000000000000000000000",
            ),
        }
    }

    pub fn validate_startup(&self) -> Result<()> {
        if !self.enabled || !self.require_rpc {
            return Ok(());
        }
        ensure_rpc_url_configured(self)?;
        ensure_matching_engine_configured(self)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PerpNonceResponse {
    pub account: String,
    pub perp_matching_engine: String,
    pub nonce: u64,
    pub source: &'static str,
}

pub trait PerpNonceProvider: Clone + Send + Sync {
    fn perp_matching_nonce(
        &self,
        matching_engine: AccountId,
        account: AccountId,
    ) -> RpcFuture<'_, u64>;
}

impl<T> PerpNonceProvider for T
where
    T: EthCallProvider + Clone + Send + Sync,
{
    fn perp_matching_nonce(
        &self,
        matching_engine: AccountId,
        account: AccountId,
    ) -> RpcFuture<'_, u64> {
        Box::pin(async move {
            let data = encode_nonces_call(&account)?;
            let output = self
                .eth_call(EthCallRequest {
                    from: zero_address(),
                    to: matching_engine,
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?
                .output;
            decode_uint256_to_u64(&output)
        })
    }
}

pub async fn read_perp_nonce<P>(
    config: &PerpNonceSyncConfig,
    provider: &P,
    account: &AccountId,
) -> Result<PerpNonceResponse>
where
    P: PerpNonceProvider,
{
    ensure_enabled(config)?;
    ensure_ready_for_read(config)?;
    parse_evm_address(account)?;
    let nonce = provider
        .perp_matching_nonce(config.perp_matching_engine_address.clone(), account.clone())
        .await?;
    Ok(PerpNonceResponse {
        account: account.0.to_ascii_lowercase(),
        perp_matching_engine: config.perp_matching_engine_address.0.to_ascii_lowercase(),
        nonce,
        source: "onchain",
    })
}

pub async fn validate_order_perp_nonce<P>(
    config: &PerpNonceSyncConfig,
    provider: &P,
    account: &AccountId,
    order_nonce: u64,
) -> Result<()>
where
    P: PerpNonceProvider,
{
    if !config.enabled || !config.strict {
        return Ok(());
    }
    let response = read_perp_nonce(config, provider, account).await?;
    if order_nonce != response.nonce {
        return Err(BackendError::PerpNonceMismatch {
            expected: response.nonce,
            got: order_nonce,
        });
    }
    Ok(())
}

fn ensure_enabled(config: &PerpNonceSyncConfig) -> Result<()> {
    if config.enabled {
        Ok(())
    } else {
        Err(BackendError::PerpNonceSyncDisabled)
    }
}

fn ensure_ready_for_read(config: &PerpNonceSyncConfig) -> Result<()> {
    ensure_rpc_url_configured(config)?;
    ensure_matching_engine_configured(config)?;
    Ok(())
}

fn ensure_rpc_url_configured(config: &PerpNonceSyncConfig) -> Result<()> {
    if config.rpc_url.is_some() {
        Ok(())
    } else {
        Err(BackendError::Config(
            "RPC_URL is required for perp nonce sync".to_string(),
        ))
    }
}

fn ensure_matching_engine_configured(config: &PerpNonceSyncConfig) -> Result<()> {
    parse_evm_address(&config.perp_matching_engine_address).map_err(|_| {
        BackendError::Config(
            "PERP_MATCHING_ENGINE_ADDRESS must be a valid EVM address for perp nonce sync"
                .to_string(),
        )
    })?;
    if config
        .perp_matching_engine_address
        .0
        .eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return Err(BackendError::Config(
            "PERP_MATCHING_ENGINE_ADDRESS is required for perp nonce sync".to_string(),
        ));
    }
    Ok(())
}

fn encode_nonces_call(account: &AccountId) -> Result<Vec<u8>> {
    let address = parse_evm_address(account)?;
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&nonces_selector());
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(&address);
    Ok(calldata)
}

fn nonces_selector() -> [u8; 4] {
    let hash = keccak256(b"nonces(address)");
    [hash[0], hash[1], hash[2], hash[3]]
}

fn decode_uint256_to_u64(output: &[u8]) -> Result<u64> {
    if output.len() != 32 {
        return Err(BackendError::Simulation(
            "PerpMatchingEngine.nonces returned invalid uint256 output".to_string(),
        ));
    }
    if output[..24].iter().any(|byte| *byte != 0) {
        return Err(BackendError::Simulation(
            "PerpMatchingEngine.nonces value exceeds u64".to_string(),
        ));
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&output[24..32]);
    Ok(u64::from_be_bytes(bytes))
}

fn zero_address() -> AccountId {
    AccountId::new("0x0000000000000000000000000000000000000000")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::NonceStore;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[derive(Clone)]
    struct MockNonceProvider {
        nonce: u64,
        calls: Arc<AtomicUsize>,
    }

    impl MockNonceProvider {
        fn new(nonce: u64) -> Self {
            Self {
                nonce,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl PerpNonceProvider for MockNonceProvider {
        fn perp_matching_nonce(
            &self,
            _matching_engine: AccountId,
            _account: AccountId,
        ) -> RpcFuture<'_, u64> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(self.nonce)
            })
        }
    }

    #[tokio::test]
    async fn disabled_sync_accepts_existing_flow_without_rpc_call() {
        let config = PerpNonceSyncConfig::disabled();
        let provider = MockNonceProvider::new(9);

        validate_order_perp_nonce(&config, &provider, &account(), 1)
            .await
            .unwrap();

        assert_eq!(provider.calls(), 0);
    }

    #[tokio::test]
    async fn strict_sync_accepts_nonce_equal_to_onchain_nonce() {
        let config = enabled_config();
        let provider = MockNonceProvider::new(7);

        validate_order_perp_nonce(&config, &provider, &account(), 7)
            .await
            .unwrap();

        assert_eq!(provider.calls(), 1);
    }

    #[tokio::test]
    async fn strict_sync_rejects_lower_nonce() {
        let error =
            validate_order_perp_nonce(&enabled_config(), &MockNonceProvider::new(7), &account(), 6)
                .await
                .unwrap_err();

        assert_eq!(
            error.to_string(),
            "perp nonce mismatch: expected on-chain nonce 7, got 6"
        );
    }

    #[tokio::test]
    async fn strict_sync_rejects_higher_nonce() {
        let error =
            validate_order_perp_nonce(&enabled_config(), &MockNonceProvider::new(7), &account(), 8)
                .await
                .unwrap_err();

        assert_eq!(
            error.to_string(),
            "perp nonce mismatch: expected on-chain nonce 7, got 8"
        );
    }

    #[tokio::test]
    async fn mismatch_does_not_consume_local_nonce() {
        let account = account();
        let error =
            validate_order_perp_nonce(&enabled_config(), &MockNonceProvider::new(7), &account, 6)
                .await
                .unwrap_err();
        assert!(matches!(error, BackendError::PerpNonceMismatch { .. }));

        let mut nonces = NonceStore::new();
        nonces.reserve(&account, 6).unwrap();
    }

    #[tokio::test]
    async fn malformed_account_address_is_rejected_without_rpc_call() {
        let provider = MockNonceProvider::new(7);
        let error = validate_order_perp_nonce(
            &enabled_config(),
            &provider,
            &AccountId::new("not-an-address"),
            7,
        )
        .await
        .unwrap_err();

        assert!(matches!(error, BackendError::MalformedAccountAddress));
        assert_eq!(provider.calls(), 0);
    }

    #[tokio::test]
    async fn missing_rpc_config_returns_clear_error_when_enabled() {
        let mut config = enabled_config();
        config.rpc_url = None;

        let error = validate_order_perp_nonce(&config, &MockNonceProvider::new(7), &account(), 7)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "configuration error: RPC_URL is required for perp nonce sync"
        );
    }

    #[test]
    fn startup_requires_rpc_when_enabled_and_require_rpc_true() {
        let mut config = enabled_config();
        config.rpc_url = None;

        let error = config.validate_startup().unwrap_err();

        assert_eq!(
            error.to_string(),
            "configuration error: RPC_URL is required for perp nonce sync"
        );
    }

    fn enabled_config() -> PerpNonceSyncConfig {
        PerpNonceSyncConfig {
            enabled: true,
            require_rpc: true,
            strict: true,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            perp_matching_engine_address: AccountId::new(
                "0x774d96E5739bffadEE91508b4D3D74F5BE29F165",
            ),
        }
    }

    fn account() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000001")
    }
}
