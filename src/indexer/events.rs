use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EthLog {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    pub block_number: Option<String>,
    pub block_hash: Option<String>,
    pub transaction_hash: Option<String>,
    pub log_index: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IndexedPerpTrade {
    #[serde(rename = "eventId")]
    pub event_id: String,
    #[serde(rename = "txHash")]
    pub tx_hash: String,
    #[serde(rename = "logIndex")]
    pub log_index: u64,
    #[serde(rename = "blockNumber")]
    pub block_number: u64,
    #[serde(rename = "blockHash")]
    pub block_hash: Option<String>,
    #[serde(rename = "onchainIntentId")]
    pub onchain_intent_id: Option<String>,
    pub buyer: String,
    pub seller: String,
    #[serde(rename = "marketId")]
    pub market_id: String,
    #[serde(rename = "sizeDelta1e8")]
    pub size_delta_1e8: String,
    #[serde(rename = "executionPrice1e8")]
    pub execution_price_1e8: String,
    #[serde(rename = "buyerIsMaker")]
    pub buyer_is_maker: bool,
    #[serde(rename = "buyerNonce")]
    pub buyer_nonce: String,
    #[serde(rename = "sellerNonce")]
    pub seller_nonce: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
}
