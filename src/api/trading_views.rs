//! M-P2c — read-only on-chain RPC orchestration for the trading API.
//!
//! Adds narrow inline `alloy_sol_types::sol!` declarations against the
//! frozen ABI surface at
//! `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/` + thin async
//! helpers that use the existing `EthCallProvider` infrastructure.
//!
//! **Posture:** read-only. NO `eth_sendRawTransaction`. NO signer call.
//! NO broadcast. NO AWS / KMS call. NO mainnet broadcast. NO state
//! mutation. The trading handler MUST tolerate `None` from any helper
//! (config missing or RPC unavailable) and emit a structured warning
//! per the M-P2b convention.
//!
//! Selectors verified at PR time against
//! `~/DEOPT/deopt-v2-sol/abis/freeze-v2-product-rc1/selectors.txt`:
//!
//! - `MarginEngineLens.getAccountState(address,address)` → `0xa57bd4cc`
//! - `MarginEngineLens.previewTradeFees(address,uint256,uint128,uint128,address,address,bool)` → `0x6ffe6d79`
//! - `MarginEngineLens.previewAccountSettlement(address,uint256,address)` → `0xe80299c3`
//! - `MarginEngineLens.previewDetailedSettlement(address,uint256,address)` → `0x884ceaae`
//! - `CollateralVaultViews.getCollateralTokens()` → `0xb58eb63f`
//! - `CollateralVault.balances(address,address)` → `0xc23f001f`
//! - `OracleRouter.getFeed(address,address)` → `0xd2edb6dd`
//! - `OracleRouter.hasActiveFeed(address,address)` → `0x6c166bb3`
//!
//! Test `selector_verification_*` (this file) re-asserts the selectors
//! at build time so any drift between this module and the frozen ABI
//! fails CI.

use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::types::AccountId;
use alloy_primitives::{Address, U256};
use alloy_sol_types::{sol, SolCall};

// ---------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------

/// Optional on-chain read addresses for the trading API.
///
/// **All fields are optional.** A `None` address routes the handler to
/// the M-P2b partial-data path (structured warning, no panic). The
/// trading handlers do NOT require any of these for startup; they only
/// upgrade `status: "partial"` to `status: "ok"` when configured.
///
/// **NO ENV WIRING in this milestone.** This struct is constructed
/// inside `AppState` with all-`None` defaults; operator-side wiring
/// from env vars lands with `M-P2d` once the per-deploy address
/// inventory is finalised. M-P2c provides the type + behaviour only.
#[derive(Clone, Debug, Default)]
pub struct TradingViewsConfig {
    pub margin_engine_lens_address: Option<AccountId>,
    pub collateral_vault_views_address: Option<AccountId>,
    pub collateral_vault_address: Option<AccountId>,
    pub oracle_router_address: Option<AccountId>,
}

impl TradingViewsConfig {
    pub fn disabled() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------
// ABI bindings
// ---------------------------------------------------------------------

sol! {
    // MarginEngineLens — UI-facing aggregated views.
    function getAccountState(address marginEngine, address trader) external view returns (bytes memory);

    // previewTradeFees(marginEngine, optionId, qty, price, buyer, seller, buyerIsMaker)
    function previewTradeFees(
        address marginEngine,
        uint256 optionId,
        uint128 quantity,
        uint128 price,
        address buyer,
        address seller,
        bool buyerIsMaker
    ) external view returns (bytes memory);

    function previewAccountSettlement(address marginEngine, uint256 optionId, address trader) external view returns (bytes memory);

    function previewDetailedSettlement(address marginEngine, uint256 optionId, address trader) external view returns (bytes memory);

    // CollateralVaultViews / CollateralVault
    function getCollateralTokens() external view returns (address[] memory);

    function balances(address account, address token) external view returns (uint256);

    // OracleRouter
    function getFeed(address baseAsset, address quoteAsset) external view returns (bytes memory);

    function hasActiveFeed(address baseAsset, address quoteAsset) external view returns (bool);
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn account_to_address(a: &AccountId) -> Option<Address> {
    a.0.strip_prefix("0x").and_then(|hex| {
        if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            None
        } else {
            let mut bytes = [0u8; 20];
            for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
                let s = std::str::from_utf8(chunk).ok()?;
                bytes[i] = u8::from_str_radix(s, 16).ok()?;
            }
            Some(Address::from(bytes))
        }
    })
}

pub fn address_to_account(a: Address) -> AccountId {
    AccountId::new(format!("{a:#x}"))
}

