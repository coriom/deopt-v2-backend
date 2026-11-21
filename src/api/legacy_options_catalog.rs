//! Bounded, public RPC reads of the historical Base Sepolia Options Registry.
//! Source: OptionProductRegistry.sol and the deployed address in
//! BASE_SEPOLIA_CLOSED_TEST_PROVISIONING_V1.md. No database or signer access.

use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::options::onchain_option_series_key;
use crate::types::AccountId;
use alloy_primitives::{Address, U256};
use sha3::{Digest, Keccak256};
use std::collections::HashSet;

pub const LEGACY_REGISTRY: &str = "0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA";
const CHAIN_ID: u64 = 84532;
const MAX_SERIES: u64 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacySeries {
    pub option_id: U256,
    pub series_id: String,
    pub underlying: String,
    pub settlement_asset: String,
    pub expiry: u64,
    pub strike_1e8: u64,
    pub contract_size_1e8: u128,
    pub is_call: bool,
    pub is_european: bool,
    pub is_active: bool,
}

fn selector(signature: &str) -> Vec<u8> {
    Keccak256::digest(signature.as_bytes())[..4].to_vec()
}

async fn call<P: EthCallProvider>(
    rpc: &P,
    from: &AccountId,
    signature: &str,
    arg: Option<U256>,
) -> Result<Vec<u8>, &'static str> {
    let mut data = selector(signature);
    if let Some(value) = arg {
        data.extend_from_slice(&value.to_be_bytes::<32>());
    }
    rpc.eth_call(EthCallRequest {
        from: from.clone(),
        to: AccountId::new(LEGACY_REGISTRY),
        data,
        value: 0,
        gas_limit: None,
    })
    .await
    .map(|result| result.output)
    .map_err(|_| "historical Registry RPC unavailable")
}

fn word(data: &[u8], index: usize) -> Result<U256, &'static str> {
    let start = index * 32;
    let bytes = data
        .get(start..start + 32)
        .ok_or("historical Registry ABI mismatch")?;
    Ok(U256::from_be_slice(bytes))
}

fn u64_word(value: U256) -> Result<u64, &'static str> {
    if value > U256::from(u64::MAX) {
        return Err("historical Registry integer overflow");
    }
    Ok(value.to::<u64>())
}

fn u128_word(value: U256) -> Result<u128, &'static str> {
    if value > U256::from(u128::MAX) {
        return Err("historical Registry integer overflow");
    }
    Ok(value.to::<u128>())
}

fn bool_word(value: U256) -> Result<bool, &'static str> {
    if value == U256::ZERO {
        Ok(false)
    } else if value == U256::from(1) {
        Ok(true)
    } else {
        Err("historical Registry invalid boolean")
    }
}

fn address_word(value: U256) -> Result<String, &'static str> {
    if (value >> 160) != U256::ZERO {
        return Err("historical Registry invalid address");
    }
    let bytes = value.to_be_bytes::<32>();
    let address = Address::from_slice(&bytes[12..]);
    if address == Address::ZERO {
        return Err("historical Registry zero asset");
    }
    Ok(format!("{address:#x}"))
}

fn decode_series(data: &[u8], option_id: U256) -> Result<LegacySeries, &'static str> {
    // OptionProductRegistry.OptionSeries has nine static ABI words.
    if data.len() != 9 * 32 {
        return Err("historical Registry ABI mismatch");
    }
    let exists = bool_word(word(data, 7)?)?;
    if !exists {
        return Err("historical Registry index references missing series");
    }
    let contract_size_1e8 = u128_word(word(data, 4)?)?;
    if contract_size_1e8 == 0 {
        return Err("historical Registry zero contract size");
    }
    Ok(LegacySeries {
        option_id,
        series_id: onchain_option_series_key(CHAIN_ID, LEGACY_REGISTRY, &option_id.to_string())
            .map_err(|_| "historical Registry series identity invalid")?,
        underlying: address_word(word(data, 0)?)?,
        settlement_asset: address_word(word(data, 1)?)?,
        expiry: u64_word(word(data, 2)?)?,
        strike_1e8: u64_word(word(data, 3)?)?,
        contract_size_1e8,
        is_call: bool_word(word(data, 5)?)?,
        is_european: bool_word(word(data, 6)?)?,
        is_active: bool_word(word(data, 8)?)?,
    })
}

