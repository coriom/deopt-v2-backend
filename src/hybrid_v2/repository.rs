//! Internal Hybrid V2 query repository.
//!
//! Frozen rules:
//! - Every method is scoped to an explicit deployment.
//! - No mutations.
//! - Deterministic pagination via `PageCursor { last_key, limit }`.
//! - Uint256 values remain decimal strings — never `f64`.
//! - No public HTTP route in this milestone; the follow-up read-API
//!   milestone wraps these methods behind an axum handler.

use crate::hybrid_v2::reducer::{
    EscapeStateRow, FeeEventRow, MatchedExecutionRow, OrderLifecycleRow, PositionRow,
    ProjectionState, RecoveryStateProjection,
};
use crate::hybrid_v2::runtime::RuntimeCursor;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PAGE_LIMIT: usize = 100;
pub const MAX_PAGE_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageCursor {
    pub last_key: Option<String>,
    pub limit: usize,
}

impl PageCursor {
    pub fn new(limit: usize) -> Self {
        Self {
            last_key: None,
            limit: limit.clamp(1, MAX_PAGE_LIMIT),
        }
    }
    pub fn from_key(last_key: String, limit: usize) -> Self {
        Self {
            last_key: Some(last_key),
            limit: limit.clamp(1, MAX_PAGE_LIMIT),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentStatus {
    pub indexed_block: u64,
    pub observed_block: u64,
    pub finalized_block: u64,
    pub ready: bool,
    pub ready_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubaccountRow {
    pub subkey: String,
    pub owner: String,
    pub subaccount_id: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollateralBalanceRow {
    pub subkey: String,
    pub token: String,
    pub balance: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReservationRow {
    pub subkey: String,
    pub token: String,
    pub engine: String,
    pub reserved: String,
}

pub struct HybridV2QueryRepository<'a> {
    pub deployment_id: u64,
    pub state: &'a ProjectionState,
    pub cursor: &'a RuntimeCursor,
    pub ready: bool,
    pub ready_reason: Option<String>,
}

impl<'a> HybridV2QueryRepository<'a> {
    pub fn new(
        deployment_id: u64,
        state: &'a ProjectionState,
        cursor: &'a RuntimeCursor,
        ready: bool,
        ready_reason: Option<String>,
    ) -> Self {
        Self {
            deployment_id,
            state,
            cursor,
            ready,
            ready_reason,
        }
    }

    pub fn deployment_status(&self) -> DeploymentStatus {
        DeploymentStatus {
            indexed_block: self.cursor.indexed_head_block,
            observed_block: self.cursor.observed_head_block,
            finalized_block: self.cursor.finalized_head_block,
            ready: self.ready,
            ready_reason: self.ready_reason.clone(),
        }
    }

    pub fn subaccounts_by_owner(&self, owner: &str) -> Vec<SubaccountRow> {
        self.state
            .subaccounts
            .iter()
            .filter(|((o, _), _)| o.eq_ignore_ascii_case(owner))
            .map(|((o, id), sk)| SubaccountRow {
                subkey: sk.clone(),
                owner: o.clone(),
                subaccount_id: *id,
            })
            .collect()
    }

    pub fn subaccount_details(&self, subkey: &str) -> Option<SubaccountRow> {
        for ((owner, id), sk) in self.state.subaccounts.iter() {
            if sk.eq_ignore_ascii_case(subkey) {
                return Some(SubaccountRow {
                    subkey: sk.clone(),
                    owner: owner.clone(),
                    subaccount_id: *id,
                });
            }
        }
        None
    }

    pub fn collateral_balances(&self, subkey: &str) -> Vec<CollateralBalanceRow> {
        self.state
            .balances
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(subkey))
            .map(|((sk, token), balance)| CollateralBalanceRow {
                subkey: sk.clone(),
                token: token.clone(),
                balance: balance.clone(),
            })
            .collect()
    }

    pub fn reservations(&self, subkey: &str) -> Vec<ReservationRow> {
        self.state
            .reservations
            .iter()
            .filter(|((sk, _, _), _)| sk.eq_ignore_ascii_case(subkey))
            .map(|((sk, token, engine), reserved)| ReservationRow {
                subkey: sk.clone(),
                token: token.clone(),
                engine: engine.clone(),
                reserved: reserved.clone(),
            })
            .collect()
    }

    pub fn active_positions(&self, subkey: &str) -> Vec<(String, PositionRow)> {
        self.state
            .positions
            .iter()
            .filter(|((sk, _), _)| sk.eq_ignore_ascii_case(subkey))
            .map(|((_, series), row)| (series.clone(), row.clone()))
            .collect()
    }

    pub fn order_lifecycle(&self, order_hash: &str) -> Option<OrderLifecycleRow> {
        self.state.order_lifecycle.get(order_hash).cloned()
    }

    pub fn matched_executions(&self, page: &PageCursor) -> Vec<(String, MatchedExecutionRow)> {
        let mut out: Vec<_> = self
            .state
            .matched_executions
            .iter()
            .filter(|(id, _)| match &page.last_key {
                Some(k) => id.as_str() > k.as_str(),
                None => true,
            })
            .take(page.limit)
            .map(|(id, row)| (id.clone(), row.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn fee_events(&self, page: &PageCursor) -> Vec<FeeEventRow> {
        self.state
            .fee_events
            .iter()
            .filter(|row| match &page.last_key {
                Some(k) => format!("{}:{}", row.block_number, row.log_index).as_str() > k.as_str(),
                None => true,
            })
            .take(page.limit)
            .cloned()
            .collect()
    }

    pub fn recovery_state(&self, subkey: &str) -> RecoveryStateProjection {
        self.state
            .recovery_state
            .get(subkey)
            .copied()
            .unwrap_or(RecoveryStateProjection::Normal)
    }

    pub fn escape_state(&self, subkey: &str) -> Option<EscapeStateRow> {
        self.state.escape_state.get(subkey).cloned()
    }
}
