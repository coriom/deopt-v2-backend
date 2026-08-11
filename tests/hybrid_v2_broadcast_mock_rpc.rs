//! Reusable deterministic broadcast RPC mock for the Hybrid V2
//! broadcast lifecycle PG integration tests (Package B Parts J–P and
//! Package C Parts Q–U).
//!
//! Frozen safety: `MockBroadcastRpc` accepts `eth_sendRawTransaction`
//! (returns a configurable outcome) and records every write invocation
//! so tests can assert `write_method_calls() == ["eth_sendRawTransaction"]`.
//! Never actually reaches a public chain.
//!
//! Package C (Part U) extended the harness with fault-injection setters
//! for delayed acceptance, connection reset, wrong-hash acceptance,
//! receipt disappearance / reorg, transaction drop, provider 429 / 5xx
//! bursts, and a malformed-receipt sentinel. All fault state is
//! deterministic — no thread-local randomness, no sleep-based races.

#![allow(dead_code)]

use async_trait::async_trait;
use deopt_v2_backend::hybrid_v2::execution::broadcast_rpc::{
    BlockHeader, BroadcastRpcError, ExecutionBroadcastRpcClient, SendOutcome, TransactionSummary,
    TxReceipt,
};
use deopt_v2_backend::hybrid_v2::execution::rpc::BlockTag;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

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
    /// Non-`eth_sendRawTransaction` write methods observed. MUST be
    /// empty at end of every test — the mock only exposes
    /// `eth_sendRawTransaction` as a write. Retained so the Part T
    /// public boundary audit can assert it.
    non_send_write_calls: Vec<&'static str>,
    /// Per-method call counter (all methods, read+write).
    call_counter: HashMap<&'static str, usize>,
    receipts: HashMap<[u8; 32], TxReceipt>,
    transactions: HashMap<[u8; 32], TransactionSummary>,
    headers_by_number: HashMap<u64, BlockHeader>,
    headers_by_hash: HashMap<[u8; 32], BlockHeader>,
    head: u64,
    finalized: Option<u64>,
    nonce_by_addr: HashMap<([u8; 20], String), u64>,
    chain_id: u64,
    /// Optional delay applied to the next `send_raw_transaction` call
    /// only. Cleared after one use.
    next_send_delay: Option<Duration>,
    /// If set, the next `send_raw_transaction` call returns a
    /// connection-reset transport error and clears the flag.
    next_send_connection_reset: bool,
    /// If Some, next send returns Accepted with THIS hash regardless of
    /// bytes. Used to test the outbox's PROVIDER_HASH_MISMATCH path.
    next_send_wrong_hash: Option<[u8; 32]>,
    /// After N successful receipt lookups for `tx_hash`, subsequent
    /// lookups return `None` (simulates a reorg dropping the receipt).
    receipt_disappearance: HashMap<[u8; 32], u32>,
    /// After the next receipt lookup for `tx_hash`, replace the
    /// receipt's `block_number` / `block_hash` (simulates a receipt
    /// block reorg).
    receipt_block_reorg: HashMap<[u8; 32], (u64, [u8; 32])>,
    /// `tx_by_hash` returns `None` for these hashes (simulates dropped
    /// tx) even when `transactions` contains an entry.
    dropped_transactions: std::collections::HashSet<[u8; 32]>,
    /// The next N receipt/tx-lookup responses return a 500-class
    /// unavailable error.
    provider_5xx_remaining: u32,
    /// The next N receipt/tx-lookup responses return a 429 (rate
    /// limited) error.
    provider_429_remaining: u32,
    /// If Some, receipt lookup for `tx_hash` returns MalformedResponse
    /// carrying the supplied string.
    malformed_receipt: HashMap<[u8; 32], String>,
}

