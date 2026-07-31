//! Options execution correlator.
//!
//! Groups the events that describe a single completed match:
//!
//! - one `OptionOrderPairExecuted` from the matching engine
//! - two `OptionOrderFilled` (buyer + seller)
//! - vault mutations: `OptionPremiumTransferred`, `OptionFeeCharged`,
//!   `OptionRebatePaid` (fees/rebates are optional)
//! - reservation mutations: `CollateralUnlocked` / `CollateralLocked`
//!   (may appear during margin rebalancing)
//!
//! Frozen rules:
//! - A correlation group cannot cross a transaction boundary.
//! - `execution_id` (topic1 of `OptionOrderPairExecuted`) is the anchor.
//! - Buyer and seller order fills MUST refer to the buyer/seller order
//!   hashes present in the pair-executed payload.
//! - Missing mandatory components → `Incomplete`.
//! - Duplicate events do not duplicate the group.
//! - Multiple executions in one transaction are distinguished by
//!   `execution_id`.
//! - Reorg removal invalidates the group (marked `InvalidatedByReorg`).

use crate::hybrid_v2::events::{EventKind, HybridV2Event};
use crate::hybrid_v2::reducer::{
    ApplyContext, ExecutionCompletion, MatchedExecutionRow, ProjectionState,
};
use std::collections::BTreeMap;

/// A single per-transaction correlation record.
#[derive(Debug, Clone, Default)]
pub struct ExecutionGroup {
    pub execution_id: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    pub matched_qty_1e8: String,
    pub premium_amount: String,
    pub fee_amount: String,
    pub rebate_amount: String,
    pub buyer_fill_seen: bool,
    pub seller_fill_seen: bool,
    pub premium_seen: bool,
    pub pair_executed_seen: bool,
}

impl ExecutionGroup {
    pub fn is_complete(&self) -> bool {
        self.pair_executed_seen
            && self.buyer_fill_seen
            && self.seller_fill_seen
            && self.premium_seen
    }
}

/// Streaming correlator invoked once per typed event during per-block
/// reduction. Emits completed groups into `ProjectionState.matched_executions`.
#[derive(Debug, Default)]
pub struct ExecutionCorrelator {
    /// Groups still being assembled, keyed on (tx_hash, execution_id).
    pending: BTreeMap<(String, String), ExecutionGroup>,
}

