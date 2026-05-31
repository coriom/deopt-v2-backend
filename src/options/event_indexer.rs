use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::execution::{EthGetLogsFilter, EthLogsProvider, HttpJsonRpcProvider};
use crate::indexer::decoder::{decode_hex_bytes, hex_0x, hex_quantity, parse_hex_quantity};
use crate::indexer::EthLog;
use crate::signing::eip712::{keccak256, parse_evm_address};
use crate::types::{now_ms, AccountId};
use alloy_primitives::U256;
use serde::Serialize;
use std::collections::BTreeMap;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{OptionEventIndexerState, OptionExecutionEvent};

pub const OPTION_EVENT_INDEXER_STATE_ID: &str = "option_events_base_sepolia";
pub const OPTION_TRADE_EXECUTED_SIGNATURE: &str =
    "OptionTradeExecuted(bytes32,address,address,uint256,uint128,uint128,bool,uint256,uint256)";
pub const MARGIN_TRADE_EXECUTED_SIGNATURE: &str =
    "TradeExecuted(address,address,uint256,uint128,uint128)";
pub const TRADING_FEE_CHARGED_SIGNATURE: &str =
    "TradingFeeCharged(address,address,address,uint256,bool,uint256,uint256,uint256,uint256,uint256,bool)";
pub const INTERNAL_TRANSFER_SIGNATURE: &str = "InternalTransfer(address,address,address,uint256)";
pub const COLLATERAL_VAULT_DEPOSITED_SIGNATURE: &str = "Deposited(address,address,uint256)";
pub const COLLATERAL_VAULT_WITHDRAWN_SIGNATURE: &str = "Withdrawn(address,address,uint256)";
pub const COLLATERAL_VAULT_SYNCED_SIGNATURE: &str = "Synced(address,address,uint256)";
pub const MARGIN_COLLATERAL_DEPOSITED_SIGNATURE: &str =
    "CollateralDeposited(address,address,uint256)";
pub const MARGIN_COLLATERAL_WITHDRAWN_SIGNATURE: &str =
    "CollateralWithdrawn(address,address,uint256,uint256)";
pub const FEE_BPS_CAP_SET_SIGNATURE: &str = "FeeBpsCapSet(uint16,uint16)";
pub const DEFAULT_FEES_SET_SIGNATURE: &str = "DefaultFeesSet(uint16,uint16,uint16,uint16)";
pub const MERKLE_ROOT_SET_SIGNATURE: &str = "MerkleRootSet(bytes32,bytes32,uint64)";
pub const TIER_CLAIMED_SIGNATURE: &str = "TierClaimed(address,uint8,uint64,uint64)";
pub const OVERRIDE_SET_SIGNATURE: &str =
    "OverrideSet(address,uint16,uint16,uint16,uint16,uint64,bool)";

// V2D-E: FeesManagerV2 emits signed-ppm fee events. These are decoded only
// when the operator configures `fees_manager_v2_address`; V1 indexing is
// unchanged when V2 is not wired up.
pub const FEE_CHARGED_V2_SIGNATURE: &str =
    "FeeChargedV2(address,address,address,address,uint8,uint8,bool,int32,uint256,uint256)";
pub const FEE_REBATED_V2_SIGNATURE: &str =
    "FeeRebatedV2(address,address,address,address,uint8,uint8,int32,uint256,uint256)";
pub const REBATE_BUDGET_FUNDED_SIGNATURE: &str = "RebateBudgetFunded(address,uint256)";
pub const REBATE_BUDGET_WITHDRAWN_SIGNATURE: &str =
    "RebateBudgetWithdrawn(address,address,uint256)";
pub const REBATE_BUDGET_SPENT_SIGNATURE: &str = "RebateBudgetSpent(address,uint256)";
pub const FEE_RECIPIENT_SET_V2_SIGNATURE: &str = "FeeRecipientSet(address,address)";
pub const FEE_CONSUMER_SET_SIGNATURE: &str = "FeeConsumerSet(address,bool)";
pub const MERKLE_ROOT_SET_V2_SIGNATURE: &str = "MerkleRootSet(bytes32,uint64,uint64)";
pub const TIER_CLAIMED_V2_SIGNATURE: &str = "TierClaimed(address,uint8,uint64)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionEventIndexerConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub from_block: u64,
    pub batch_blocks: u64,
    pub confirmation_blocks: u64,
    pub require_rpc: bool,
    pub rpc_url: Option<String>,
    pub matching_engine_address: AccountId,
    pub margin_engine_address: AccountId,
    pub collateral_vault_address: AccountId,
    pub fees_manager_address: Option<AccountId>,
    /// Optional V2 fees manager (FeesManagerV2) address. When set, the
    /// indexer subscribes to the V2 signed-ppm fee events on this contract
    /// alongside any V1 events on `fees_manager_address`. V1 behavior is
    /// fully unchanged when this is `None`.
    pub fees_manager_v2_address: Option<AccountId>,
    /// V2G-F observability metadata: optional address of the legacy /
    /// stranded MarginEngine. Used solely by the
    /// `deopt_option_fee_charged_v2_total{consumer="old"}` metric and
    /// its rebated sibling — never used to route broadcast or
    /// execution traffic. `None` means "OLD address not configured"
    /// and any unmatched OPTION consumer is bucketed as `"unknown"`.
    pub old_margin_engine_address: Option<AccountId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionEventEmitterContract {
    pub role: String,
    pub contract_address: AccountId,
}

impl OptionEventIndexerConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            poll_interval_ms: 15_000,
            from_block: 0,
            batch_blocks: 1_000,
            confirmation_blocks: 3,
            require_rpc: true,
            rpc_url: None,
            matching_engine_address: AccountId::new(""),
            margin_engine_address: AccountId::new(""),
            collateral_vault_address: AccountId::new(""),
            fees_manager_address: None,
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
        }
    }

    pub fn emitter_contracts(&self) -> Vec<OptionEventEmitterContract> {
        let mut contracts = Vec::new();
        push_emitter_contract(
            &mut contracts,
            "matching_engine",
            &self.matching_engine_address,
        );
        push_emitter_contract(&mut contracts, "margin_engine", &self.margin_engine_address);
        push_emitter_contract(
            &mut contracts,
            "collateral_vault",
            &self.collateral_vault_address,
        );
        if let Some(address) = self.fees_manager_address.as_ref() {
            push_emitter_contract(&mut contracts, "fees_manager", address);
        }
        if let Some(address) = self.fees_manager_v2_address.as_ref() {
            push_emitter_contract(&mut contracts, "fees_manager_v2", address);
        }
        contracts
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.poll_interval_ms == 0 {
            return Err(BackendError::Config(
                "OPTION_EVENT_INDEXER_POLL_INTERVAL_MS must be greater than zero".to_string(),
            ));
        }
        if self.batch_blocks == 0 {
            return Err(BackendError::Config(
                "OPTION_EVENT_INDEXER_BATCH_BLOCKS must be greater than zero".to_string(),
            ));
        }
        if !self.enabled {
            return Ok(());
        }
        if !persistence_enabled {
            return Err(BackendError::Config(
                "option event indexer requires persistence enabled".to_string(),
            ));
        }
        if self.require_rpc && self.rpc_url.is_none() {
            return Err(BackendError::Config(
                "RPC_URL is required when OPTION_EVENT_INDEXER_ENABLED=true and OPTION_EVENT_INDEXER_REQUIRE_RPC=true".to_string(),
            ));
        }
        validate_required_emitter_address(
            "OPTION_EVENT_INDEXER_MATCHING_ENGINE_ADDRESS",
            &self.matching_engine_address,
        )?;
        validate_required_emitter_address(
            "OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS",
            &self.margin_engine_address,
        )?;
        validate_required_emitter_address(
            "OPTION_EVENT_INDEXER_COLLATERAL_VAULT_ADDRESS",
            &self.collateral_vault_address,
        )?;
        if let Some(address) = self.fees_manager_address.as_ref() {
            validate_optional_emitter_address(
                "OPTION_EVENT_INDEXER_FEES_MANAGER_ADDRESS",
                address,
            )?;
        }
        if let Some(address) = self.fees_manager_v2_address.as_ref() {
            validate_optional_emitter_address(
                "OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS",
                address,
            )?;
        }
        Ok(())
    }
}

fn push_emitter_contract(
    contracts: &mut Vec<OptionEventEmitterContract>,
    role: &str,
    address: &AccountId,
) {
    if !address.0.trim().is_empty() {
        contracts.push(OptionEventEmitterContract {
            role: role.to_string(),
            contract_address: AccountId::new(address.0.to_ascii_lowercase()),
        });
    }
}

fn validate_required_emitter_address(env_key: &str, address: &AccountId) -> Result<()> {
    if address.0.trim().is_empty() {
        return Err(BackendError::Config(format!(
            "{env_key} is required when OPTION_EVENT_INDEXER_ENABLED=true"
        )));
    }
    validate_optional_emitter_address(env_key, address)
}

