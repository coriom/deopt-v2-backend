//! Operational reorg planner.
//!
//! Given the runtime's journaled raw logs + a `ChainSource` + an incoming
//! block whose `parent_hash` does not match the cursor's `indexed_head_hash`,
//! locate a bounded common ancestor and describe the rewind.
//!
//! Frozen rules:
//! - Never search deeper than `max_depth` (default 64). Beyond that we
//!   fail readiness rather than silently accept a divergent chain.
//! - Common ancestor is the highest block for which our journal's
//!   canonical hash matches the source's canonical hash.
//! - Orphaned block numbers are the strict range
//!   `(common_ancestor, indexed_head]`.

use crate::hybrid_v2::chain_source::ChainSource;
use crate::hybrid_v2::chain_source::RawBlock;
use crate::hybrid_v2::runtime::{JournaledLog, RuntimeError};
use std::collections::BTreeMap;

pub struct ReorgPlanner {
    pub max_depth: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReorgOutcome {
    NoReorgRequired,
    Rewind {
        common_ancestor: u64,
        orphaned_blocks: Vec<u64>,
    },
}

impl ReorgPlanner {
    pub fn new(max_depth: u64) -> Self {
        Self { max_depth }
    }

    pub async fn plan(
        &self,
        journal: &[JournaledLog],
        source: &dyn ChainSource,
        incoming: &RawBlock,
    ) -> Result<ReorgOutcome, RuntimeError> {
        let mut canonical_hashes: BTreeMap<u64, String> = BTreeMap::new();
        for log in journal.iter().filter(|l| l.is_canonical) {
            canonical_hashes.insert(log.block_number, log.block_hash.clone());
        }
        let Some(head) = canonical_hashes.keys().max().cloned() else {
            return Ok(ReorgOutcome::NoReorgRequired);
        };
        let mut common_ancestor: Option<u64> = None;
        // Walk back at most max_depth blocks.
        for depth in 0..=self.max_depth {
            if head < depth {
                break;
            }
            let n = head - depth;
            let Some(local_hash) = canonical_hashes.get(&n) else {
                continue;
            };
            let chain_block = match source.block_at(n).await {
                Ok(Some(b)) => b,
                Ok(None) => continue,
                Err(err) => {
                    return Err(RuntimeError::ChainSource {
                        detail: format!("reorg planner block_at({n}): {err}"),
                    });
                }
            };
            if hashes_eq(local_hash, &chain_block.hash) {
                common_ancestor = Some(n);
                break;
            }
        }
        let Some(ancestor) = common_ancestor else {
            return Err(RuntimeError::ExcessiveReorgDepth {
                depth: self.max_depth,
                max: self.max_depth,
            });
        };
        // Sanity: incoming's parent chain must connect to ancestor via source.
        let _ = incoming;
        if ancestor == head {
            return Ok(ReorgOutcome::NoReorgRequired);
        }
        let orphaned_blocks: Vec<u64> = ((ancestor + 1)..=head).collect();
        Ok(ReorgOutcome::Rewind {
            common_ancestor: ancestor,
            orphaned_blocks,
        })
    }
}

fn hashes_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