pub async fn read_series<P: EthCallProvider>(
    rpc: &P,
    from: &AccountId,
    chain_id: u64,
) -> Result<Vec<LegacySeries>, &'static str> {
    if chain_id != CHAIN_ID {
        return Err("historical Registry chain mismatch");
    }
    let count_data = call(rpc, from, "totalSeries()", None).await?;
    if count_data.len() != 32 {
        return Err("historical Registry ABI mismatch");
    }
    let count = u64_word(word(&count_data, 0)?)?;
    if count > MAX_SERIES {
        return Err("historical Registry catalogue exceeds bounded read limit");
    }
    let mut result = Vec::with_capacity(count as usize);
    let mut ids = HashSet::new();
    for index in 0..count {
        let id_data = call(rpc, from, "seriesAt(uint256)", Some(U256::from(index))).await?;
        if id_data.len() != 32 {
            return Err("historical Registry ABI mismatch");
        }
        let id = word(&id_data, 0)?;
        if !ids.insert(id) {
            return Err("historical Registry duplicate series id");
        }
        let series_data = call(rpc, from, "getSeries(uint256)", Some(id)).await?;
        result.push(decode_series(&series_data, id)?);
    }
    let final_count = call(rpc, from, "totalSeries()", None).await?;
    if final_count.len() != 32 || word(&final_count, 0)? != U256::from(count) {
        return Err("historical Registry changed during catalogue read");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BackendError;
    use crate::execution::rpc::{EthCallSuccess, RpcFuture};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockRpc(Arc<Mutex<VecDeque<Result<Vec<u8>, BackendError>>>>);

    impl EthCallProvider for MockRpc {
        fn eth_call(&self, _request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            Box::pin(async move {
                self.0
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("mock reply")
                    .map(|output| EthCallSuccess {
                        block_number: Some(123),
                        output,
                    })
            })
        }
    }

    fn encode(words: &[U256]) -> Vec<u8> {
        words
            .iter()
            .flat_map(|word| word.to_be_bytes::<32>())
            .collect()
    }

    #[test]
    fn decodes_exact_abi_and_canonical_identity() {
        let data = encode(&[
            U256::from(0x1234u64),
            U256::from(0x5678u64),
            U256::from(1_893_456_000u64),
            U256::from(300_000_000_000u64),
            U256::from(200_000_000u64),
            U256::from(1),
            U256::from(1),
            U256::from(1),
            U256::from(0),
        ]);
        let series = decode_series(&data, U256::from(7)).unwrap();
        assert_eq!(series.strike_1e8, 300_000_000_000);
        assert_eq!(series.contract_size_1e8, 200_000_000);
        assert!(!series.is_active);
        assert!(series.is_european);
        assert_eq!(series.series_id.len(), 66);
        assert_ne!(
            series.series_id,
            onchain_option_series_key(CHAIN_ID, "0xcb7bc539ed4A5771F3420f498002e0fc4eE1bD27", "7")
                .unwrap()
        );
        assert!(decode_series(&data[..32], U256::from(7)).is_err());
    }

    #[tokio::test]
    async fn bounded_catalogue_preserves_both_statuses_and_fails_on_rpc_error() {
        let active = encode(&[
            U256::from(0x1234u64),
            U256::from(0x5678u64),
            U256::from(1_893_456_000u64),
            U256::from(300_000_000_000u64),
            U256::from(100_000_000u64),
            U256::from(1),
            U256::from(1),
            U256::from(1),
            U256::from(1),
        ]);
        let mut inactive = active.clone();
        inactive[8 * 32 + 31] = 0;
        let queue = VecDeque::from([
            Ok(encode(&[U256::from(2)])),
            Ok(encode(&[U256::from(11)])),
            Ok(active),
            Ok(encode(&[U256::from(12)])),
            Ok(inactive),
            Ok(encode(&[U256::from(2)])),
        ]);
        let from = AccountId::new("0x0000000000000000000000000000000000000001");
        let series = read_series(&MockRpc(Arc::new(Mutex::new(queue))), &from, CHAIN_ID)
            .await
            .unwrap();
        assert_eq!(series.len(), 2);
        assert!(series[0].is_active);
        assert!(!series[1].is_active);
        assert_ne!(series[0].series_id, series[1].series_id);
        let empty = MockRpc(Arc::new(Mutex::new(VecDeque::from([
            Ok(encode(&[U256::ZERO])),
            Ok(encode(&[U256::ZERO])),
        ]))));
        assert!(read_series(&empty, &from, CHAIN_ID)
            .await
            .unwrap()
            .is_empty());
        let error = MockRpc(Arc::new(Mutex::new(VecDeque::from([Err(
            BackendError::Simulation("unavailable".into()),
        )]))));
        assert!(read_series(&error, &from, CHAIN_ID).await.is_err());
        assert!(read_series(&empty, &from, 1).await.is_err());
    }

    #[tokio::test]
    #[ignore = "requires Base Sepolia RPC"]
    async fn live_base_sepolia_historical_registry() {
        use crate::execution::rpc::{HttpJsonRpcProvider, TransactionBroadcastProvider};
        let rpc = HttpJsonRpcProvider::new("https://sepolia.base.org");
        let from = AccountId::new("0x0000000000000000000000000000000000000001");
        let chain_id = rpc.chain_id().await.unwrap();
        let started = std::time::Instant::now();
        let series = read_series(&rpc, &from, chain_id).await.unwrap();
        assert_eq!(series.len(), 2);
        assert!(series[0].is_call && series[0].is_european && series[0].is_active);
        assert!(series[1].is_call && series[1].is_european && !series[1].is_active);
        assert_eq!(series[0].expiry, 1_893_456_000);
        assert_eq!(series[1].expiry, 1_893_456_000);
        assert_eq!(series[0].strike_1e8, 300_000_000_000);
        assert_eq!(series[1].strike_1e8, 9_000_000_000_000);
        println!("V40_LIVE_RPC_MS={}", started.elapsed().as_millis());
    }
}