fn validate_optional_emitter_address(env_key: &str, address: &AccountId) -> Result<()> {
    parse_evm_address(address).map_err(|_| {
        BackendError::Config(format!(
            "{env_key} must be a valid address when OPTION_EVENT_INDEXER_ENABLED=true"
        ))
    })?;
    if address
        .0
        .eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return Err(BackendError::Config(format!(
            "{env_key} must be nonzero when OPTION_EVENT_INDEXER_ENABLED=true"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OptionEventIndexerTickResult {
    pub enabled: bool,
    pub chain_id: u64,
    pub current_block_number: Option<u64>,
    pub safe_head: Option<u64>,
    pub from_block: u64,
    pub to_block: u64,
    pub batch_blocks: u64,
    pub confirmation_blocks: u64,
    pub logs_found: usize,
    pub events_decoded: usize,
    pub events_indexed: u64,
    pub cursor_updated: bool,
    pub last_indexed_block: u64,
}

pub fn option_trade_executed_topic0() -> String {
    hex_0x(&keccak256(OPTION_TRADE_EXECUTED_SIGNATURE.as_bytes()))
}

pub fn margin_trade_executed_topic0() -> String {
    hex_0x(&keccak256(MARGIN_TRADE_EXECUTED_SIGNATURE.as_bytes()))
}

pub fn trading_fee_charged_topic0() -> String {
    hex_0x(&keccak256(TRADING_FEE_CHARGED_SIGNATURE.as_bytes()))
}

pub fn internal_transfer_topic0() -> String {
    hex_0x(&keccak256(INTERNAL_TRANSFER_SIGNATURE.as_bytes()))
}

pub fn collateral_vault_deposited_topic0() -> String {
    hex_0x(&keccak256(COLLATERAL_VAULT_DEPOSITED_SIGNATURE.as_bytes()))
}

pub fn collateral_vault_withdrawn_topic0() -> String {
    hex_0x(&keccak256(COLLATERAL_VAULT_WITHDRAWN_SIGNATURE.as_bytes()))
}

pub fn collateral_vault_synced_topic0() -> String {
    hex_0x(&keccak256(COLLATERAL_VAULT_SYNCED_SIGNATURE.as_bytes()))
}

pub fn margin_collateral_deposited_topic0() -> String {
    hex_0x(&keccak256(MARGIN_COLLATERAL_DEPOSITED_SIGNATURE.as_bytes()))
}

pub fn margin_collateral_withdrawn_topic0() -> String {
    hex_0x(&keccak256(MARGIN_COLLATERAL_WITHDRAWN_SIGNATURE.as_bytes()))
}

pub fn fee_bps_cap_set_topic0() -> String {
    hex_0x(&keccak256(FEE_BPS_CAP_SET_SIGNATURE.as_bytes()))
}

pub fn default_fees_set_topic0() -> String {
    hex_0x(&keccak256(DEFAULT_FEES_SET_SIGNATURE.as_bytes()))
}

pub fn merkle_root_set_topic0() -> String {
    hex_0x(&keccak256(MERKLE_ROOT_SET_SIGNATURE.as_bytes()))
}

pub fn tier_claimed_topic0() -> String {
    hex_0x(&keccak256(TIER_CLAIMED_SIGNATURE.as_bytes()))
}

pub fn override_set_topic0() -> String {
    hex_0x(&keccak256(OVERRIDE_SET_SIGNATURE.as_bytes()))
}

pub fn fee_charged_v2_topic0() -> String {
    hex_0x(&keccak256(FEE_CHARGED_V2_SIGNATURE.as_bytes()))
}

pub fn fee_rebated_v2_topic0() -> String {
    hex_0x(&keccak256(FEE_REBATED_V2_SIGNATURE.as_bytes()))
}

pub fn rebate_budget_funded_topic0() -> String {
    hex_0x(&keccak256(REBATE_BUDGET_FUNDED_SIGNATURE.as_bytes()))
}

pub fn rebate_budget_withdrawn_topic0() -> String {
    hex_0x(&keccak256(REBATE_BUDGET_WITHDRAWN_SIGNATURE.as_bytes()))
}

pub fn rebate_budget_spent_topic0() -> String {
    hex_0x(&keccak256(REBATE_BUDGET_SPENT_SIGNATURE.as_bytes()))
}

pub fn fee_recipient_set_v2_topic0() -> String {
    hex_0x(&keccak256(FEE_RECIPIENT_SET_V2_SIGNATURE.as_bytes()))
}

pub fn fee_consumer_set_topic0() -> String {
    hex_0x(&keccak256(FEE_CONSUMER_SET_SIGNATURE.as_bytes()))
}

pub fn merkle_root_set_v2_topic0() -> String {
    hex_0x(&keccak256(MERKLE_ROOT_SET_V2_SIGNATURE.as_bytes()))
}

pub fn tier_claimed_v2_topic0() -> String {
    hex_0x(&keccak256(TIER_CLAIMED_V2_SIGNATURE.as_bytes()))
}

pub fn default_option_event_counts() -> BTreeMap<String, u64> {
    BTreeMap::from([
        ("CollateralDeposited".to_string(), 0),
        ("CollateralWithdrawn".to_string(), 0),
        ("DefaultFeesSet".to_string(), 0),
        ("Deposited".to_string(), 0),
        ("FeeBpsCapSet".to_string(), 0),
        ("FeeChargedV2".to_string(), 0),
        ("FeeConsumerSetV2".to_string(), 0),
        ("FeeRebatedV2".to_string(), 0),
        ("FeeRecipientSetV2".to_string(), 0),
        ("InternalTransfer".to_string(), 0),
        ("MerkleRootSet".to_string(), 0),
        ("MerkleRootSetV2".to_string(), 0),
        ("OptionPositionUpdated".to_string(), 0),
        ("OptionTradeExecuted".to_string(), 0),
        ("OverrideSet".to_string(), 0),
        ("RebateBudgetFunded".to_string(), 0),
        ("RebateBudgetSpent".to_string(), 0),
        ("RebateBudgetWithdrawn".to_string(), 0),
        ("TierClaimed".to_string(), 0),
        ("TierClaimedV2".to_string(), 0),
        ("TradeExecuted".to_string(), 0),
        ("TradingFeeCharged".to_string(), 0),
        ("Synced".to_string(), 0),
        ("Withdrawn".to_string(), 0),
    ])
}

fn event_topics_for_emitter_role(role: &str) -> Vec<String> {
    match role {
        "matching_engine" => vec![option_trade_executed_topic0()],
        "margin_engine" => vec![
            margin_trade_executed_topic0(),
            trading_fee_charged_topic0(),
            margin_collateral_deposited_topic0(),
            margin_collateral_withdrawn_topic0(),
        ],
        "collateral_vault" => vec![
            internal_transfer_topic0(),
            collateral_vault_deposited_topic0(),
            collateral_vault_withdrawn_topic0(),
            collateral_vault_synced_topic0(),
        ],
        "fees_manager" => vec![
            fee_bps_cap_set_topic0(),
            default_fees_set_topic0(),
            merkle_root_set_topic0(),
            tier_claimed_topic0(),
            override_set_topic0(),
        ],
        "fees_manager_v2" => vec![
            fee_charged_v2_topic0(),
            fee_rebated_v2_topic0(),
            rebate_budget_funded_topic0(),
            rebate_budget_withdrawn_topic0(),
            rebate_budget_spent_topic0(),
            fee_recipient_set_v2_topic0(),
            fee_consumer_set_topic0(),
            merkle_root_set_v2_topic0(),
            tier_claimed_v2_topic0(),
        ],
        _ => Vec::new(),
    }
}

pub async fn index_option_events_with_provider<P>(
    state: &AppState,
    provider: &P,
) -> Result<OptionEventIndexerTickResult>
where
    P: EthLogsProvider,
{
    let config = state.option_event_indexer_config.clone();
    if !config.enabled {
        return Ok(OptionEventIndexerTickResult {
            enabled: false,
            chain_id: state.chain_id,
            current_block_number: None,
            safe_head: None,
            from_block: config.from_block.saturating_add(1),
            to_block: config.from_block,
            batch_blocks: config.batch_blocks,
            confirmation_blocks: config.confirmation_blocks,
            logs_found: 0,
            events_decoded: 0,
            events_indexed: 0,
            cursor_updated: false,
            last_indexed_block: config.from_block,
        });
    }

    let current_block_number = provider.block_number().await?;
    let safe_head = current_block_number.saturating_sub(config.confirmation_blocks);
    let cursor = get_option_event_indexer_state(state).await?;
    let last_indexed_block = cursor
        .as_ref()
        .map(|state| state.last_indexed_block)
        .unwrap_or(config.from_block);
    let from_block = last_indexed_block.saturating_add(1);

    if from_block > safe_head {
        let result = OptionEventIndexerTickResult {
            enabled: true,
            chain_id: state.chain_id,
            current_block_number: Some(current_block_number),
            safe_head: Some(safe_head),
            from_block,
            to_block: last_indexed_block,
            batch_blocks: config.batch_blocks,
            confirmation_blocks: config.confirmation_blocks,
            logs_found: 0,
            events_decoded: 0,
            events_indexed: 0,
            cursor_updated: false,
            last_indexed_block,
        };
        publish_latest_tick(state, &result);
        return Ok(result);
    }

    let range_end = from_block
        .saturating_add(config.batch_blocks)
        .saturating_sub(1);
    let to_block = safe_head.min(range_end);
    let mut logs = Vec::new();
    for emitter in config.emitter_contracts() {
        for topic0 in event_topics_for_emitter_role(&emitter.role) {
            logs.extend(
                provider
                    .get_logs(EthGetLogsFilter {
                        from_block: hex_quantity(from_block),
                        to_block: hex_quantity(to_block),
                        address: emitter.contract_address.0.clone(),
                        topics: vec![topic0],
                    })
                    .await?,
            );
        }
    }
    let logs_found = logs.len();
    let mut events = Vec::with_capacity(logs.len());
    for log in logs {
        let Some(mut event) = decode_option_execution_event(&log, state.chain_id)? else {
            continue;
        };
        let link = find_option_execution_event_link(
            state,
            &event.tx_hash,
            event.onchain_intent_id.as_deref(),
        )
        .await?;
        event.intent_id = link.intent_id;
        event.option_execution_transaction_id = link.option_execution_transaction_id;
        events.push(event);
    }
    let events_decoded = events.len();
    let events_indexed =
        persist_option_execution_events_and_cursor(state, &events, to_block).await?;

    let result = OptionEventIndexerTickResult {
        enabled: true,
        chain_id: state.chain_id,
        current_block_number: Some(current_block_number),
        safe_head: Some(safe_head),
        from_block,
        to_block,
        batch_blocks: config.batch_blocks,
        confirmation_blocks: config.confirmation_blocks,
        logs_found,
        events_decoded,
        events_indexed,
        cursor_updated: true,
        last_indexed_block: to_block,
    };
    publish_latest_tick(state, &result);
    Ok(result)
}

pub async fn list_option_execution_events(
    state: &AppState,
    limit: u32,
) -> Result<Vec<OptionExecutionEvent>> {
    if let Some(repository) = state.repository.clone() {
        return repository.list_option_execution_events(limit).await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .list_option_execution_events(limit))
}

pub async fn summarize_option_execution_events(state: &AppState) -> Result<BTreeMap<String, u64>> {
    let mut counts = default_option_event_counts();
    let stored = if let Some(repository) = state.repository.clone() {
        repository.summarize_option_execution_events().await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .summarize_option_execution_events()
    };
    for (event_name, count) in stored {
        counts.insert(event_name, count);
    }
    Ok(counts)
}

pub async fn summarize_option_execution_events_by_contract_address(
    state: &AppState,
) -> Result<BTreeMap<String, u64>> {
    let stored = if let Some(repository) = state.repository.clone() {
        repository
            .summarize_option_execution_events_by_contract_address()
            .await?
    } else {
        state
            .options_store
            .lock()
            .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
            .summarize_option_execution_events_by_contract_address()
    };
    Ok(stored
        .into_iter()
        .map(|(address, count)| (address.to_ascii_lowercase(), count))
        .collect())
}

pub async fn get_option_event_indexer_state(
    state: &AppState,
) -> Result<Option<OptionEventIndexerState>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID))
}

async fn find_option_execution_event_link(
    state: &AppState,
    tx_hash: &str,
    onchain_intent_id: Option<&str>,
) -> Result<super::OptionExecutionEventLink> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .find_option_execution_event_link(tx_hash, onchain_intent_id)
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .find_option_execution_event_link(tx_hash, onchain_intent_id))
}