/// Iterate the configured collateral token list.
///
/// Returns `Ok(Some(Vec<Address>))` on success, `Ok(None)` when no
/// view-config is supplied, or `Err` when the configured RPC call
/// fails. Callers map `Err` → typed structured warning + `partial`
/// envelope.
pub async fn try_get_collateral_tokens<P: EthCallProvider>(
    cfg: &TradingViewsConfig,
    from: &AccountId,
    provider: &P,
) -> Result<Option<Vec<Address>>, String> {
    let Some(addr) = cfg.collateral_vault_views_address.as_ref() else {
        return Ok(None);
    };
    let req = EthCallRequest {
        from: from.clone(),
        to: addr.clone(),
        data: getCollateralTokensCall {}.abi_encode(),
        value: 0,
        gas_limit: None,
    };
    let success = provider
        .eth_call(req)
        .await
        .map_err(|e| format!("eth_call failed: {e}"))?;
    let decoded = getCollateralTokensCall::abi_decode_returns(&success.output, true)
        .map_err(|e| format!("decode failed: {e}"))?;
    Ok(Some(decoded._0))
}

/// Read a single per-account per-token balance.
pub async fn try_get_balance<P: EthCallProvider>(
    cfg: &TradingViewsConfig,
    from: &AccountId,
    account: &AccountId,
    token: Address,
    provider: &P,
) -> Result<Option<U256>, String> {
    let Some(cv_addr) = cfg.collateral_vault_address.as_ref() else {
        return Ok(None);
    };
    let Some(account_addr) = account_to_address(account) else {
        return Err("account address invalid".to_string());
    };
    let req = EthCallRequest {
        from: from.clone(),
        to: cv_addr.clone(),
        data: balancesCall {
            account: account_addr,
            token,
        }
        .abi_encode(),
        value: 0,
        gas_limit: None,
    };
    let success = provider
        .eth_call(req)
        .await
        .map_err(|e| format!("eth_call failed: {e}"))?;
    let decoded = balancesCall::abi_decode_returns(&success.output, true)
        .map_err(|e| format!("decode failed: {e}"))?;
    Ok(Some(decoded._0))
}