impl ExecutionCorrelator {
    pub fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }

    /// Feed one event into the correlator. When a group becomes complete
    /// this method writes a canonical row into `state.matched_executions`.
    pub fn observe(
        &mut self,
        event: &HybridV2Event,
        ctx: &ApplyContext,
        state: &mut ProjectionState,
    ) {
        match event.kind {
            EventKind::OptionOrderPairExecuted => {
                let exec_id = match event.execution_id.clone() {
                    Some(id) => id,
                    None => return,
                };
                let key = (ctx.tx_hash.clone(), exec_id.clone());
                let g = self.pending.entry(key.clone()).or_default();
                g.execution_id = exec_id;
                g.tx_hash = ctx.tx_hash.clone();
                g.block_number = ctx.block_number;
                g.pair_executed_seen = true;
                if let Some(hash) = event
                    .payload
                    .get("buyer_order_hash")
                    .and_then(|v| v.as_str())
                {
                    g.buyer_order_hash = hash.to_string();
                }
                if let Some(hash) = event
                    .payload
                    .get("seller_order_hash")
                    .and_then(|v| v.as_str())
                {
                    g.seller_order_hash = hash.to_string();
                }
                if let Some(sk) = event.payload.get("buyer_subkey").and_then(|v| v.as_str()) {
                    g.buyer_subkey = sk.to_string();
                }
                if let Some(sk) = event
                    .payload
                    .get("seller_subkey")
                    .and_then(|v| v.as_str())
                    .or_else(|| event.subkey.as_deref())
                {
                    g.seller_subkey = sk.to_string();
                }
                if let Some(qty) = event
                    .payload
                    .get("matched_qty_1e8")
                    .and_then(|v| v.as_str())
                {
                    g.matched_qty_1e8 = qty.to_string();
                }
                if let Some(prem) = event.payload.get("premium_1e8").and_then(|v| v.as_str()) {
                    g.premium_amount = prem.to_string();
                }
                if let Some(s) = event.payload.get("series_id").and_then(|v| v.as_str()) {
                    g.series_id = s.to_string();
                }
                self.finalize_if_ready(key, state);
            }
            EventKind::OptionOrderFilled => {
                let order_hash = match event.order_hash.clone() {
                    Some(h) => h,
                    None => return,
                };
                for ((tx, _exec), g) in self.pending.iter_mut() {
                    if tx != &ctx.tx_hash {
                        continue;
                    }
                    if order_hash == g.buyer_order_hash {
                        g.buyer_fill_seen = true;
                    }
                    if order_hash == g.seller_order_hash {
                        g.seller_fill_seen = true;
                    }
                }
                // Finalisation check for every affected group.
                let keys: Vec<_> = self
                    .pending
                    .keys()
                    .filter(|(tx, _)| tx == &ctx.tx_hash)
                    .cloned()
                    .collect();
                for k in keys {
                    self.finalize_if_ready(k, state);
                }
            }
            EventKind::OptionPremiumTransferred => {
                let amount = event
                    .payload
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                for ((tx, _exec), g) in self.pending.iter_mut() {
                    if tx != &ctx.tx_hash {
                        continue;
                    }
                    if g.premium_seen {
                        continue;
                    }
                    if g.premium_amount == "0" {
                        g.premium_amount = amount.clone();
                    }
                    g.premium_seen = true;
                }
                let keys: Vec<_> = self
                    .pending
                    .keys()
                    .filter(|(tx, _)| tx == &ctx.tx_hash)
                    .cloned()
                    .collect();
                for k in keys {
                    self.finalize_if_ready(k, state);
                }
            }
            EventKind::OptionFeeCharged => {
                let amount = event
                    .payload
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                for ((tx, _exec), g) in self.pending.iter_mut() {
                    if tx == &ctx.tx_hash {
                        g.fee_amount = amount.clone();
                    }
                }
            }
            EventKind::OptionRebatePaid => {
                let amount = event
                    .payload
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();
                for ((tx, _exec), g) in self.pending.iter_mut() {
                    if tx == &ctx.tx_hash {
                        g.rebate_amount = amount.clone();
                    }
                }
            }
            _ => {}
        }
    }

    /// End-of-transaction hook. Any groups that are still incomplete get
    /// written as `INCOMPLETE` and dropped.
    pub fn flush_transaction(&mut self, tx_hash: &str, state: &mut ProjectionState) {
        let keys: Vec<_> = self
            .pending
            .keys()
            .filter(|(tx, _)| tx == tx_hash)
            .cloned()
            .collect();
        for key in keys {
            if let Some(group) = self.pending.remove(&key) {
                let row = MatchedExecutionRow {
                    buyer_order_hash: group.buyer_order_hash,
                    seller_order_hash: group.seller_order_hash,
                    buyer_subkey: group.buyer_subkey,
                    seller_subkey: group.seller_subkey,
                    series_id: group.series_id,
                    matched_qty_1e8: group.matched_qty_1e8,
                    premium_amount: group.premium_amount,
                    fee_amount: group.fee_amount,
                    rebate_amount: group.rebate_amount,
                    block_number: group.block_number,
                    tx_hash: group.tx_hash,
                    completion_status: if group.pair_executed_seen
                        && group.buyer_fill_seen
                        && group.seller_fill_seen
                        && group.premium_seen
                    {
                        ExecutionCompletion::Complete
                    } else {
                        ExecutionCompletion::Incomplete
                    },
                };
                state.matched_executions.insert(group.execution_id, row);
            }
        }
    }

    /// Invalidate every completed / pending group that landed in one of
    /// the orphaned blocks. Called by the reorg planner.
    pub fn invalidate_blocks(&mut self, state: &mut ProjectionState, orphaned_blocks: &[u64]) {
        // Mark completed rows as INVALIDATED_BY_REORG for the affected blocks.
        for (_id, row) in state.matched_executions.iter_mut() {
            if orphaned_blocks.contains(&row.block_number) {
                row.completion_status = ExecutionCompletion::InvalidatedByReorg;
            }
        }
        // Drop any pending groups whose tx belonged to an orphaned block.
        self.pending
            .retain(|(_tx, _exec), g| !orphaned_blocks.contains(&g.block_number));
    }

    fn finalize_if_ready(&mut self, key: (String, String), state: &mut ProjectionState) {
        if let Some(g) = self.pending.get(&key) {
            if g.is_complete() {
                if let Some(group) = self.pending.remove(&key) {
                    let row = MatchedExecutionRow {
                        buyer_order_hash: group.buyer_order_hash,
                        seller_order_hash: group.seller_order_hash,
                        buyer_subkey: group.buyer_subkey,
                        seller_subkey: group.seller_subkey,
                        series_id: group.series_id,
                        matched_qty_1e8: group.matched_qty_1e8,
                        premium_amount: group.premium_amount,
                        fee_amount: group.fee_amount,
                        rebate_amount: group.rebate_amount,
                        block_number: group.block_number,
                        tx_hash: group.tx_hash,
                        completion_status: ExecutionCompletion::Complete,
                    };
                    state.matched_executions.insert(group.execution_id, row);
                }
            }
        }
    }
}