async fn persist_option_execution_events_and_cursor(
    state: &AppState,
    events: &[OptionExecutionEvent],
    last_indexed_block: u64,
) -> Result<u64> {
    let now = now_ms();
    if let Some(repository) = state.repository.clone() {
        return repository
            .persist_option_execution_events_and_cursor(
                OPTION_EVENT_INDEXER_STATE_ID,
                events,
                last_indexed_block,
                now,
            )
            .await;
    }
    Ok(state
        .options_store
        .lock()
        .map_err(|_| BackendError::Config("options store lock poisoned".to_string()))?
        .persist_option_execution_events_and_cursor(
            OPTION_EVENT_INDEXER_STATE_ID,
            events,
            last_indexed_block,
            now,
        ))
}

fn publish_latest_tick(state: &AppState, result: &OptionEventIndexerTickResult) {
    if let Ok(mut slot) = state.option_event_indexer_last_tick.lock() {
        *slot = Some(result.clone());
    }
}

pub fn decode_option_execution_event(
    log: &EthLog,
    chain_id: u64,
) -> Result<Option<OptionExecutionEvent>> {
    let Some(topic0) = log.topics.first() else {
        return Err(BackendError::Indexer("log missing topic0".to_string()));
    };
    if topic0.eq_ignore_ascii_case(&option_trade_executed_topic0()) {
        return decode_option_trade_executed_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&margin_trade_executed_topic0()) {
        return decode_margin_trade_executed_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&trading_fee_charged_topic0()) {
        return decode_trading_fee_charged_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&internal_transfer_topic0()) {
        return decode_internal_transfer_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&collateral_vault_deposited_topic0()) {
        return decode_vault_balance_log(
            log,
            chain_id,
            "Deposited",
            COLLATERAL_VAULT_DEPOSITED_SIGNATURE,
            "amount",
        )
        .map(Some);
    }
    if topic0.eq_ignore_ascii_case(&collateral_vault_withdrawn_topic0()) {
        return decode_vault_balance_log(
            log,
            chain_id,
            "Withdrawn",
            COLLATERAL_VAULT_WITHDRAWN_SIGNATURE,
            "amount",
        )
        .map(Some);
    }
    if topic0.eq_ignore_ascii_case(&collateral_vault_synced_topic0()) {
        return decode_vault_balance_log(
            log,
            chain_id,
            "Synced",
            COLLATERAL_VAULT_SYNCED_SIGNATURE,
            "newBalance",
        )
        .map(Some);
    }
    if topic0.eq_ignore_ascii_case(&margin_collateral_deposited_topic0()) {
        return decode_margin_collateral_deposited_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&margin_collateral_withdrawn_topic0()) {
        return decode_margin_collateral_withdrawn_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&fee_bps_cap_set_topic0()) {
        return decode_fee_bps_cap_set_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&default_fees_set_topic0()) {
        return decode_default_fees_set_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&merkle_root_set_topic0()) {
        return decode_merkle_root_set_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&tier_claimed_topic0()) {
        return decode_tier_claimed_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&override_set_topic0()) {
        return decode_override_set_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&fee_charged_v2_topic0()) {
        return decode_fee_charged_v2_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&fee_rebated_v2_topic0()) {
        return decode_fee_rebated_v2_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&rebate_budget_funded_topic0()) {
        return decode_rebate_budget_funded_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&rebate_budget_withdrawn_topic0()) {
        return decode_rebate_budget_withdrawn_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&rebate_budget_spent_topic0()) {
        return decode_rebate_budget_spent_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&fee_recipient_set_v2_topic0()) {
        return decode_fee_recipient_set_v2_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&fee_consumer_set_topic0()) {
        return decode_fee_consumer_set_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&merkle_root_set_v2_topic0()) {
        return decode_merkle_root_set_v2_log(log, chain_id).map(Some);
    }
    if topic0.eq_ignore_ascii_case(&tier_claimed_v2_topic0()) {
        return decode_tier_claimed_v2_log(log, chain_id).map(Some);
    }
    Ok(None)
}

fn decode_option_trade_executed_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "OptionTradeExecuted log must have four topics".to_string(),
        ));
    }
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 6 {
        return Err(BackendError::Indexer(
            "OptionTradeExecuted data must contain six ABI words".to_string(),
        ));
    }
    let onchain_intent_id = decode_topic_bytes32(&log.topics[1])?;
    let buyer = decode_topic_address(&log.topics[2])?;
    let seller = decode_topic_address(&log.topics[3])?;
    let option_id = decode_data_u256(&data, 0)?.to_string();
    let quantity_contracts = decode_data_u256(&data, 1)?.to_string();
    let premium_per_contract_native = decode_data_u256(&data, 2)?.to_string();
    let buyer_is_maker = decode_bool(&data, 3)?;
    let buyer_nonce = decode_data_u256(&data, 4)?.to_string();
    let seller_nonce = decode_data_u256(&data, 5)?.to_string();
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "OptionTradeExecuted".to_string(),
        event_signature: OPTION_TRADE_EXECUTED_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: Some(onchain_intent_id.clone()),
        option_execution_transaction_id: None,
        buyer: Some(buyer.clone()),
        seller: Some(seller.clone()),
        account: None,
        option_id: Some(option_id.clone()),
        quantity_contracts: Some(quantity_contracts.clone()),
        premium_per_contract_native: Some(premium_per_contract_native.clone()),
        raw_topics: serde_json::Value::Array(
            log.topics
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "intentId": onchain_intent_id,
            "buyer": buyer,
            "seller": seller,
            "optionId": option_id,
            "quantity": quantity_contracts,
            "premiumPerContract": premium_per_contract_native,
            "buyerIsMaker": buyer_is_maker,
            "buyerNonce": buyer_nonce,
            "sellerNonce": seller_nonce
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn decode_margin_trade_executed_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "TradeExecuted log must have four topics".to_string(),
        ));
    }
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 2 {
        return Err(BackendError::Indexer(
            "TradeExecuted data must contain two ABI words".to_string(),
        ));
    }
    let buyer = decode_topic_address(&log.topics[1])?;
    let seller = decode_topic_address(&log.topics[2])?;
    let option_id = decode_topic_u256(&log.topics[3])?.to_string();
    let quantity_contracts = decode_data_u256(&data, 0)?.to_string();
    let premium_per_contract_native = decode_data_u256(&data, 1)?.to_string();
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "TradeExecuted".to_string(),
        event_signature: MARGIN_TRADE_EXECUTED_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer: Some(buyer.clone()),
        seller: Some(seller.clone()),
        account: None,
        option_id: Some(option_id.clone()),
        quantity_contracts: Some(quantity_contracts.clone()),
        premium_per_contract_native: Some(premium_per_contract_native.clone()),
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "buyer": buyer,
            "seller": seller,
            "optionId": option_id,
            "quantity": quantity_contracts,
            "price": premium_per_contract_native
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn decode_trading_fee_charged_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "TradingFeeCharged log must have four topics".to_string(),
        ));
    }
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 8 {
        return Err(BackendError::Indexer(
            "TradingFeeCharged data must contain eight ABI words".to_string(),
        ));
    }
    let trader = decode_topic_address(&log.topics[1])?;
    let recipient = decode_topic_address(&log.topics[2])?;
    let settlement_asset = decode_topic_address(&log.topics[3])?;
    let option_id = decode_data_u256(&data, 0)?.to_string();
    let is_maker = decode_bool(&data, 1)?;
    let premium = decode_data_u256(&data, 2)?.to_string();
    let notional_implicit = decode_data_u256(&data, 3)?.to_string();
    let notional_fee = decode_data_u256(&data, 4)?.to_string();
    let premium_cap_fee = decode_data_u256(&data, 5)?.to_string();
    let applied_fee = decode_data_u256(&data, 6)?.to_string();
    let capped_by_premium = decode_bool(&data, 7)?;
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "TradingFeeCharged".to_string(),
        event_signature: TRADING_FEE_CHARGED_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer: None,
        seller: None,
        account: Some(trader.clone()),
        option_id: Some(option_id.clone()),
        quantity_contracts: None,
        premium_per_contract_native: Some(premium.clone()),
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "trader": trader,
            "recipient": recipient,
            "settlementAsset": settlement_asset,
            "optionId": option_id,
            "isMaker": is_maker,
            "premium": premium,
            "notionalImplicit": notional_implicit,
            "notionalFee": notional_fee,
            "premiumCapFee": premium_cap_fee,
            "appliedFee": applied_fee,
            "cappedByPremium": capped_by_premium
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn decode_internal_transfer_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "InternalTransfer log must have four topics".to_string(),
        ));
    }
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "InternalTransfer data must contain one ABI word".to_string(),
        ));
    }
    let token = decode_topic_address(&log.topics[1])?;
    let from = decode_topic_address(&log.topics[2])?;
    let to = decode_topic_address(&log.topics[3])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "InternalTransfer".to_string(),
        event_signature: INTERNAL_TRANSFER_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer: None,
        seller: None,
        account: Some(from.clone()),
        option_id: None,
        quantity_contracts: None,
        premium_per_contract_native: Some(amount.clone()),
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "token": token,
            "from": from,
            "to": to,
            "amount": amount
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn decode_vault_balance_log(
    log: &EthLog,
    chain_id: u64,
    event_name: &str,
    event_signature: &str,
    amount_field: &str,
) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 3 {
        return Err(BackendError::Indexer(format!(
            "{event_name} log must have three topics"
        )));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(format!(
            "{event_name} data must contain one ABI word"
        )));
    }
    let user = decode_topic_address(&log.topics[1])?;
    let token = decode_topic_address(&log.topics[2])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    let mut decoded = serde_json::json!({
        "user": user,
        "token": token
    });
    if let Some(object) = decoded.as_object_mut() {
        object.insert(
            amount_field.to_string(),
            serde_json::Value::String(amount.clone()),
        );
    }
    option_event_from_decoded_fields(
        log,
        chain_id,
        event_name,
        event_signature,
        None,
        None,
        Some(user.clone()),
        None,
        None,
        Some(amount.clone()),
        decoded,
    )
}

