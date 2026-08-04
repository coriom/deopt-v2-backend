//! Chain block source abstraction for the Hybrid V2 indexer runtime.
//!
//! Frozen rules:
//! - A `ChainSource` is a READ-ONLY interface. No signing, no chain
//!   writes, no state-changing RPC.
//! - The runtime asks for one block at a time. Bounded batches.
//! - Deterministic in-memory implementation is used for tests +
//!   local rebuild fixtures.
//! - Real RPC providers implement the trait behind an async HTTP/WS
//!   client; error paths must return typed `ChainSourceError` so the
//!   runtime never conflates a transient transport failure with a
//!   canonical projection failure.

use crate::hybrid_v2::decoder::CanonicalRawLog;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// A canonical block with its included Hybrid V2 logs. The runtime only
/// cares about the (block hash, parent hash, timestamp, logs) tuple; any
/// non-hybrid transactions are filtered out upstream by an emitter filter
/// keyed on the manifest module addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawBlock {
    pub number: u64,
    pub hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
    pub logs: Vec<CanonicalRawLog>,
}

/// Typed classification of `ChainSource` failures. Runtime code MUST
/// distinguish a transient transport / provider failure (retryable at
/// the worker layer) from a deterministic protocol failure (chain-id
/// mismatch, malformed response) which is unlikely to succeed on retry.
///
/// Frozen posture: no error variant carries an RPC URL, credentials,
/// or full-response body. `Malformed` and `Transport` accept short
/// descriptions with no secrets.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChainSourceError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("timeout")]
    Timeout,
    #[error("rate-limited (HTTP 429)")]
    RateLimited,
    #[error("provider server error (HTTP {status})")]
    ServerError { status: u16 },
    #[error("malformed response: {0}")]
    Malformed(String),
    #[error("json-rpc error {code}: {message}")]
    RpcError { code: i64, message: String },
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("cancelled")]
    Cancelled,
}

/// Trait implemented by any read-only block source (in-memory fixtures,
/// HTTP RPC, WebSocket subscriber…).
///
/// Frozen: THIS TRAIT MUST HAVE NO WRITE / SIGN / BROADCAST METHOD.
/// Adding a state-changing method here breaks
/// `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY`.
#[async_trait]
pub trait ChainSource: Send + Sync {
    /// Chain id declared by this source. Compared against the manifest.
    async fn chain_id(&self) -> Result<u64, ChainSourceError>;
    /// Highest observed head block number.
    async fn head_block_number(&self) -> Result<u64, ChainSourceError>;
    /// The last finalised block on this source. May be zero for local
    /// fixtures (no finality assumed).
    async fn finalized_block_number(&self) -> Result<u64, ChainSourceError> {
        Ok(0)
    }
    /// Fetch a specific canonical block by number on the current best
    /// chain. Returns `Ok(None)` when the number is beyond the head.
    async fn block_at(&self, number: u64) -> Result<Option<RawBlock>, ChainSourceError>;
    /// Fetch a specific block by hash — used for reorg replays where the
    /// number alone would ambiguate.
    async fn block_by_hash(&self, hash: &str) -> Result<Option<RawBlock>, ChainSourceError>;
}

/// Deterministic in-memory chain source. Supports registering multiple
/// "forks" so reorg tests can walk between them.
#[derive(Debug, Default, Clone)]
pub struct InMemoryChainSource {
    pub chain_id: u64,
    pub best_chain: Vec<RawBlock>,
    pub by_hash: BTreeMap<String, RawBlock>,
    pub finalized: u64,
}

impl InMemoryChainSource {
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            best_chain: Vec::new(),
            by_hash: BTreeMap::new(),
            finalized: 0,
        }
    }

    /// Append a block on the current best chain. Sets parent_hash to the
    /// previous head's hash automatically when left empty.
    pub fn push(&mut self, mut block: RawBlock) -> &mut Self {
        if block.parent_hash.is_empty() {
            block.parent_hash = self
                .best_chain
                .last()
                .map(|b| b.hash.clone())
                .unwrap_or_default();
        }
        self.by_hash.insert(block.hash.clone(), block.clone());
        self.best_chain.push(block);
        self
    }

    /// Replace the best chain from `start_number` onwards with the given
    /// blocks. Retains the orphaned blocks in `by_hash` so a runtime
    /// asking for them by hash still finds them.
    pub fn reorg_from(&mut self, start_number: u64, replacement: Vec<RawBlock>) -> &mut Self {
        self.best_chain.retain(|b| b.number < start_number);
        for block in replacement {
            self.by_hash.insert(block.hash.clone(), block.clone());
            self.best_chain.push(block);
        }
        self
    }

    pub fn set_finalized(&mut self, block: u64) -> &mut Self {
        self.finalized = block;
        self
    }
}

#[async_trait]
impl ChainSource for InMemoryChainSource {
    async fn chain_id(&self) -> Result<u64, ChainSourceError> {
        Ok(self.chain_id)
    }
    async fn head_block_number(&self) -> Result<u64, ChainSourceError> {
        Ok(self.best_chain.last().map(|b| b.number).unwrap_or(0))
    }
    async fn block_at(&self, number: u64) -> Result<Option<RawBlock>, ChainSourceError> {
        Ok(self.best_chain.iter().find(|b| b.number == number).cloned())
    }
    async fn block_by_hash(&self, hash: &str) -> Result<Option<RawBlock>, ChainSourceError> {
        Ok(self.by_hash.get(hash).cloned())
    }
    async fn finalized_block_number(&self) -> Result<u64, ChainSourceError> {
        Ok(self.finalized)
    }
}
