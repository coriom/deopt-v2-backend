//! Reusable deterministic broadcast RPC mock for the Hybrid V2
//! broadcast lifecycle PG integration tests (Package B, Parts J–P).
//!
//! Frozen safety: `MockBroadcastRpc` accepts `eth_sendRawTransaction`
//! (returns a configurable outcome) and records every write invocation
//! so tests can assert `write_method_calls() == ["eth_sendRawTransaction"]`.
//! Never actually reaches a public chain.

#![allow(dead_code)]

use async_trait::async_trait;
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BlockHeader, BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TransactionSummary,
    TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag;
use std::collections::HashMap;
use std::sync::Mutex;

/// Deterministic mock. All configurable state lives behind a mutex —
/// tests build one instance per case and configure it via the setters.
pub struct MockBroadcastRpc {
    inner: Mutex<MockInner>,
}

#[derive(Default)]
struct MockInner {
    /// Queue of `send_raw_transaction` outcomes. Consumed FIFO; the
    /// last element is repeated after the queue drains (so many
    /// consecutive resends receive the same outcome).
    send_outcomes: Vec<Result<SendOutcome, BroadcastRpcError>>,
    write_calls: Vec<&'static str>,
    receipts: HashMap<[u8; 32], TxReceipt>,
    transactions: HashMap<[u8; 32], TransactionSummary>,
    headers_by_number: HashMap<u64, BlockHeader>,
    headers_by_hash: HashMap<[u8; 32], BlockHeader>,
    head: u64,
    finalized: Option<u64>,
    nonce_by_addr: HashMap<([u8; 20], String), u64>,
    chain_id: u64,
}

impl Default for MockBroadcastRpc {
    fn default() -> Self {
        Self {
            inner: Mutex::new(MockInner {
                send_outcomes: Vec::new(),
                write_calls: Vec::new(),
                receipts: HashMap::new(),
                transactions: HashMap::new(),
                headers_by_number: HashMap::new(),
                headers_by_hash: HashMap::new(),
                head: 100,
                finalized: Some(90),
                nonce_by_addr: HashMap::new(),
                chain_id: 84532,
            }),
        }
    }
}

impl MockBroadcastRpc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.inner.lock().unwrap().chain_id = chain_id;
    }

    pub fn set_head(&self, head: u64) {
        self.inner.lock().unwrap().head = head;
    }

    pub fn set_finalized(&self, finalized: Option<u64>) {
        self.inner.lock().unwrap().finalized = finalized;
    }

    /// Push the next `send_raw_transaction` outcome. If called multiple
    /// times, outcomes are consumed FIFO.
    pub fn set_next_send_outcome(&self, outcome: Result<SendOutcome, BroadcastRpcError>) {
        self.inner.lock().unwrap().send_outcomes.push(outcome);
    }

    pub fn set_receipt_response(&self, tx_hash: [u8; 32], receipt: Option<TxReceipt>) {
        let mut g = self.inner.lock().unwrap();
        match receipt {
            Some(r) => {
                g.receipts.insert(tx_hash, r);
            }
            None => {
                g.receipts.remove(&tx_hash);
            }
        }
    }

    pub fn set_transaction_response(&self, tx_hash: [u8; 32], tx: Option<TransactionSummary>) {
        let mut g = self.inner.lock().unwrap();
        match tx {
            Some(t) => {
                g.transactions.insert(tx_hash, t);
            }
            None => {
                g.transactions.remove(&tx_hash);
            }
        }
    }

    pub fn set_block_hash_at(&self, block_number: u64, hash: [u8; 32]) {
        let header = BlockHeader {
            number: block_number,
            hash,
            parent_hash: [0; 32],
            timestamp: 0,
        };
        let mut g = self.inner.lock().unwrap();
        g.headers_by_number.insert(block_number, header.clone());
        g.headers_by_hash.insert(hash, header);
    }

    pub fn clear_block_at(&self, block_number: u64) {
        let mut g = self.inner.lock().unwrap();
        if let Some(header) = g.headers_by_number.remove(&block_number) {
            g.headers_by_hash.remove(&header.hash);
        }
    }

    pub fn set_transaction_count(&self, addr: [u8; 20], tag: BlockTag, count: u64) {
        let key = (addr, block_tag_key(tag));
        self.inner.lock().unwrap().nonce_by_addr.insert(key, count);
    }

    /// Recorded write method invocations. Every element should be
    /// `"eth_sendRawTransaction"` — that is the ONLY write method the
    /// mock offers.
    pub fn write_method_calls(&self) -> Vec<&'static str> {
        self.inner.lock().unwrap().write_calls.clone()
    }
}

fn block_tag_key(tag: BlockTag) -> String {
    match tag {
        BlockTag::Latest => "latest".to_string(),
        BlockTag::Pending => "pending".to_string(),
        BlockTag::Number(n) => format!("n:{n}"),
        BlockTag::Hash(h) => format!("h:0x{}", hex_encode(&h)),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[async_trait]
impl ExecutionBroadcastRpcClient for MockBroadcastRpc {
    async fn chain_id(&self) -> Result<u64, BroadcastRpcError> {
        Ok(self.inner.lock().unwrap().chain_id)
    }
    async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
        Ok(self.inner.lock().unwrap().head)
    }
    async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
        Ok(self.inner.lock().unwrap().finalized)
    }
    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, BroadcastRpcError> {
        let key = (address, block_tag_key(block_tag));
        Ok(self
            .inner
            .lock()
            .unwrap()
            .nonce_by_addr
            .get(&key)
            .copied()
            .unwrap_or(0))
    }
    async fn transaction_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TransactionSummary>, BroadcastRpcError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .transactions
            .get(&tx_hash)
            .cloned())
    }
    async fn receipt_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
        Ok(self.inner.lock().unwrap().receipts.get(&tx_hash).cloned())
    }
    async fn block_header_by_number(
        &self,
        number: u64,
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .headers_by_number
            .get(&number)
            .cloned())
    }
    async fn block_header_by_hash(
        &self,
        hash: [u8; 32],
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .headers_by_hash
            .get(&hash)
            .cloned())
    }
    async fn send_raw_transaction(
        &self,
        _raw_tx_bytes: &[u8],
    ) -> Result<SendOutcome, BroadcastRpcError> {
        let mut g = self.inner.lock().unwrap();
        g.write_calls.push("eth_sendRawTransaction");
        // Strict FIFO: every call pops one outcome. Empty queue is a
        // configuration bug in the caller — surface as Malformed so
        // tests notice immediately.
        if g.send_outcomes.is_empty() {
            return Err(BroadcastRpcError::Malformed(
                "mock: no send outcome configured".into(),
            ));
        }
        g.send_outcomes.remove(0)
    }
}

// Dummy #[test] so this file is picked up as a test binary (Rust
// integration test files must have at least one #[test] fn).
#[test]
fn mock_broadcast_rpc_smoke() {
    let m = MockBroadcastRpc::new();
    m.set_head(1);
    assert!(m.write_method_calls().is_empty());
}
