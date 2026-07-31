//! Chain block source abstraction for the Hybrid V2 indexer runtime.
//!
//! Frozen rules:
//! - A `ChainSource` is a READ-ONLY interface. No signing, no chain
//!   writes, no state-changing RPC.
//! - The runtime asks for one block at a time. Bounded batches.
//! - Deterministic in-memory implementation is used for tests +
//!   local rebuild fixtures.
//! - Real RPC providers implement the trait behind a HTTP/WS client;
//!   they land in the follow-up read-API milestone once the batching
//!   contract is exercised in production.

use crate::hybrid_v2::decoder::CanonicalRawLog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// Trait implemented by any read-only block source (in-memory fixtures,
/// HTTP RPC, WebSocket subscriber…).
pub trait ChainSource: Send + Sync {
    /// Chain id declared by this source. Compared against the manifest.
    fn chain_id(&self) -> u64;
    /// Highest observed head block number.
    fn head_block_number(&self) -> u64;
    /// Fetch a specific canonical block by number on the current best
    /// chain. Returns None when the number is beyond the head.
    fn block_at(&self, number: u64) -> Option<RawBlock>;
    /// Fetch a specific block by hash — used for reorg replays where the
    /// number alone would ambiguate.
    fn block_by_hash(&self, hash: &str) -> Option<RawBlock>;
    /// The last finalised block on this source. May be zero for local
    /// fixtures (no finality assumed).
    fn finalized_block_number(&self) -> u64 {
        0
    }
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

impl ChainSource for InMemoryChainSource {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }
    fn head_block_number(&self) -> u64 {
        self.best_chain.last().map(|b| b.number).unwrap_or(0)
    }
    fn block_at(&self, number: u64) -> Option<RawBlock> {
        self.best_chain.iter().find(|b| b.number == number).cloned()
    }
    fn block_by_hash(&self, hash: &str) -> Option<RawBlock> {
        self.by_hash.get(hash).cloned()
    }
    fn finalized_block_number(&self) -> u64 {
        self.finalized
    }
}
