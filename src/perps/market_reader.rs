//! PERPS-MINIMAL-MARKET-AND-PRICE-V1 — read-only PerpMarketRegistry
//! reader.
//!
//! Behind a trait so tests can inject a deterministic fake without
//! standing up a real RPC endpoint. The concrete
//! `PerpMarketRegistryRpcReader` uses the existing
//! `EthCallProvider`/`HttpJsonRpcProvider` and hand-declares only the
//! `marketExists(uint256)` and `isMarketActive(uint256)` ABI stubs — the rich
//! `getMarket` struct decode is deferred to
//! `PERPS-ISOLATED-MARGIN-POSITION-ENGINE-V1` when the full market
//! shape is actually needed on the backend.

use crate::error::{BackendError, Result};
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::perps::config::{PerpsReadConfig, PerpsReadMarket};
use crate::perps::types::PerpMarketStatus;
use crate::types::AccountId;
use alloy_primitives::U256;
use alloy_sol_types::{sol, SolCall};
use async_trait::async_trait;

sol! {
    /// PerpMarketRegistry read ABI. The concrete address is supplied
    /// by PERPS_MARKET_REGISTRY_ADDRESS at startup. Richer
    /// reads (risk config, funding config, launch caps) land in later
    /// milestones alongside the code paths that would consume them.
    function isMarketActive(uint256 marketId) external view returns (bool);
    function marketExists(uint256 marketId) external view returns (bool);
}

/// Read a single market's on-chain liveness flag.
#[async_trait]
pub trait PerpMarketRegistryReader: Send + Sync {
    /// Return `Ok(status)` mapped to `ReadOnly` (active) or `Paused`
    /// (inactive). Returns `PerpsMarketNotFound` when the market has not
    /// yet been created, and `PerpsRegistryUnavailable` when the chain
    /// read cannot establish existence or status.
    async fn read_status(&self, market: &PerpsReadMarket) -> Result<PerpMarketStatus>;
}

/// Concrete reader wired to the deployed `PerpMarketRegistry` via the
/// existing `HttpJsonRpcProvider`. Never fabricates status on failure —
/// callers get the `BackendError` untouched.
#[derive(Clone)]
pub struct PerpMarketRegistryRpcReader<P: EthCallProvider> {
    provider: P,
    registry_address: AccountId,
    /// Used as the `from` field on every `eth_call`. Reads don't care
    /// about the sender, but Alloy's provider requires a value.
    caller: AccountId,
}

impl<P: EthCallProvider> PerpMarketRegistryRpcReader<P> {
    pub fn new(provider: P, cfg: &PerpsReadConfig) -> Result<Self> {
        let registry_address = cfg.market_registry_address.clone().ok_or_else(|| {
            BackendError::Config(
                "PerpMarketRegistryRpcReader requires \
                 PERPS_MARKET_REGISTRY_ADDRESS to be set"
                    .to_string(),
            )
        })?;
        Ok(Self {
            provider,
            registry_address,
            caller: AccountId::new("0x0000000000000000000000000000000000000000"),
        })
    }
}

#[async_trait]
impl<P: EthCallProvider + 'static> PerpMarketRegistryReader for PerpMarketRegistryRpcReader<P> {
    async fn read_status(&self, market: &PerpsReadMarket) -> Result<PerpMarketStatus> {
        // isMarketActive reverts for an absent market. Check existence
        // explicitly so a configured future market is not advertised as
        // an existing paused market, and so transport errors are not
        // mistaken for a genuine 404.
        let exists = self
            .provider
            .eth_call(EthCallRequest {
                from: self.caller.clone(),
                to: self.registry_address.clone(),
                data: marketExistsCall {
                    marketId: U256::from(market.onchain_market_id),
                }
                .abi_encode(),
                value: 0,
                gas_limit: None,
            })
            .await
            .map_err(|_| {
                BackendError::PerpsRegistryUnavailable("market existence read failed".into())
            })?;
        let exists = marketExistsCall::abi_decode_returns(&exists.output, true).map_err(|_| {
            BackendError::PerpsRegistryUnavailable("invalid market existence response".into())
        })?;
        if !exists._0 {
            return Err(BackendError::PerpsMarketNotFound(market.symbol.clone()));
        }
        let call = isMarketActiveCall {
            marketId: U256::from(market.onchain_market_id),
        };
        let request = EthCallRequest {
            from: self.caller.clone(),
            to: self.registry_address.clone(),
            data: call.abi_encode(),
            value: 0,
            gas_limit: None,
        };
        let success = self.provider.eth_call(request).await.map_err(|_| {
            BackendError::PerpsRegistryUnavailable(format!(
                "market status read failed for id {}",
                market.onchain_market_id
            ))
        })?;
        let decoded =
            isMarketActiveCall::abi_decode_returns(&success.output, true).map_err(|_| {
                BackendError::PerpsRegistryUnavailable(format!(
                    "invalid market status response for id {}",
                    market.onchain_market_id
                ))
            })?;
        Ok(if decoded._0 {
            PerpMarketStatus::ReadOnly
        } else {
            PerpMarketStatus::Paused
        })
    }
}

/// Deterministic in-memory reader for tests. Returns whatever the
/// caller seeded per `onchain_market_id`; unseeded ids return
/// `PerpMarketStatus::Unknown` so tests can assert the negative path
/// too.
#[derive(Clone, Debug, Default)]
pub struct InMemoryPerpMarketRegistryReader {
    by_id: std::collections::HashMap<u64, PerpMarketStatus>,
    /// When Some, `read_status` always returns this error, letting
    /// tests exercise the RPC-failure branch without wiring a real
    /// provider that flakes.
    force_error: Option<String>,
}

impl InMemoryPerpMarketRegistryReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_status(mut self, market_id: u64, status: PerpMarketStatus) -> Self {
        self.by_id.insert(market_id, status);
        self
    }

    pub fn with_forced_error(mut self, error: impl Into<String>) -> Self {
        self.force_error = Some(error.into());
        self
    }
}

#[async_trait]
impl PerpMarketRegistryReader for InMemoryPerpMarketRegistryReader {
    async fn read_status(&self, market: &PerpsReadMarket) -> Result<PerpMarketStatus> {
        if let Some(err) = &self.force_error {
            return Err(BackendError::PerpsMarketNotFound(err.clone()));
        }
        Ok(self
            .by_id
            .get(&market.onchain_market_id)
            .copied()
            .unwrap_or(PerpMarketStatus::Unknown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::rpc::{EthCallSuccess, RpcFuture};

    #[derive(Clone)]
    struct RegistryRpc {
        exists: Option<bool>,
        active: Option<bool>,
    }

    impl EthCallProvider for RegistryRpc {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let value = if request.data.starts_with(&marketExistsCall::SELECTOR) {
                self.exists
            } else if request.data.starts_with(&isMarketActiveCall::SELECTOR) {
                self.active
            } else {
                None
            };
            Box::pin(async move {
                let value =
                    value.ok_or_else(|| BackendError::Simulation("rpc unavailable".into()))?;
                let mut output = vec![0; 32];
                output[31] = u8::from(value);
                Ok(EthCallSuccess {
                    block_number: Some(1),
                    output,
                })
            })
        }
    }

    fn eth_market() -> PerpsReadMarket {
        PerpsReadMarket {
            symbol: "ETH-PERP".to_string(),
            onchain_market_id: 1,
            base_asset_label: "ETH".to_string(),
            quote_asset_label: "mUSDC".to_string(),
            base_asset_address: AccountId::new("0x0000000000000000000000000000000000000001"),
            quote_asset_address: AccountId::new("0x0000000000000000000000000000000000000002"),
            max_leverage: 10,
            maintenance_margin_bps: 500,
            max_order_size_1e8: None,
            max_order_notional_1e8: None,
            max_subaccount_notional_1e8: None,
            max_open_interest_1e8: None,
        }
    }

    #[tokio::test]
    async fn in_memory_reader_returns_seeded_status() {
        let reader =
            InMemoryPerpMarketRegistryReader::new().with_status(1, PerpMarketStatus::ReadOnly);
        let status = reader.read_status(&eth_market()).await.unwrap();
        assert_eq!(status, PerpMarketStatus::ReadOnly);
    }

    #[tokio::test]
    async fn in_memory_reader_returns_unknown_for_unseeded_id() {
        let reader = InMemoryPerpMarketRegistryReader::new();
        let status = reader.read_status(&eth_market()).await.unwrap();
        assert_eq!(status, PerpMarketStatus::Unknown);
    }

    #[tokio::test]
    async fn in_memory_reader_propagates_forced_error() {
        let reader =
            InMemoryPerpMarketRegistryReader::new().with_forced_error("simulated rpc down");
        let err = reader.read_status(&eth_market()).await.unwrap_err();
        assert!(matches!(err, BackendError::PerpsMarketNotFound(_)));
    }

    #[tokio::test]
    async fn absent_onchain_market_is_not_a_paused_market() {
        let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        let reader = PerpMarketRegistryRpcReader::new(
            RegistryRpc {
                exists: Some(false),
                active: Some(true),
            },
            &cfg,
        )
        .unwrap();
        assert!(matches!(
            reader.read_status(&eth_market()).await,
            Err(BackendError::PerpsMarketNotFound(_))
        ));
        assert!(crate::perps::service::list_perp_markets(&cfg, &reader)
            .await
            .unwrap()
            .markets
            .is_empty());
        assert!(matches!(
            crate::perps::service::get_perp_market(&cfg, &reader, "ETH-PERP").await,
            Err(BackendError::PerpsMarketNotFound(_))
        ));
    }

    #[tokio::test]
    async fn registry_outage_is_distinct_from_absent_market() {
        let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        let reader = PerpMarketRegistryRpcReader::new(
            RegistryRpc {
                exists: None,
                active: None,
            },
            &cfg,
        )
        .unwrap();
        assert!(matches!(
            reader.read_status(&eth_market()).await,
            Err(BackendError::PerpsRegistryUnavailable(_))
        ));
        let listing = crate::perps::service::list_perp_markets(&cfg, &reader)
            .await
            .unwrap();
        assert_eq!(listing.markets.len(), 2);
        assert!(listing
            .markets
            .iter()
            .all(|m| m.status == PerpMarketStatus::Unknown && !m.trading_enabled));
    }

    #[tokio::test]
    async fn existing_inactive_market_is_paused() {
        let cfg = PerpsReadConfig::enabled_in_memory_for_tests();
        let reader = PerpMarketRegistryRpcReader::new(
            RegistryRpc {
                exists: Some(true),
                active: Some(false),
            },
            &cfg,
        )
        .unwrap();
        assert_eq!(
            reader.read_status(&eth_market()).await.unwrap(),
            PerpMarketStatus::Paused
        );
    }
}