/// MarginEngineLens.previewTradeFees — returns the encoded bytes; the
/// caller decodes into the engine-specific struct shape.
#[allow(clippy::too_many_arguments)]
pub async fn try_preview_trade_fees<P: EthCallProvider>(
    cfg: &TradingViewsConfig,
    from: &AccountId,
    margin_engine: Address,
    option_id: U256,
    quantity: u128,
    price: u128,
    buyer: Address,
    seller: Address,
    buyer_is_maker: bool,
    provider: &P,
) -> Result<Option<Vec<u8>>, String> {
    let Some(lens_addr) = cfg.margin_engine_lens_address.as_ref() else {
        return Ok(None);
    };
    let req = EthCallRequest {
        from: from.clone(),
        to: lens_addr.clone(),
        data: previewTradeFeesCall {
            marginEngine: margin_engine,
            optionId: option_id,
            quantity,
            price,
            buyer,
            seller,
            buyerIsMaker: buyer_is_maker,
        }
        .abi_encode(),
        value: 0,
        gas_limit: None,
    };
    let success = provider
        .eth_call(req)
        .await
        .map_err(|e| format!("eth_call failed: {e}"))?;
    Ok(Some(success.output))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::error::{BackendError, Result as BackendResult};
    use crate::execution::rpc::{EthCallSuccess, RpcFuture};
    use std::sync::{Arc, Mutex};

    // ----- Selector verification -----
    //
    // Re-asserts each declared sol! function against the frozen ABI
    // selectors. Any drift fails CI.

    #[test]
    fn selector_get_collateral_tokens() {
        let bytes = getCollateralTokensCall {}.abi_encode();
        assert_eq!(&bytes[..4], &[0xb5, 0x8e, 0xb6, 0x3f]);
    }

    #[test]
    fn selector_balances() {
        let bytes = balancesCall {
            account: Address::ZERO,
            token: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0xc2, 0x3f, 0x00, 0x1f]);
    }

    #[test]
    fn selector_get_account_state() {
        let bytes = getAccountStateCall {
            marginEngine: Address::ZERO,
            trader: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0xa5, 0x7b, 0xd4, 0xcc]);
    }

    #[test]
    fn selector_preview_trade_fees() {
        let bytes = previewTradeFeesCall {
            marginEngine: Address::ZERO,
            optionId: U256::ZERO,
            quantity: 0,
            price: 0,
            buyer: Address::ZERO,
            seller: Address::ZERO,
            buyerIsMaker: false,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0x6f, 0xfe, 0x6d, 0x79]);
    }

    #[test]
    fn selector_preview_account_settlement() {
        let bytes = previewAccountSettlementCall {
            marginEngine: Address::ZERO,
            optionId: U256::ZERO,
            trader: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0xe8, 0x02, 0x99, 0xc3]);
    }

    #[test]
    fn selector_preview_detailed_settlement() {
        let bytes = previewDetailedSettlementCall {
            marginEngine: Address::ZERO,
            optionId: U256::ZERO,
            trader: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0x88, 0x4c, 0xea, 0xae]);
    }

    #[test]
    fn selector_get_feed() {
        let bytes = getFeedCall {
            baseAsset: Address::ZERO,
            quoteAsset: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0xd2, 0xed, 0xb6, 0xdd]);
    }

    #[test]
    fn selector_has_active_feed() {
        let bytes = hasActiveFeedCall {
            baseAsset: Address::ZERO,
            quoteAsset: Address::ZERO,
        }
        .abi_encode();
        assert_eq!(&bytes[..4], &[0x6c, 0x16, 0x6b, 0xb3]);
    }

    // ----- Mock provider -----

    type MockRule = ([u8; 4], Result<Vec<u8>, ()>);

    #[derive(Clone, Default)]
    pub struct ProgrammableMockProvider {
        // (selector_first_4_bytes, EthCallSuccess or error)
        pub rules: Arc<Mutex<Vec<MockRule>>>,
    }

    impl ProgrammableMockProvider {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn returns(&self, selector: [u8; 4], output: Vec<u8>) {
            self.rules.lock().unwrap().push((selector, Ok(output)));
        }
        pub fn fails(&self, selector: [u8; 4]) {
            self.rules.lock().unwrap().push((selector, Err(())));
        }
    }

    impl EthCallProvider for ProgrammableMockProvider {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let rules = self.rules.lock().unwrap();
            let outcome = rules
                .iter()
                .find(|(sel, _)| request.data.starts_with(sel))
                .cloned();
            drop(rules);
            Box::pin(async move {
                let r: BackendResult<EthCallSuccess> = match outcome {
                    Some((_, Ok(out))) => Ok(EthCallSuccess {
                        block_number: Some(1),
                        output: out,
                    }),
                    Some((_, Err(()))) => Err(BackendError::Simulation("mock fail".into())),
                    None => Err(BackendError::Simulation("no rule".into())),
                };
                r
            })
        }
    }

    fn anvil_address() -> AccountId {
        AccountId::new("0x1234567890abcdef1234567890abcdef12345678")
    }

    fn account_2() -> AccountId {
        AccountId::new("0xabcdef0123456789abcdef0123456789abcdef01")
    }

    // ----- try_get_collateral_tokens -----

    #[tokio::test]
    async fn collateral_tokens_returns_none_when_not_configured() {
        let cfg = TradingViewsConfig::default();
        let mock = ProgrammableMockProvider::new();
        let out = try_get_collateral_tokens(&cfg, &anvil_address(), &mock).await;
        assert!(matches!(out, Ok(None)));
    }

    #[tokio::test]
    async fn collateral_tokens_returns_list_when_configured() {
        let cfg = TradingViewsConfig {
            collateral_vault_views_address: Some(anvil_address()),
            ..Default::default()
        };
        // ABI-encode a return of [tokenA, tokenB].
        let tokens: Vec<Address> = vec![Address::from([1u8; 20]), Address::from([2u8; 20])];
        let encoded = alloy_sol_types::SolValue::abi_encode(&tokens);
        let mock = ProgrammableMockProvider::new();
        mock.returns([0xb5, 0x8e, 0xb6, 0x3f], encoded);
        let out = try_get_collateral_tokens(&cfg, &anvil_address(), &mock)
            .await
            .expect("ok")
            .expect("Some");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], Address::from([1u8; 20]));
        assert_eq!(out[1], Address::from([2u8; 20]));
    }

    #[tokio::test]
    async fn collateral_tokens_rpc_failure_yields_err() {
        let cfg = TradingViewsConfig {
            collateral_vault_views_address: Some(anvil_address()),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xb5, 0x8e, 0xb6, 0x3f]);
        let out = try_get_collateral_tokens(&cfg, &anvil_address(), &mock).await;
        assert!(out.is_err());
    }

    // ----- try_get_balance -----

    #[tokio::test]
    async fn balance_returns_none_when_cv_not_configured() {
        let cfg = TradingViewsConfig::default();
        let mock = ProgrammableMockProvider::new();
        let out = try_get_balance(&cfg, &anvil_address(), &account_2(), Address::ZERO, &mock).await;
        assert!(matches!(out, Ok(None)));
    }

    #[tokio::test]
    async fn balance_returns_value_when_configured() {
        let cfg = TradingViewsConfig {
            collateral_vault_address: Some(anvil_address()),
            ..Default::default()
        };
        let val = U256::from(1_000_000_000u64);
        let encoded = alloy_sol_types::SolValue::abi_encode(&val);
        let mock = ProgrammableMockProvider::new();
        mock.returns([0xc2, 0x3f, 0x00, 0x1f], encoded);
        let out = try_get_balance(
            &cfg,
            &anvil_address(),
            &account_2(),
            Address::from([0xaa; 20]),
            &mock,
        )
        .await
        .expect("ok")
        .expect("Some");
        assert_eq!(out, val);
    }

    #[tokio::test]
    async fn balance_rejects_malformed_account() {
        let cfg = TradingViewsConfig {
            collateral_vault_address: Some(anvil_address()),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        let out = try_get_balance(
            &cfg,
            &anvil_address(),
            &AccountId::new("not-an-address"),
            Address::ZERO,
            &mock,
        )
        .await;
        assert!(out.is_err());
    }

    #[tokio::test]
    async fn balance_rpc_failure_yields_err() {
        let cfg = TradingViewsConfig {
            collateral_vault_address: Some(anvil_address()),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        mock.fails([0xc2, 0x3f, 0x00, 0x1f]);
        let out = try_get_balance(&cfg, &anvil_address(), &account_2(), Address::ZERO, &mock).await;
        assert!(out.is_err());
    }

    // ----- try_preview_trade_fees -----

    #[tokio::test]
    async fn preview_trade_fees_returns_none_when_lens_not_configured() {
        let cfg = TradingViewsConfig::default();
        let mock = ProgrammableMockProvider::new();
        let out = try_preview_trade_fees(
            &cfg,
            &anvil_address(),
            Address::ZERO,
            U256::ZERO,
            0,
            0,
            Address::ZERO,
            Address::ZERO,
            false,
            &mock,
        )
        .await;
        assert!(matches!(out, Ok(None)));
    }

    #[tokio::test]
    async fn preview_trade_fees_returns_raw_output_when_configured() {
        let cfg = TradingViewsConfig {
            margin_engine_lens_address: Some(anvil_address()),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        let raw = vec![0xde, 0xad, 0xbe, 0xef];
        mock.returns([0x6f, 0xfe, 0x6d, 0x79], raw.clone());
        let out = try_preview_trade_fees(
            &cfg,
            &anvil_address(),
            Address::ZERO,
            U256::ZERO,
            1,
            2,
            Address::ZERO,
            Address::ZERO,
            false,
            &mock,
        )
        .await
        .expect("ok")
        .expect("Some");
        assert_eq!(out, raw);
    }

    #[tokio::test]
    async fn preview_trade_fees_rpc_failure_yields_err() {
        let cfg = TradingViewsConfig {
            margin_engine_lens_address: Some(anvil_address()),
            ..Default::default()
        };
        let mock = ProgrammableMockProvider::new();
        mock.fails([0x6f, 0xfe, 0x6d, 0x79]);
        let out = try_preview_trade_fees(
            &cfg,
            &anvil_address(),
            Address::ZERO,
            U256::ZERO,
            0,
            0,
            Address::ZERO,
            Address::ZERO,
            false,
            &mock,
        )
        .await;
        assert!(out.is_err());
    }

    // ----- account_to_address -----

    #[test]
    fn account_to_address_handles_valid_hex() {
        let a = AccountId::new("0x1234567890abcdef1234567890abcdef12345678");
        let addr = account_to_address(&a).expect("parsed");
        assert_eq!(addr.0 .0[0], 0x12);
        assert_eq!(addr.0 .0[19], 0x78);
    }

    #[test]
    fn account_to_address_rejects_bad_input() {
        assert!(account_to_address(&AccountId::new("nope")).is_none());
        assert!(account_to_address(&AccountId::new("0xabcd")).is_none());
        assert!(account_to_address(&AccountId::new(
            "0xZZZZ567890abcdef1234567890abcdef12345678"
        ))
        .is_none());
    }
}