fn decode_margin_collateral_deposited_log(
    log: &EthLog,
    chain_id: u64,
) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 3 {
        return Err(BackendError::Indexer(
            "CollateralDeposited log must have three topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "CollateralDeposited data must contain one ABI word".to_string(),
        ));
    }
    let trader = decode_topic_address(&log.topics[1])?;
    let token = decode_topic_address(&log.topics[2])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "CollateralDeposited",
        MARGIN_COLLATERAL_DEPOSITED_SIGNATURE,
        None,
        None,
        Some(trader.clone()),
        None,
        None,
        Some(amount.clone()),
        serde_json::json!({
            "trader": trader,
            "token": token,
            "amount": amount
        }),
    )
}

fn decode_margin_collateral_withdrawn_log(
    log: &EthLog,
    chain_id: u64,
) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 3 {
        return Err(BackendError::Indexer(
            "CollateralWithdrawn log must have three topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 2 {
        return Err(BackendError::Indexer(
            "CollateralWithdrawn data must contain two ABI words".to_string(),
        ));
    }
    let trader = decode_topic_address(&log.topics[1])?;
    let token = decode_topic_address(&log.topics[2])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    let margin_ratio_after_bps = decode_data_u256(&data, 1)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "CollateralWithdrawn",
        MARGIN_COLLATERAL_WITHDRAWN_SIGNATURE,
        None,
        None,
        Some(trader.clone()),
        None,
        None,
        Some(amount.clone()),
        serde_json::json!({
            "trader": trader,
            "token": token,
            "amount": amount,
            "marginRatioAfterBps": margin_ratio_after_bps
        }),
    )
}

fn decode_fee_bps_cap_set_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 1 {
        return Err(BackendError::Indexer(
            "FeeBpsCapSet log must have one topic".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 2 {
        return Err(BackendError::Indexer(
            "FeeBpsCapSet data must contain two ABI words".to_string(),
        ));
    }
    let old_cap = decode_data_u256(&data, 0)?.to_string();
    let new_cap = decode_data_u256(&data, 1)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "FeeBpsCapSet",
        FEE_BPS_CAP_SET_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "oldCap": old_cap,
            "newCap": new_cap
        }),
    )
}

fn decode_default_fees_set_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 1 {
        return Err(BackendError::Indexer(
            "DefaultFeesSet log must have one topic".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 4 {
        return Err(BackendError::Indexer(
            "DefaultFeesSet data must contain four ABI words".to_string(),
        ));
    }
    let maker_notional_fee_bps = decode_data_u256(&data, 0)?.to_string();
    let maker_premium_cap_bps = decode_data_u256(&data, 1)?.to_string();
    let taker_notional_fee_bps = decode_data_u256(&data, 2)?.to_string();
    let taker_premium_cap_bps = decode_data_u256(&data, 3)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "DefaultFeesSet",
        DEFAULT_FEES_SET_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "makerNotionalFeeBps": maker_notional_fee_bps,
            "makerPremiumCapBps": maker_premium_cap_bps,
            "takerNotionalFeeBps": taker_notional_fee_bps,
            "takerPremiumCapBps": taker_premium_cap_bps
        }),
    )
}

fn decode_merkle_root_set_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "MerkleRootSet log must have four topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if !data.is_empty() {
        return Err(BackendError::Indexer(
            "MerkleRootSet data must be empty".to_string(),
        ));
    }
    let old_root = decode_topic_bytes32(&log.topics[1])?;
    let new_root = decode_topic_bytes32(&log.topics[2])?;
    let new_epoch = decode_topic_u256(&log.topics[3])?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "MerkleRootSet",
        MERKLE_ROOT_SET_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "oldRoot": old_root,
            "newRoot": new_root,
            "newEpoch": new_epoch
        }),
    )
}

fn decode_tier_claimed_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "TierClaimed log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 3 {
        return Err(BackendError::Indexer(
            "TierClaimed data must contain three ABI words".to_string(),
        ));
    }
    let trader = decode_topic_address(&log.topics[1])?;
    let tier_class = decode_data_u256(&data, 0)?.to_string();
    let expiry = decode_data_u256(&data, 1)?.to_string();
    let epoch = decode_data_u256(&data, 2)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "TierClaimed",
        TIER_CLAIMED_SIGNATURE,
        None,
        None,
        Some(trader.clone()),
        None,
        None,
        None,
        serde_json::json!({
            "trader": trader,
            "tierClass": tier_class,
            "expiry": expiry,
            "epoch": epoch
        }),
    )
}

fn decode_override_set_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "OverrideSet log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 6 {
        return Err(BackendError::Indexer(
            "OverrideSet data must contain six ABI words".to_string(),
        ));
    }
    let trader = decode_topic_address(&log.topics[1])?;
    let maker_notional_fee_bps = decode_data_u256(&data, 0)?.to_string();
    let maker_premium_cap_bps = decode_data_u256(&data, 1)?.to_string();
    let taker_notional_fee_bps = decode_data_u256(&data, 2)?.to_string();
    let taker_premium_cap_bps = decode_data_u256(&data, 3)?.to_string();
    let expiry = decode_data_u256(&data, 4)?.to_string();
    let enabled = decode_bool(&data, 5)?;
    option_event_from_decoded_fields(
        log,
        chain_id,
        "OverrideSet",
        OVERRIDE_SET_SIGNATURE,
        None,
        None,
        Some(trader.clone()),
        None,
        None,
        None,
        serde_json::json!({
            "trader": trader,
            "makerNotionalFeeBps": maker_notional_fee_bps,
            "makerPremiumCapBps": maker_premium_cap_bps,
            "takerNotionalFeeBps": taker_notional_fee_bps,
            "takerPremiumCapBps": taker_premium_cap_bps,
            "expiry": expiry,
            "enabled": enabled
        }),
    )
}

/// Decode `FeeChargedV2(address indexed consumer, address indexed trader,
/// address indexed recipient, address settlementAsset, uint8 productKind,
/// uint8 flowKind, bool isMaker, int32 feePpm, uint256 basisAmount,
/// uint256 feeAmount)`.
fn decode_fee_charged_v2_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "FeeChargedV2 log must have four topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 7 {
        return Err(BackendError::Indexer(
            "FeeChargedV2 data must contain seven ABI words".to_string(),
        ));
    }
    let consumer = decode_topic_address(&log.topics[1])?;
    let trader = decode_topic_address(&log.topics[2])?;
    let recipient = decode_topic_address(&log.topics[3])?;
    let settlement_asset = decode_data_address(&data, 0)?;
    let product_kind_raw = decode_data_u256(&data, 1)?.to::<u64>();
    let flow_kind_raw = decode_data_u256(&data, 2)?.to::<u64>();
    let is_maker = decode_bool(&data, 3)?;
    let fee_ppm = decode_data_i32(&data, 4)?;
    let basis_amount = decode_data_u256(&data, 5)?.to_string();
    let fee_amount = decode_data_u256(&data, 6)?.to_string();
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "FeeChargedV2".to_string(),
        event_signature: FEE_CHARGED_V2_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer: None,
        seller: None,
        account: Some(trader.clone()),
        option_id: None,
        quantity_contracts: None,
        premium_per_contract_native: Some(fee_amount.clone()),
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "consumer": consumer,
            "trader": trader,
            "recipient": recipient,
            "settlementAsset": settlement_asset,
            "productKind": product_kind_label(product_kind_raw),
            "productKindRaw": product_kind_raw,
            "flowKind": flow_kind_label(flow_kind_raw),
            "flowKindRaw": flow_kind_raw,
            "isMaker": is_maker,
            "feePpm": fee_ppm,
            "basisAmount": basis_amount,
            "feeAmount": fee_amount,
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// Decode `FeeRebatedV2(address indexed consumer, address indexed trader,
/// address indexed recipient, address settlementAsset, uint8 productKind,
/// uint8 flowKind, int32 rebatePpm, uint256 basisAmount,
/// uint256 rebateAmount)`. Rebates are always credited to a maker on the
/// V2 path, so `isMaker` is implicit (`true`).
fn decode_fee_rebated_v2_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 4 {
        return Err(BackendError::Indexer(
            "FeeRebatedV2 log must have four topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 6 {
        return Err(BackendError::Indexer(
            "FeeRebatedV2 data must contain six ABI words".to_string(),
        ));
    }
    let consumer = decode_topic_address(&log.topics[1])?;
    let trader = decode_topic_address(&log.topics[2])?;
    let recipient = decode_topic_address(&log.topics[3])?;
    let settlement_asset = decode_data_address(&data, 0)?;
    let product_kind_raw = decode_data_u256(&data, 1)?.to::<u64>();
    let flow_kind_raw = decode_data_u256(&data, 2)?.to::<u64>();
    let rebate_ppm = decode_data_i32(&data, 3)?;
    let basis_amount = decode_data_u256(&data, 4)?.to_string();
    let rebate_amount = decode_data_u256(&data, 5)?.to_string();
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: "FeeRebatedV2".to_string(),
        event_signature: FEE_REBATED_V2_SIGNATURE.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer: None,
        seller: None,
        account: Some(trader.clone()),
        option_id: None,
        quantity_contracts: None,
        premium_per_contract_native: Some(rebate_amount.clone()),
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(serde_json::json!({
            "consumer": consumer,
            "trader": trader,
            "recipient": recipient,
            "settlementAsset": settlement_asset,
            "productKind": product_kind_label(product_kind_raw),
            "productKindRaw": product_kind_raw,
            "flowKind": flow_kind_label(flow_kind_raw),
            "flowKindRaw": flow_kind_raw,
            "isMaker": true,
            "rebatePpm": rebate_ppm,
            "basisAmount": basis_amount,
            "rebateAmount": rebate_amount,
        })),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn decode_rebate_budget_funded_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "RebateBudgetFunded log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "RebateBudgetFunded data must contain one ABI word".to_string(),
        ));
    }
    let settlement_asset = decode_topic_address(&log.topics[1])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "RebateBudgetFunded",
        REBATE_BUDGET_FUNDED_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        Some(amount.clone()),
        serde_json::json!({
            "settlementAsset": settlement_asset,
            "amount": amount,
        }),
    )
}