impl Default for MockBroadcastRpc {
    fn default() -> Self {
        Self {
            inner: Mutex::new(MockInner {
                send_outcomes: Vec::new(),
                write_calls: Vec::new(),
                non_send_write_calls: Vec::new(),
                call_counter: HashMap::new(),
                receipts: HashMap::new(),
                transactions: HashMap::new(),
                headers_by_number: HashMap::new(),
                headers_by_hash: HashMap::new(),
                head: 100,
                finalized: Some(90),
                nonce_by_addr: HashMap::new(),
                chain_id: 84532,
                next_send_delay: None,
                next_send_connection_reset: false,
                next_send_wrong_hash: None,
                receipt_disappearance: HashMap::new(),
                receipt_block_reorg: HashMap::new(),
                dropped_transactions: std::collections::HashSet::new(),
                provider_5xx_remaining: 0,
                provider_429_remaining: 0,
                malformed_receipt: HashMap::new(),
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

    /// Non-`eth_sendRawTransaction` write invocations. MUST be empty at
    /// the end of every test — the mock refuses to forward any other
    /// write. Retained as a public accessor so tests can assert the
    /// invariant explicitly.
    pub fn non_send_write_methods(&self) -> Vec<&'static str> {
        self.inner.lock().unwrap().non_send_write_calls.clone()
    }

    /// Prohibited-call surface for compatibility with existing tests.
    /// Retained as an alias for `non_send_write_methods` — both are
    /// asserted `is_empty()` at end of every mock-RPC test.
    pub fn prohibited_calls_seen(&self) -> Vec<&'static str> {
        self.non_send_write_methods()
    }

    /// Per-method call counter. Useful for asserting bounded retries.
    pub fn expected_call_count(&self, method: &'static str) -> usize {
        self.inner
            .lock()
            .unwrap()
            .call_counter
            .get(method)
            .copied()
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------
    //  Part U — fault-injection setters
    // -----------------------------------------------------------------

    /// Queue N `send_raw_transaction` outcomes in FIFO order. Convenience
    /// helper equivalent to N invocations of `set_next_send_outcome`.
    pub fn set_next_send_outcome_sequence(
        &self,
        outcomes: Vec<Result<SendOutcome, BroadcastRpcError>>,
    ) {
        let mut g = self.inner.lock().unwrap();
        for o in outcomes {
            g.send_outcomes.push(o);
        }
    }

    /// Apply a fixed delay to the NEXT `send_raw_transaction` call only.
    /// Simulates a slow provider without changing the outcome shape.
    pub fn set_next_send_delay(&self, delay: Duration) {
        self.inner.lock().unwrap().next_send_delay = Some(delay);
    }

    /// If `true`, the next `send_raw_transaction` returns a
    /// connection-reset transport error and clears the flag. Simulates
    /// a mid-send TCP RST.
    pub fn set_next_send_connection_reset(&self, on: bool) {
        self.inner.lock().unwrap().next_send_connection_reset = on;
    }

    /// SECURITY test hook — if `on`, the next `send_raw_transaction`
    /// returns `Accepted { provider_tx_hash = wrong_hash }` regardless
    /// of the input bytes. Clears after one use.
    pub fn set_next_send_wrong_hash(&self, on: bool, wrong_hash: [u8; 32]) {
        self.inner.lock().unwrap().next_send_wrong_hash = if on { Some(wrong_hash) } else { None };
    }

    /// After `after_n` successful receipt lookups for `tx_hash`,
    /// subsequent lookups return `None` (simulates reorg dropping the
    /// receipt).
    pub fn set_receipt_disappearance(&self, tx_hash: [u8; 32], after_n: u32) {
        self.inner
            .lock()
            .unwrap()
            .receipt_disappearance
            .insert(tx_hash, after_n);
    }

    /// Overwrite the receipt block for `tx_hash` on the NEXT lookup —
    /// simulates the receipt block hash / number changing under a
    /// reorg. The stored receipt is mutated in place; the setter is
    /// idempotent.
    pub fn set_receipt_block_reorg(
        &self,
        tx_hash: [u8; 32],
        new_block: u64,
        new_hash: [u8; 32],
    ) {
        self.inner
            .lock()
            .unwrap()
            .receipt_block_reorg
            .insert(tx_hash, (new_block, new_hash));
    }

    /// `tx_by_hash` returns `None` for this hash regardless of any
    /// prior `set_transaction_response`. Simulates a dropped tx.
    pub fn set_transaction_dropped(&self, tx_hash: [u8; 32]) {
        self.inner.lock().unwrap().dropped_transactions.insert(tx_hash);
    }

    /// The next `n` receipt / transaction lookups return an unavailable
    /// (5xx) error. Every call decrements the counter until zero.
    pub fn set_provider_5xx_next(&self, n: u32) {
        self.inner.lock().unwrap().provider_5xx_remaining = n;
    }

    /// The next `n` receipt / transaction lookups return a rate-limited
    /// (429) error. Every call decrements the counter until zero.
    pub fn set_provider_429_next(&self, n: u32) {
        self.inner.lock().unwrap().provider_429_remaining = n;
    }

    /// Receipt lookup for `tx_hash` returns `BroadcastRpcError::Malformed`
    /// carrying the supplied detail. Cleared after one use.
    pub fn set_malformed_receipt_response(&self, tx_hash: [u8; 32], detail: String) {
        self.inner
            .lock()
            .unwrap()
            .malformed_receipt
            .insert(tx_hash, detail);
    }

    /// Set head + finalized in one call for reorg-depth scenarios.
    pub fn set_head_finalized(&self, head: u64, finalized: Option<u64>) {
        let mut g = self.inner.lock().unwrap();
        g.head = head;
        g.finalized = finalized;
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
        bump_counter(self, "eth_chainId");
        Ok(self.inner.lock().unwrap().chain_id)
    }
    async fn head_block_number(&self) -> Result<u64, BroadcastRpcError> {
        bump_counter(self, "eth_blockNumber");
        Ok(self.inner.lock().unwrap().head)
    }
    async fn finalized_block_number(&self) -> Result<Option<u64>, BroadcastRpcError> {
        bump_counter(self, "eth_getBlockByNumber");
        Ok(self.inner.lock().unwrap().finalized)
    }
    async fn transaction_count(
        &self,
        address: [u8; 20],
        block_tag: BlockTag,
    ) -> Result<u64, BroadcastRpcError> {
        bump_counter(self, "eth_getTransactionCount");
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
        bump_counter(self, "eth_getTransactionByHash");
        if let Some(err) = take_fault_5xx_429(self) {
            return Err(err);
        }
        let g = self.inner.lock().unwrap();
        if g.dropped_transactions.contains(&tx_hash) {
            return Ok(None);
        }
        Ok(g.transactions.get(&tx_hash).cloned())
    }
    async fn receipt_by_hash(
        &self,
        tx_hash: [u8; 32],
    ) -> Result<Option<TxReceipt>, BroadcastRpcError> {
        bump_counter(self, "eth_getTransactionReceipt");
        if let Some(err) = take_fault_5xx_429(self) {
            return Err(err);
        }
        // Malformed sentinel takes precedence.
        if let Some(detail) = self
            .inner
            .lock()
            .unwrap()
            .malformed_receipt
            .remove(&tx_hash)
        {
            return Err(BroadcastRpcError::Malformed(detail));
        }
        // Receipt block reorg: mutate stored receipt in place and clear
        // the reorg override so this happens exactly once.
        let reorg = self
            .inner
            .lock()
            .unwrap()
            .receipt_block_reorg
            .remove(&tx_hash);
        if let Some((new_block, new_hash)) = reorg {
            if let Some(r) = self.inner.lock().unwrap().receipts.get_mut(&tx_hash) {
                r.block_number = new_block;
                r.block_hash = new_hash;
            }
        }
        // Receipt disappearance: decrement counter; on zero, drop the
        // receipt entirely.
        let disappearing = {
            let mut g = self.inner.lock().unwrap();
            let n = g.receipt_disappearance.get(&tx_hash).copied();
            match n {
                Some(0) => {
                    g.receipts.remove(&tx_hash);
                    g.receipt_disappearance.remove(&tx_hash);
                    true
                }
                Some(count) => {
                    g.receipt_disappearance.insert(tx_hash, count - 1);
                    false
                }
                None => false,
            }
        };
        if disappearing {
            return Ok(None);
        }
        Ok(self.inner.lock().unwrap().receipts.get(&tx_hash).cloned())
    }
    async fn block_header_by_number(
        &self,
        number: u64,
    ) -> Result<Option<BlockHeader>, BroadcastRpcError> {
        bump_counter(self, "eth_getBlockByNumber");
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
        bump_counter(self, "eth_getBlockByHash");
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
        // Fault injection: pull one-shot flags OUT of the lock BEFORE
        // sleeping so we don't hold the mutex across `.await`.
        let delay;
        let wrong_hash;
        let reset;
        {
            let mut g = self.inner.lock().unwrap();
            delay = g.next_send_delay.take();
            wrong_hash = g.next_send_wrong_hash.take();
            reset = g.next_send_connection_reset;
            if reset {
                g.next_send_connection_reset = false;
            }
        }
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }

        let mut g = self.inner.lock().unwrap();
        g.write_calls.push("eth_sendRawTransaction");
        *g.call_counter.entry("eth_sendRawTransaction").or_insert(0) += 1;

        if reset {
            return Err(BroadcastRpcError::Transport("connection reset".into()));
        }
        if let Some(h) = wrong_hash {
            return Ok(SendOutcome::Accepted {
                provider_tx_hash: h,
            });
        }
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

fn bump_counter(mock: &MockBroadcastRpc, method: &'static str) {
    let mut g = mock.inner.lock().unwrap();
    *g.call_counter.entry(method).or_insert(0) += 1;
}

/// Pop one 5xx / 429 fault if the corresponding counter is >0. Used at
/// the top of receipt / tx-by-hash so bounded retries can be exercised.
fn take_fault_5xx_429(mock: &MockBroadcastRpc) -> Option<BroadcastRpcError> {
    let mut g = mock.inner.lock().unwrap();
    if g.provider_5xx_remaining > 0 {
        g.provider_5xx_remaining -= 1;
        return Some(BroadcastRpcError::Unavailable("mock: http 502".into()));
    }
    if g.provider_429_remaining > 0 {
        g.provider_429_remaining -= 1;
        return Some(BroadcastRpcError::RateLimited);
    }
    None
}

// Dummy #[test] so this file is picked up as a test binary (Rust
// integration test files must have at least one #[test] fn).
#[test]
fn mock_broadcast_rpc_smoke() {
    let m = MockBroadcastRpc::new();
    m.set_head(1);
    assert!(m.write_method_calls().is_empty());
    // The mock exposes ONLY `eth_sendRawTransaction` as a write —
    // non_send_write_methods must be empty on a fresh instance and stay
    // empty for the lifetime of the mock.
    assert!(m.non_send_write_methods().is_empty());
    assert!(m.prohibited_calls_seen().is_empty());
    // Fault-injection setters compile and clear correctly.
    m.set_next_send_delay(Duration::from_millis(0));
    m.set_next_send_connection_reset(true);
    m.set_next_send_connection_reset(false);
    m.set_next_send_wrong_hash(false, [0u8; 32]);
    m.set_provider_5xx_next(0);
    m.set_provider_429_next(0);
    m.set_head_finalized(2, Some(1));
    let _ = m.expected_call_count("eth_sendRawTransaction");
}