fn decode_rebate_budget_withdrawn_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 3 {
        return Err(BackendError::Indexer(
            "RebateBudgetWithdrawn log must have three topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "RebateBudgetWithdrawn data must contain one ABI word".to_string(),
        ));
    }
    let settlement_asset = decode_topic_address(&log.topics[1])?;
    let to = decode_topic_address(&log.topics[2])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "RebateBudgetWithdrawn",
        REBATE_BUDGET_WITHDRAWN_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        Some(amount.clone()),
        serde_json::json!({
            "settlementAsset": settlement_asset,
            "to": to,
            "amount": amount,
        }),
    )
}

fn decode_rebate_budget_spent_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "RebateBudgetSpent log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "RebateBudgetSpent data must contain one ABI word".to_string(),
        ));
    }
    let settlement_asset = decode_topic_address(&log.topics[1])?;
    let amount = decode_data_u256(&data, 0)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "RebateBudgetSpent",
        REBATE_BUDGET_SPENT_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        Some(amount.clone()),
        serde_json::json!({
            "settlementAsset": settlement_asset,
            "amount": amount,
        }),
    )
}

fn decode_fee_recipient_set_v2_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 3 {
        return Err(BackendError::Indexer(
            "FeeRecipientSet (V2) log must have three topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if !data.is_empty() {
        return Err(BackendError::Indexer(
            "FeeRecipientSet (V2) data must be empty".to_string(),
        ));
    }
    let old_recipient = decode_topic_address(&log.topics[1])?;
    let new_recipient = decode_topic_address(&log.topics[2])?;
    option_event_from_decoded_fields(
        log,
        chain_id,
        "FeeRecipientSetV2",
        FEE_RECIPIENT_SET_V2_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "oldRecipient": old_recipient,
            "newRecipient": new_recipient,
        }),
    )
}

fn decode_fee_consumer_set_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "FeeConsumerSet log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 {
        return Err(BackendError::Indexer(
            "FeeConsumerSet data must contain one ABI word".to_string(),
        ));
    }
    let consumer = decode_topic_address(&log.topics[1])?;
    let allowed = decode_bool(&data, 0)?;
    option_event_from_decoded_fields(
        log,
        chain_id,
        "FeeConsumerSetV2",
        FEE_CONSUMER_SET_SIGNATURE,
        None,
        None,
        Some(consumer.clone()),
        None,
        None,
        None,
        serde_json::json!({
            "consumer": consumer,
            "allowed": allowed,
        }),
    )
}

fn decode_merkle_root_set_v2_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "MerkleRootSet (V2) log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 2 {
        return Err(BackendError::Indexer(
            "MerkleRootSet (V2) data must contain two ABI words".to_string(),
        ));
    }
    let root = decode_topic_bytes32(&log.topics[1])?;
    let valid_from = decode_data_u256(&data, 0)?.to_string();
    let valid_until = decode_data_u256(&data, 1)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "MerkleRootSetV2",
        MERKLE_ROOT_SET_V2_SIGNATURE,
        None,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "root": root,
            "validFrom": valid_from,
            "validUntil": valid_until,
        }),
    )
}

fn decode_tier_claimed_v2_log(log: &EthLog, chain_id: u64) -> Result<OptionExecutionEvent> {
    if log.topics.len() != 2 {
        return Err(BackendError::Indexer(
            "TierClaimed (V2) log must have two topics".to_string(),
        ));
    }
    let data = decode_hex_bytes(&log.data)?;
    if data.len() != 32 * 2 {
        return Err(BackendError::Indexer(
            "TierClaimed (V2) data must contain two ABI words".to_string(),
        ));
    }
    let account = decode_topic_address(&log.topics[1])?;
    let tier = decode_data_u256(&data, 0)?.to::<u64>();
    let valid_until = decode_data_u256(&data, 1)?.to_string();
    option_event_from_decoded_fields(
        log,
        chain_id,
        "TierClaimedV2",
        TIER_CLAIMED_V2_SIGNATURE,
        None,
        None,
        Some(account.clone()),
        None,
        None,
        None,
        serde_json::json!({
            "account": account,
            "tier": tier,
            "validUntil": valid_until,
        }),
    )
}

fn product_kind_label(raw: u64) -> &'static str {
    match raw {
        0 => "option",
        1 => "perp",
        _ => "unknown",
    }
}

fn flow_kind_label(raw: u64) -> &'static str {
    match raw {
        0 => "orderbook",
        1 => "rfq",
        _ => "unknown",
    }
}

#[allow(clippy::too_many_arguments)]
fn option_event_from_decoded_fields(
    log: &EthLog,
    chain_id: u64,
    event_name: &str,
    event_signature: &str,
    buyer: Option<String>,
    seller: Option<String>,
    account: Option<String>,
    option_id: Option<String>,
    quantity_contracts: Option<String>,
    premium_per_contract_native: Option<String>,
    decoded: serde_json::Value,
) -> Result<OptionExecutionEvent> {
    let tx_hash =
        required_field(log.transaction_hash.as_ref(), "transactionHash")?.to_ascii_lowercase();
    let log_index = parse_hex_quantity(required_field(log.log_index.as_ref(), "logIndex")?)?;
    let block_number =
        parse_hex_quantity(required_field(log.block_number.as_ref(), "blockNumber")?)?;
    let now = now_ms();

    Ok(OptionExecutionEvent {
        id: Uuid::new_v4(),
        chain_id,
        contract_address: log.address.to_ascii_lowercase(),
        tx_hash,
        log_index,
        block_number,
        block_hash: log.block_hash.clone(),
        event_name: event_name.to_string(),
        event_signature: event_signature.to_string(),
        intent_id: None,
        onchain_intent_id: None,
        option_execution_transaction_id: None,
        buyer,
        seller,
        account,
        option_id,
        quantity_contracts,
        premium_per_contract_native,
        raw_topics: raw_topics_json(log),
        raw_data: log.data.clone(),
        decoded: Some(decoded),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn required_field<'a>(value: Option<&'a String>, field: &str) -> Result<&'a String> {
    value.ok_or_else(|| BackendError::Indexer(format!("log missing {field}")))
}

fn decode_topic_address(topic: &str) -> Result<String> {
    let bytes = decode_fixed_hex(topic, 32)?;
    Ok(format!("0x{}", hex_lower(&bytes[12..])))
}

fn decode_topic_bytes32(topic: &str) -> Result<String> {
    Ok(hex_0x(&decode_fixed_hex(topic, 32)?))
}

fn decode_topic_u256(topic: &str) -> Result<U256> {
    Ok(U256::from_be_slice(&decode_fixed_hex(topic, 32)?))
}

fn raw_topics_json(log: &EthLog) -> serde_json::Value {
    serde_json::Value::Array(
        log.topics
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
}

fn decode_data_u256(data: &[u8], word_index: usize) -> Result<U256> {
    let start = word_index * 32;
    let end = start + 32;
    let word = data.get(start..end).ok_or_else(|| {
        BackendError::Indexer(format!("missing ABI data word at index {word_index}"))
    })?;
    Ok(U256::from_be_slice(word))
}

fn decode_data_address(data: &[u8], word_index: usize) -> Result<String> {
    let start = word_index * 32;
    let end = start + 32;
    let word = data.get(start..end).ok_or_else(|| {
        BackendError::Indexer(format!(
            "missing ABI data word at index {word_index} (address)"
        ))
    })?;
    Ok(format!("0x{}", hex_lower(&word[12..])))
}

/// Decode a signed `int32` ABI-encoded as a sign-extended 32-byte word.
fn decode_data_i32(data: &[u8], word_index: usize) -> Result<i32> {
    let start = word_index * 32;
    let end = start + 32;
    let word = data.get(start..end).ok_or_else(|| {
        BackendError::Indexer(format!(
            "missing ABI data word at index {word_index} (int32)"
        ))
    })?;
    let is_negative = word[0] & 0x80 != 0;
    let expected_pad = if is_negative { 0xffu8 } else { 0x00u8 };
    for byte in &word[..28] {
        if *byte != expected_pad {
            return Err(BackendError::Indexer(
                "int32 ABI word is not sign-extended to 32 bytes".to_string(),
            ));
        }
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&word[28..32]);
    Ok(i32::from_be_bytes(bytes))
}

fn decode_bool(data: &[u8], word_index: usize) -> Result<bool> {
    let value = decode_data_u256(data, word_index)?;
    if value == U256::ZERO {
        Ok(false)
    } else if value == U256::from(1u8) {
        Ok(true)
    } else {
        Err(BackendError::Indexer("invalid ABI bool".to_string()))
    }
}

fn decode_fixed_hex(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let bytes = decode_hex_bytes(value)?;
    if bytes.len() != expected_len {
        return Err(BackendError::Indexer(format!(
            "expected {expected_len} hex bytes"
        )));
    }
    Ok(bytes)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub fn spawn_option_event_indexer(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !state.option_event_indexer_config.enabled {
            tracing::info!("option event indexer disabled");
            return;
        }
        let poll_interval_ms = state.option_event_indexer_config.poll_interval_ms;
        let Some(rpc_url) = state.option_event_indexer_config.rpc_url.clone() else {
            tracing::warn!("option event indexer enabled without RPC_URL; refusing to spawn");
            return;
        };
        let provider = HttpJsonRpcProvider::new(rpc_url);
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(poll_interval_ms));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match index_option_events_with_provider(&state, &provider).await {
                Ok(result) => {
                    if result.cursor_updated || result.logs_found > 0 {
                        tracing::info!(
                            from_block = result.from_block,
                            to_block = result.to_block,
                            logs_found = result.logs_found,
                            events_indexed = result.events_indexed,
                            "option event indexer tick"
                        );
                    }
                }
                Err(error) => tracing::warn!(%error, "option event indexer tick failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;
    use crate::execution::{ExecutionTransactionStatus, RpcFuture};
    use crate::options::{
        OptionExecutionIntent, OptionExecutionIntentStatus, OptionExecutionSimulationStatus,
        OptionExecutionSourceType, OptionExecutionTransaction, OptionsConfig,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockEventLogProvider {
        head: u64,
        logs: Arc<Mutex<Vec<EthLog>>>,
        block_calls: Arc<Mutex<u64>>,
        filters: Arc<Mutex<Vec<EthGetLogsFilter>>>,
    }

    impl MockEventLogProvider {
        fn new(head: u64, logs: Vec<EthLog>) -> Self {
            Self {
                head,
                logs: Arc::new(Mutex::new(logs)),
                block_calls: Arc::new(Mutex::new(0)),
                filters: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn block_call_count(&self) -> u64 {
            *self.block_calls.lock().unwrap()
        }

        fn filters(&self) -> Vec<EthGetLogsFilter> {
            self.filters.lock().unwrap().clone()
        }
    }

    impl EthLogsProvider for MockEventLogProvider {
        fn block_number(&self) -> RpcFuture<'_, u64> {
            let calls = self.block_calls.clone();
            let head = self.head;
            Box::pin(async move {
                *calls.lock().unwrap() += 1;
                Ok(head)
            })
        }

        fn get_logs(&self, filter: EthGetLogsFilter) -> RpcFuture<'_, Vec<EthLog>> {
            let filters = self.filters.clone();
            let logs = self.logs.clone();
            Box::pin(async move {
                let topic0 = filter.topics.first().cloned();
                let address = filter.address.to_ascii_lowercase();
                filters.lock().unwrap().push(filter);
                Ok(logs
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|log| log.address.eq_ignore_ascii_case(&address))
                    .filter(|log| {
                        matches!(
                            (log.topics.first(), topic0.as_ref()),
                            (Some(left), Some(right)) if left.eq_ignore_ascii_case(right)
                        )
                    })
                    .cloned()
                    .collect())
            })
        }
    }

    #[tokio::test]
    async fn disabled_indexer_does_nothing() {
        let state = state_with_event_indexer(false, 0, 1_000, 3);
        let provider = MockEventLogProvider::new(100, vec![option_trade_log(10, 2)]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert!(!result.enabled);
        assert_eq!(provider.block_call_count(), 0);
        assert!(provider.filters().is_empty());
        assert!(state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10)
            .is_empty());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn cursor_initializes_from_config_when_no_safe_range_exists() {
        let state = state_with_event_indexer(true, 100, 1_000, 3);
        let provider = MockEventLogProvider::new(102, Vec::new());

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.last_indexed_block, 100);
        assert_eq!(result.from_block, 101);
        assert_eq!(result.to_block, 100);
        assert!(!result.cursor_updated);
        assert!(state
            .options_store
            .lock()
            .unwrap()
            .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID)
            .is_none());
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn finality_safe_head_is_respected() {
        let state = state_with_event_indexer(true, 0, 1_000, 3);
        let provider = MockEventLogProvider::new(2, Vec::new());

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.safe_head, Some(0));
        assert_eq!(result.from_block, 1);
        assert_eq!(result.to_block, 0);
        assert!(provider.filters().is_empty());
        assert!(!result.cursor_updated);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn batch_size_is_respected() {
        let state = state_with_event_indexer(true, 0, 10, 0);
        let provider = MockEventLogProvider::new(100, Vec::new());

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.from_block, 1);
        assert_eq!(result.to_block, 10);
        let filters = provider.filters();
        assert_eq!(filters.len(), 9);
        assert_eq!(filters[0].from_block, "0x1");
        assert_eq!(filters[0].to_block, "0xa");
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn no_logs_advances_cursor() {
        let state = state_with_event_indexer(true, 0, 10, 0);
        let provider = MockEventLogProvider::new(100, Vec::new());

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert!(result.cursor_updated);
        assert_eq!(result.events_indexed, 0);
        let cursor = state
            .options_store
            .lock()
            .unwrap()
            .get_option_event_indexer_state(OPTION_EVENT_INDEXER_STATE_ID)
            .unwrap();
        assert_eq!(cursor.last_indexed_block, 10);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn option_trade_executed_decodes_and_persists() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(20, vec![option_trade_log(10, 2)]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.logs_found, 1);
        assert_eq!(result.events_decoded, 1);
        assert_eq!(result.events_indexed, 1);
        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].chain_id, 84532);
        assert_eq!(events[0].event_name, "OptionTradeExecuted");
        assert_eq!(
            events[0].onchain_intent_id.as_deref(),
            Some("0x1111111111111111111111111111111111111111111111111111111111111111")
        );
        assert_eq!(
            events[0].buyer.as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );
        assert_eq!(events[0].option_id.as_deref(), Some("7"));
        assert_eq!(events[0].quantity_contracts.as_deref(), Some("1"));
        assert_eq!(
            events[0].premium_per_contract_native.as_deref(),
            Some("10000")
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn multiple_emitter_contracts_are_supported() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(
            20,
            vec![
                option_trade_log(10, 2),
                trading_fee_log(10, 3),
                internal_transfer_log(10, 4),
            ],
        );

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.logs_found, 3);
        assert_eq!(result.events_decoded, 3);
        assert_eq!(result.events_indexed, 3);
        let filters = provider.filters();
        assert!(filters
            .iter()
            .any(|filter| filter.address == "0x00000000000000000000000000000000000000ee"));
        assert!(filters
            .iter()
            .any(|filter| filter.address == "0x00000000000000000000000000000000000000aa"));
        assert!(filters
            .iter()
            .any(|filter| filter.address == "0x00000000000000000000000000000000000000bb"));
        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .any(|event| event.event_name == "OptionTradeExecuted"));
        assert!(events
            .iter()
            .any(|event| event.event_name == "TradingFeeCharged"));
        assert!(events
            .iter()
            .any(|event| event.event_name == "InternalTransfer"));
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn trading_fee_charged_from_margin_engine_decodes_and_persists() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(20, vec![trading_fee_log(10, 3)]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.events_indexed, 1);
        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "TradingFeeCharged");
        assert_eq!(
            events[0].contract_address,
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(
            events[0].account.as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );
        assert_eq!(events[0].option_id.as_deref(), Some("7"));
        assert_eq!(
            events[0].premium_per_contract_native.as_deref(),
            Some("10000")
        );
        assert_eq!(events[0].decoded.as_ref().unwrap()["appliedFee"], "6");
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn internal_transfer_from_collateral_vault_decodes_and_persists() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(20, vec![internal_transfer_log(10, 4)]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.events_indexed, 1);
        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "InternalTransfer");
        assert_eq!(
            events[0].contract_address,
            "0x00000000000000000000000000000000000000bb"
        );
        assert_eq!(
            events[0].account.as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );
        assert_eq!(
            events[0].premium_per_contract_native.as_deref(),
            Some("10000")
        );
        assert_eq!(
            events[0].decoded.as_ref().unwrap()["to"],
            "0x0000000000000000000000000000000000000002"
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn duplicate_log_is_idempotent() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let log = option_trade_log(10, 2);
        let provider = MockEventLogProvider::new(20, vec![log.clone(), log]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.logs_found, 2);
        assert_eq!(result.events_decoded, 2);
        assert_eq!(result.events_indexed, 1);
        assert_eq!(
            state
                .options_store
                .lock()
                .unwrap()
                .list_option_execution_events(10)
                .len(),
            1
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn same_tx_multi_contract_events_link_to_same_option_transaction() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let tx_hash = tx_hash();
        let (intent, transaction) = insert_broadcast_submitted_transaction(&state, tx_hash);
        let provider = MockEventLogProvider::new(
            20,
            vec![
                option_trade_log(10, 2),
                trading_fee_log(10, 3),
                internal_transfer_log(10, 4),
            ],
        );

        index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 3);
        for event in events {
            assert_eq!(event.intent_id, Some(intent.intent_id));
            assert_eq!(
                event.option_execution_transaction_id.as_deref(),
                Some(transaction.transaction_id.as_str())
            );
        }
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn event_links_to_transaction_by_tx_hash() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let tx_hash = tx_hash();
        let (intent, transaction) = insert_broadcast_submitted_transaction(&state, tx_hash);
        let provider = MockEventLogProvider::new(20, vec![option_trade_log(10, 2)]);

        index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].intent_id, Some(intent.intent_id));
        assert_eq!(
            events[0].option_execution_transaction_id.as_deref(),
            Some(transaction.transaction_id.as_str())
        );
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn latest_tick_is_published_in_memory() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(20, Vec::new());

        index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        let latest = state.option_event_indexer_last_tick.lock().unwrap().clone();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().to_block, 20);
        assert_no_generic_execution_rows(&state);
    }

    #[tokio::test]
    async fn no_broadcast_path_is_touched() {
        let state = state_with_event_indexer(true, 0, 20, 0);
        let provider = MockEventLogProvider::new(20, vec![option_trade_log(10, 2)]);
        let mock_broadcast_send_count = Arc::new(Mutex::new(0u64));

        index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(*mock_broadcast_send_count.lock().unwrap(), 0);
        assert_eq!(provider.block_call_count(), 1);
        assert_eq!(provider.filters().len(), 9);
        assert_no_generic_execution_rows(&state);
    }

    #[test]
    fn option_trade_topic_matches_solidity_signature() {
        assert_eq!(
            option_trade_executed_topic0(),
            "0xb2387b9f0e4823ecef9a16ea4aaba6598c0703fb5e9d8dba37ef303add4cb808"
        );
        assert_eq!(
            margin_trade_executed_topic0(),
            "0x6f0909c4bf7f20fe8de71b889c29e66311610d5f753a42ae63495e08bbb65f7e"
        );
        assert_eq!(
            trading_fee_charged_topic0(),
            "0x12cf63383901008103b6e03c39d208d7757a2f9842d9d4e18e58bc13f75f7f7b"
        );
        assert_eq!(
            internal_transfer_topic0(),
            "0x77178bcf8f3c991d39734824771477a42787fe19b60d5a29c0ec72de167699b3"
        );
    }

    #[test]
    fn disabled_indexer_allows_missing_emitters() {
        let config = OptionEventIndexerConfig::disabled();

        config.validate_startup(false).unwrap();
    }

    #[test]
    fn enabled_indexer_validates_required_emitters() {
        let mut config = OptionEventIndexerConfig::disabled();
        config.enabled = true;
        config.require_rpc = false;
        config.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        config.collateral_vault_address =
            AccountId::new("0x00000000000000000000000000000000000000bb");

        let error = config.validate_startup(true).unwrap_err();

        assert!(error
            .to_string()
            .contains("OPTION_EVENT_INDEXER_MARGIN_ENGINE_ADDRESS is required"));
    }

    #[test]
    fn fee_charged_v2_log_decodes_topics_and_signed_ppm() {
        let log = fee_charged_v2_log(15, 6);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "FeeChargedV2");
        assert_eq!(event.event_signature, FEE_CHARGED_V2_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["consumer"],
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(
            decoded["trader"],
            "0x0000000000000000000000000000000000000001"
        );
        assert_eq!(
            decoded["recipient"],
            "0x00000000000000000000000000000000000000f0"
        );
        assert_eq!(
            decoded["settlementAsset"],
            "0x0000000000000000000000000000000000000020"
        );
        assert_eq!(decoded["productKind"], "option");
        assert_eq!(decoded["flowKind"], "orderbook");
        assert_eq!(decoded["isMaker"], false);
        assert_eq!(decoded["feePpm"], 250);
        assert_eq!(decoded["basisAmount"], "10000");
        assert_eq!(decoded["feeAmount"], "25");
        assert_eq!(
            event.account.as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );
        assert_eq!(event.premium_per_contract_native.as_deref(), Some("25"));
    }

    /// V2F-N: a PERP FeeChargedV2 log emitted by FeesManagerV2 when
    /// PerpEngineV2 calls `chargeFee` decodes with `productKind = "perp"`
    /// and `flowKind = "orderbook"`. Reproduces the V2F-LM live taker leg
    /// (`feePpm = 300`, `basisAmount = 30`, `feeAmount = 1`).
    #[test]
    fn fee_charged_v2_perp_log_decodes_with_perp_product_kind() {
        let log = fee_charged_v2_perp_log(42_188_599, 0);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "FeeChargedV2");
        let decoded = event.decoded.expect("decoded");
        assert_eq!(decoded["productKind"], "perp");
        assert_eq!(decoded["productKindRaw"], 1);
        assert_eq!(decoded["flowKind"], "orderbook");
        assert_eq!(decoded["flowKindRaw"], 0);
        assert_eq!(decoded["isMaker"], false);
        assert_eq!(decoded["feePpm"], 300);
        assert_eq!(decoded["basisAmount"], "30");
        assert_eq!(decoded["feeAmount"], "1");
    }

    #[test]
    fn fee_rebated_v2_log_decodes_negative_ppm_and_amount() {
        let log = fee_rebated_v2_log(15, 7);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "FeeRebatedV2");
        assert_eq!(event.event_signature, FEE_REBATED_V2_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["trader"],
            "0x0000000000000000000000000000000000000002"
        );
        assert_eq!(
            decoded["recipient"],
            "0x0000000000000000000000000000000000000002"
        );
        assert_eq!(decoded["productKind"], "option");
        assert_eq!(decoded["flowKind"], "orderbook");
        assert_eq!(decoded["rebatePpm"], -50);
        assert_eq!(decoded["isMaker"], true);
        assert_eq!(decoded["basisAmount"], "10000");
        assert_eq!(decoded["rebateAmount"], "5");
    }

    #[test]
    fn rebate_budget_funded_decodes_amount() {
        let log = rebate_budget_funded_log(15, 8);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "RebateBudgetFunded");
        assert_eq!(event.event_signature, REBATE_BUDGET_FUNDED_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["settlementAsset"],
            "0x0000000000000000000000000000000000000020"
        );
        assert_eq!(decoded["amount"], "100000");
    }

    #[test]
    fn rebate_budget_withdrawn_decodes_two_addresses() {
        let log = rebate_budget_withdrawn_log(15, 9);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "RebateBudgetWithdrawn");
        assert_eq!(event.event_signature, REBATE_BUDGET_WITHDRAWN_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["settlementAsset"],
            "0x0000000000000000000000000000000000000020"
        );
        assert_eq!(decoded["to"], "0x00000000000000000000000000000000000000c0");
        assert_eq!(decoded["amount"], "500");
    }

    #[test]
    fn rebate_budget_spent_decodes_amount() {
        let log = rebate_budget_spent_log(15, 10);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "RebateBudgetSpent");
        assert_eq!(event.event_signature, REBATE_BUDGET_SPENT_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(decoded["amount"], "5");
    }

    #[test]
    fn fee_recipient_set_v2_decodes_topics() {
        let log = fee_recipient_set_v2_log(15, 11);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "FeeRecipientSetV2");
        assert_eq!(event.event_signature, FEE_RECIPIENT_SET_V2_SIGNATURE);
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["oldRecipient"],
            "0x0000000000000000000000000000000000000000"
        );
        assert_eq!(
            decoded["newRecipient"],
            "0x00000000000000000000000000000000000000f0"
        );
    }

    #[test]
    fn fee_consumer_set_decodes_bool() {
        let log = fee_consumer_set_log(15, 12);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "FeeConsumerSetV2");
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["consumer"],
            "0x00000000000000000000000000000000000000aa"
        );
        assert_eq!(decoded["allowed"], true);
    }

    #[test]
    fn merkle_root_set_v2_decodes_window() {
        let log = merkle_root_set_v2_log(15, 13);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "MerkleRootSetV2");
        let decoded = event.decoded.expect("decoded");
        assert_eq!(decoded["validFrom"], "1700000000");
        assert_eq!(decoded["validUntil"], "1800000000");
    }

    #[test]
    fn tier_claimed_v2_decodes_account_and_tier() {
        let log = tier_claimed_v2_log(15, 14);
        let event = decode_option_execution_event(&log, 84532).unwrap().unwrap();
        assert_eq!(event.event_name, "TierClaimedV2");
        let decoded = event.decoded.expect("decoded");
        assert_eq!(
            decoded["account"],
            "0x0000000000000000000000000000000000000001"
        );
        assert_eq!(decoded["tier"], 3);
        assert_eq!(decoded["validUntil"], "1800000000");
    }

    #[tokio::test]
    async fn fees_manager_v2_emitter_role_subscribes_to_v2_topics_and_decodes() {
        let mut state = state_with_event_indexer(true, 0, 20, 0);
        state.option_event_indexer_config.fees_manager_v2_address =
            Some(AccountId::new("0x00000000000000000000000000000000000000dd"));
        let log = fee_charged_v2_log_on("0x00000000000000000000000000000000000000dd", 15, 6);
        let provider = MockEventLogProvider::new(20, vec![log]);

        let result = index_option_events_with_provider(&state, &provider)
            .await
            .unwrap();

        assert_eq!(result.events_indexed, 1);
        let events = state
            .options_store
            .lock()
            .unwrap()
            .list_option_execution_events(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "FeeChargedV2");
        assert_eq!(
            events[0].contract_address,
            "0x00000000000000000000000000000000000000dd"
        );
        // Ensure the FeesManagerV2 emitter requested all nine V2 topics.
        let filters = provider.filters();
        let v2_topic_count = filters
            .iter()
            .filter(|filter| filter.address == "0x00000000000000000000000000000000000000dd")
            .count();
        assert_eq!(v2_topic_count, 9);
        assert_no_generic_execution_rows(&state);
    }

    #[test]
    fn v1_default_indexer_does_not_subscribe_to_v2_topics() {
        let config = OptionEventIndexerConfig {
            enabled: true,
            poll_interval_ms: 15_000,
            from_block: 0,
            batch_blocks: 10,
            confirmation_blocks: 0,
            require_rpc: false,
            rpc_url: None,
            matching_engine_address: AccountId::new("0x00000000000000000000000000000000000000ee"),
            margin_engine_address: AccountId::new("0x00000000000000000000000000000000000000aa"),
            collateral_vault_address: AccountId::new("0x00000000000000000000000000000000000000bb"),
            fees_manager_address: Some(AccountId::new(
                "0x00000000000000000000000000000000000000cc",
            )),
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
        };
        let roles: Vec<String> = config
            .emitter_contracts()
            .into_iter()
            .map(|contract| contract.role)
            .collect();
        assert!(roles.contains(&"fees_manager".to_string()));
        assert!(!roles.contains(&"fees_manager_v2".to_string()));
    }

    fn fee_charged_v2_log(block_number: u64, log_index: u64) -> EthLog {
        fee_charged_v2_log_on(
            "0x00000000000000000000000000000000000000dd",
            block_number,
            log_index,
        )
    }

    fn fee_charged_v2_log_on(address: &str, block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: address.to_string(),
            topics: vec![
                fee_charged_v2_topic0(),
                topic_address("00000000000000000000000000000000000000aa"),
                topic_address("0000000000000000000000000000000000000001"),
                topic_address("00000000000000000000000000000000000000f0"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}{}",
                topic_address_no_prefix("0000000000000000000000000000000000000020"),
                word_no_prefix(0), // productKind: OPTION
                word_no_prefix(0), // flowKind: ORDERBOOK
                word_no_prefix(0), // isMaker: false
                signed_word(250),  // feePpm
                word_no_prefix(10_000),
                word_no_prefix(25),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    /// V2F-N: a PERP-flavoured FeeChargedV2 log mirroring the V2F-LM smoke
    /// transaction's taker leg on Base Sepolia.
    fn fee_charged_v2_perp_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00da0b9876bcbf0c79cb5bcacfebafb8c7ad774f".to_string(),
            topics: vec![
                fee_charged_v2_topic0(),
                topic_address("c6c592100723fe0c66343a16e95ec34cc0c2141c"),
                topic_address("8b94a83d1ad3bd2337b1886e7962ca8e0bba9a34"),
                topic_address("009f38440f058d095b61e0e2ee7fabdf05be7500"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}{}",
                topic_address_no_prefix("6eae407f5640b006fac9965182e238582a3b412e"),
                word_no_prefix(1), // productKind: PERP
                word_no_prefix(0), // flowKind: ORDERBOOK
                word_no_prefix(0), // isMaker: false
                signed_word(300),  // feePpm
                word_no_prefix(30),
                word_no_prefix(1),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x3333333333333333333333333333333333333333333333333333333333333333".to_string(),
            ),
            transaction_hash: Some(
                "0x400acedf36381034ae37c983cc50e80d11a81587ca8065fbaef40293ff63a79a".to_string(),
            ),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn fee_rebated_v2_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                fee_rebated_v2_topic0(),
                topic_address("00000000000000000000000000000000000000aa"),
                topic_address("0000000000000000000000000000000000000002"),
                topic_address("0000000000000000000000000000000000000002"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}",
                topic_address_no_prefix("0000000000000000000000000000000000000020"),
                word_no_prefix(0), // productKind: OPTION
                word_no_prefix(0), // flowKind: ORDERBOOK
                signed_word(-50),  // rebatePpm
                word_no_prefix(10_000),
                word_no_prefix(5),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn rebate_budget_funded_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                rebate_budget_funded_topic0(),
                topic_address("0000000000000000000000000000000000000020"),
            ],
            data: format!("0x{}", word_no_prefix(100_000)),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn rebate_budget_withdrawn_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                rebate_budget_withdrawn_topic0(),
                topic_address("0000000000000000000000000000000000000020"),
                topic_address("00000000000000000000000000000000000000c0"),
            ],
            data: format!("0x{}", word_no_prefix(500)),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn rebate_budget_spent_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                rebate_budget_spent_topic0(),
                topic_address("0000000000000000000000000000000000000020"),
            ],
            data: format!("0x{}", word_no_prefix(5)),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn fee_recipient_set_v2_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                fee_recipient_set_v2_topic0(),
                topic_address("0000000000000000000000000000000000000000"),
                topic_address("00000000000000000000000000000000000000f0"),
            ],
            data: "0x".to_string(),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn fee_consumer_set_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                fee_consumer_set_topic0(),
                topic_address("00000000000000000000000000000000000000aa"),
            ],
            data: format!("0x{}", word_no_prefix(1)),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn merkle_root_set_v2_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                merkle_root_set_v2_topic0(),
                "0x3333333333333333333333333333333333333333333333333333333333333333".to_string(),
            ],
            data: format!(
                "0x{}{}",
                word_no_prefix(1_700_000_000),
                word_no_prefix(1_800_000_000),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn tier_claimed_v2_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000dd".to_string(),
            topics: vec![
                tier_claimed_v2_topic0(),
                topic_address("0000000000000000000000000000000000000001"),
            ],
            data: format!("0x{}{}", word_no_prefix(3), word_no_prefix(1_800_000_000),),
            block_number: Some(hex_quantity(block_number)),
            block_hash: None,
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn topic_address_no_prefix(address_without_prefix: &str) -> String {
        format!("{:0>64}", address_without_prefix)
    }

    /// Encode a signed `int32` as a sign-extended 32-byte ABI word.
    fn signed_word(value: i32) -> String {
        let extended_byte = if value < 0 { 0xffu8 } else { 0x00u8 };
        let mut bytes = [extended_byte; 32];
        bytes[28..].copy_from_slice(&value.to_be_bytes());
        let mut out = String::with_capacity(64);
        for byte in bytes {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    fn state_with_event_indexer(
        enabled: bool,
        from_block: u64,
        batch_blocks: u64,
        confirmation_blocks: u64,
    ) -> AppState {
        let mut options = OptionsConfig::enabled_in_memory_for_tests();
        options.matching_engine_address =
            AccountId::new("0x00000000000000000000000000000000000000ee");
        let mut state = AppState::with_options_config(EngineState::with_default_markets(), options);
        state.option_event_indexer_config = OptionEventIndexerConfig {
            enabled,
            poll_interval_ms: 15_000,
            from_block,
            batch_blocks,
            confirmation_blocks,
            require_rpc: true,
            rpc_url: Some("http://127.0.0.1:8545".to_string()),
            matching_engine_address: AccountId::new("0x00000000000000000000000000000000000000ee"),
            margin_engine_address: AccountId::new("0x00000000000000000000000000000000000000aa"),
            collateral_vault_address: AccountId::new("0x00000000000000000000000000000000000000bb"),
            fees_manager_address: None,
            fees_manager_v2_address: None,
            old_margin_engine_address: None,
        };
        state
    }

    fn insert_broadcast_submitted_transaction(
        state: &AppState,
        tx_hash: &str,
    ) -> (OptionExecutionIntent, OptionExecutionTransaction) {
        let intent = option_intent();
        let transaction = OptionExecutionTransaction {
            transaction_id: "option-tx-1".to_string(),
            intent_id: intent.intent_id,
            onchain_intent_id: Some(intent.onchain_intent_id.clone()),
            from: AccountId::new("0x00000000000000000000000000000000000000c0"),
            to: AccountId::new("0x00000000000000000000000000000000000000ee"),
            calldata: "0x1234".to_string(),
            value_wei: "0".to_string(),
            gas_limit: Some(1_500_000),
            tx_hash: Some(tx_hash.to_string()),
            status: ExecutionTransactionStatus::Submitted,
            error: None,
            estimated_gas: None,
            required_gas: None,
            simulation_gas_limit: None,
            broadcast_gas_limit: None,
            gas_safety_bps: None,
            gas_check_status: None,
            gas_check_error: None,
            confirmation_status: None,
            confirmed_at_ms: None,
            confirmed_block_number: None,
            receipt_status: None,
            confirmation_error: None,
            gas_used: None,
            effective_gas_price: None,
            cumulative_gas_used: None,
            receipt_block_hash: None,
            receipt_transaction_index: None,
            receipt_observed_at_ms: None,
            created_at_ms: 2,
            updated_at_ms: 2,
        };
        {
            let mut store = state.options_store.lock().unwrap();
            let inserted = store.insert_option_execution_intent(intent);
            let inserted_tx = store
                .insert_option_execution_transaction(transaction)
                .unwrap();
            (inserted, inserted_tx)
        }
    }

    fn option_intent() -> OptionExecutionIntent {
        OptionExecutionIntent {
            intent_id: Uuid::from_u128(1),
            onchain_intent_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
            source_type: OptionExecutionSourceType::OptionOrderbookFill,
            source_id: "fill-1".to_string(),
            option_series_id: "series-1".to_string(),
            onchain_option_id: "7".to_string(),
            buyer: AccountId::new("0x0000000000000000000000000000000000000001"),
            seller: AccountId::new("0x0000000000000000000000000000000000000002"),
            underlying: AccountId::new("0x0000000000000000000000000000000000000010"),
            settlement_asset: AccountId::new("0x0000000000000000000000000000000000000020"),
            expiry: 4_102_444_800,
            strike_1e8: 300_000_000_000,
            is_call: true,
            contract_size_1e8: 100_000_000,
            quantity_contracts: 1,
            source_size_1e8: 100_000_000,
            source_price_1e8: 10_000_000,
            premium_per_contract_native: 10_000,
            buyer_is_maker: false,
            buyer_nonce: Some(0),
            seller_nonce: Some(0),
            deadline: 0,
            buyer_signature: Some("0x01".to_string()),
            seller_signature: Some("0x02".to_string()),
            calldata: Some("0x12345678".to_string()),
            status: OptionExecutionIntentStatus::BroadcastSubmitted,
            error: None,
            simulation_status: Some(OptionExecutionSimulationStatus::SimulationOk),
            simulation_error: None,
            simulation_block_number: Some(10),
            simulation_revert_data: None,
            simulation_revert_selector: None,
            simulated_at_ms: Some(1),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn option_trade_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000ee".to_string(),
            topics: vec![
                option_trade_executed_topic0(),
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
                topic_address("0000000000000000000000000000000000000001"),
                topic_address("0000000000000000000000000000000000000002"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}",
                word_no_prefix(7),
                word_no_prefix(1),
                word_no_prefix(10_000),
                word_no_prefix(0),
                word_no_prefix(0),
                word_no_prefix(0),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn trading_fee_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000aa".to_string(),
            topics: vec![
                trading_fee_charged_topic0(),
                topic_address("0000000000000000000000000000000000000001"),
                topic_address("00000000000000000000000000000000000000f0"),
                topic_address("0000000000000000000000000000000000000020"),
            ],
            data: format!(
                "0x{}{}{}{}{}{}{}{}",
                word_no_prefix(7),
                word_no_prefix(0),
                word_no_prefix(10_000),
                word_no_prefix(3_000_000),
                word_no_prefix(6),
                word_no_prefix(1_000),
                word_no_prefix(6),
                word_no_prefix(0),
            ),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn internal_transfer_log(block_number: u64, log_index: u64) -> EthLog {
        EthLog {
            address: "0x00000000000000000000000000000000000000bb".to_string(),
            topics: vec![
                internal_transfer_topic0(),
                topic_address("0000000000000000000000000000000000000020"),
                topic_address("0000000000000000000000000000000000000001"),
                topic_address("0000000000000000000000000000000000000002"),
            ],
            data: format!("0x{}", word_no_prefix(10_000)),
            block_number: Some(hex_quantity(block_number)),
            block_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            transaction_hash: Some(tx_hash().to_string()),
            log_index: Some(hex_quantity(log_index)),
        }
    }

    fn topic_address(address_without_prefix: &str) -> String {
        format!("0x{:0>64}", address_without_prefix)
    }

    fn word_no_prefix(value: u128) -> String {
        format!("{value:064x}")
    }

    fn tx_hash() -> &'static str {
        "0x5964a7b3d2c18d051baaa780413d31c44d419ce530f45263cb4c46f720881125"
    }

    fn assert_no_generic_execution_rows(state: &AppState) {
        assert!(state.repository.is_none());
        assert!(state.trade_signatures.lock().unwrap().is_empty());
    }
}
