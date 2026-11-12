//! Authenticated, read-only projection of covered-call obligations.
//! Contract lots are fungible per (buyer, seller); fills do not carry lot IDs.
use super::http::AppState;
use super::trading::{
    authorize_account_read, Envelope, MetaBlock, TradingApiError, TradingErrorCode, Warning,
};
use crate::execution::rpc::{
    EthCallProvider, EthCallRequest, EthLogsProvider, HttpJsonRpcProvider,
    TransactionBroadcastProvider,
};
use crate::options::OptionExecutionEvent;
use crate::types::AccountId;
use alloy_primitives::{Address, U256};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Serialize;
use sha3::{Digest, Keccak256};
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Serialize)]
pub struct PhysicalEventProof {
    event: String,
    quantity: String,
    tx_hash: String,
    block_hash: String,
    log_index: u64,
}

#[derive(Serialize)]
pub struct PhysicalPosition {
    buyer: String,
    seller: String,
    filled_contracts: String,
    open_contracts: String,
    exercised_contracts: String,
    lapsed_contracts: String,
    waived_contracts: String,
    premium_native: String,
    strike_paid_native: String,
    underlying_locked_native: String,
    underlying_delivered_native: String,
    underlying_returned_native: String,
    fee_native: Option<String>,
    buyer_fee_native: Option<String>,
    seller_fee_native: Option<String>,
    proof: Vec<PhysicalEventProof>,
}

#[derive(Serialize)]
pub struct PhysicalPositionsData {
    address: String,
    chain_id: u64,
    registry: String,
    engine: String,
    series_id: String,
    option_id: String,
    underlying: String,
    quote: String,
    underlying_per_contract_native: String,
    strike_per_contract_native: String,
    expiry: u64,
    exercise_deadline: u64,
    chain_timestamp: u64,
    positions: Vec<PhysicalPosition>,
}

/// Public read-only view of the isolated stack before its first series exists.
/// A database row alone is never treated as evidence that trading is live.
#[derive(Serialize)]
pub struct PhysicalStackData {
    chain_id: u64,
    registry: String,
    engine: String,
    matcher: String,
    governance_owner: String,
    series_count_onchain: String,
    series_count_indexed: usize,
    registry_paused: bool,
    engine_paused: bool,
    matcher_paused: bool,
    max_open_contracts: String,
    matching_engine_bound: bool,
    economic_state: &'static str,
}

#[derive(Default)]
struct Totals {
    filled: u128,
    open: u128,
    exercised: u128,
    lapsed: u128,
    waived: u128,
    premium: u128,
    strike_paid: u128,
    locked: u128,
    delivered: u128,
    returned: u128,
    buyer_fee: u128,
    seller_fee: u128,
    fee_contracts: u128,
    proof: Vec<PhysicalEventProof>,
}

fn selector(signature: &str) -> Vec<u8> {
    Keccak256::digest(signature.as_bytes())[..4].to_vec()
}

async fn call_word<P: EthCallProvider>(
    rpc: &P,
    from: &AccountId,
    engine: &AccountId,
    signature: &str,
    args: &[Address],
) -> Result<U256, String> {
    let mut data = selector(signature);
    for address in args {
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(address.as_slice());
    }
    let result = rpc
        .eth_call(EthCallRequest {
            from: from.clone(),
            to: engine.clone(),
            data,
            value: 0,
            gas_limit: None,
        })
        .await
        .map_err(|_| "covered engine read failed".to_string())?;
    if result.output.len() != 32 {
        return Err("covered engine ABI mismatch".into());
    }
    Ok(U256::from_be_slice(&result.output))
}

fn word_address(word: U256) -> Result<Address, String> {
    let bytes = word.to_be_bytes::<32>();
    if bytes[..12].iter().any(|byte| *byte != 0) {
        return Err("invalid address word".into());
    }
    let address = Address::from_slice(&bytes[12..]);
    if address == Address::ZERO {
        return Err("zero contract address".into());
    }
    Ok(address)
}

fn amount(event: &OptionExecutionEvent, key: &str) -> Result<u128, String> {
    event
        .decoded
        .as_ref()
        .and_then(|decoded| decoded[key].as_str())
        .and_then(|text| text.parse::<u128>().ok())
        .ok_or_else(|| format!("invalid physical {key}"))
}

fn add(total: &mut u128, value: u128) -> Result<(), String> {
    *total = total.checked_add(value).ok_or("physical amount overflow")?;
    Ok(())
}

fn project_event(
    totals: &mut Totals,
    event: &OptionExecutionEvent,
    underlying_per_contract: u128,
    strike_per_contract: u128,
) -> Result<(), String> {
    let qty = amount(event, "quantity")?;
    if qty == 0 {
        return Err("zero physical event quantity".into());
    }
    let underlying = qty
        .checked_mul(underlying_per_contract)
        .ok_or("underlying overflow")?;
    match event.event_name.as_str() {
        "PhysicalFeePaid" => {
            let buyer_fee = amount(event, "buyerFee")?;
            let seller_fee = amount(event, "sellerFee")?;
            let recipient = event
                .decoded
                .as_ref()
                .and_then(|decoded| decoded["recipient"].as_str())
                .ok_or("missing fee recipient")?;
            if buyer_fee
                .checked_add(seller_fee)
                .ok_or("fee amount overflow")?
                > 0
                && (recipient.eq_ignore_ascii_case(event.buyer.as_deref().unwrap_or(""))
                    || recipient.eq_ignore_ascii_case(event.seller.as_deref().unwrap_or("")))
            {
                return Err("fee recipient overlaps trader".into());
            }
            add(&mut totals.buyer_fee, buyer_fee)?;
            add(&mut totals.seller_fee, seller_fee)?;
            add(&mut totals.fee_contracts, qty)?;
        }
        "CoveredFill" => {
            if amount(event, "lockedUnderlying")? != underlying {
                return Err("fill cover mismatch".into());
            }
            add(&mut totals.filled, qty)?;
            add(&mut totals.open, qty)?;
            add(&mut totals.locked, underlying)?;
            add(&mut totals.premium, amount(event, "premium")?)?;
        }
        "PhysicalExercise" => {
            if amount(event, "underlyingDelivered")? != underlying
                || amount(event, "strikePaid")?
                    != qty
                        .checked_mul(strike_per_contract)
                        .ok_or("strike overflow")?
            {
                return Err("physical delivery mismatch".into());
            }
            totals.open = totals
                .open
                .checked_sub(qty)
                .ok_or("exercise exceeds fills")?;
            add(&mut totals.exercised, qty)?;
            add(&mut totals.strike_paid, amount(event, "strikePaid")?)?;
            add(&mut totals.delivered, underlying)?;
        }
        "LongWaived" | "LongExpired" => {
            if amount(event, "underlyingReturned")? != underlying {
                return Err("physical return mismatch".into());
            }
            totals.open = totals.open.checked_sub(qty).ok_or("return exceeds fills")?;
            if event.event_name == "LongWaived" {
                add(&mut totals.waived, qty)?;
            } else {
                add(&mut totals.lapsed, qty)?;
            }
            add(&mut totals.returned, underlying)?;
        }
        _ => return Err("unexpected physical event".into()),
    }
    totals.proof.push(PhysicalEventProof {
        event: event.event_name.clone(),
        quantity: qty.to_string(),
        tx_hash: event.tx_hash.clone(),
        block_hash: event
            .block_hash
            .clone()
            .ok_or("missing canonical block hash")?,
        log_index: event.log_index,
    });
    Ok(())
}

async fn chain_timestamp(rpc_url: &str) -> Result<u64, String> {
    let response: serde_json::Value = reqwest::Client::new().post(rpc_url)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["latest",false]}))
        .send().await.map_err(|_| "chain clock unavailable")?
        .json().await.map_err(|_| "invalid chain clock")?;
    let raw = response["result"]["timestamp"]
        .as_str()
        .ok_or("missing chain timestamp")?;
    u64::from_str_radix(raw.trim_start_matches("0x"), 16)
        .map_err(|_| "invalid chain timestamp".into())
}

pub async fn physical_stack_read(
    State(state): State<AppState>,
) -> Result<Json<Envelope<PhysicalStackData>>, TradingApiError> {
    let unavailable = |reason: &str| {
        TradingApiError::new(
            TradingErrorCode::SourceUnavailable,
            reason.to_owned(),
            MetaBlock::new(&state, "physical_stack"),
        )
    };
    if state.chain_id != 84532 {
        return Err(unavailable(
            "physical stack requires Base Sepolia chain identity",
        ));
    }
    let cfg = &state.trading_views;
    let registry = cfg
        .physical_registry_address
        .as_ref()
        .ok_or_else(|| unavailable("physical Registry not configured"))?;
    let engine = cfg
        .physical_engine_address
        .as_ref()
        .ok_or_else(|| unavailable("physical engine not configured"))?;
    let matcher = cfg
        .physical_matching_engine_address
        .as_ref()
        .ok_or_else(|| unavailable("physical matcher not configured"))?;
    let rpc_url = state
        .execution_config
        .rpc_url
        .as_deref()
        .ok_or_else(|| unavailable("RPC unavailable"))?;
    let repo = state
        .repository
        .as_ref()
        .ok_or_else(|| unavailable("PostgreSQL unavailable"))?;
    let rpc = HttpJsonRpcProvider::new(rpc_url);
    if rpc
        .chain_id()
        .await
        .map_err(|_| unavailable("RPC chain unavailable"))?
        != state.chain_id
    {
        return Err(unavailable("RPC chain mismatch"));
    }
    let from = &state.execution_config.executor_from_address;
    macro_rules! read_word {
        ($address:expr, $signature:literal) => {
            call_word(&rpc, from, $address, $signature, &[])
                .await
                .map_err(|_| unavailable("physical contract read failed"))?
        };
    }
    let registry_owner = word_address(read_word!(registry, "owner()"))
        .map_err(|_| unavailable("Registry owner invalid"))?;
    let engine_owner = word_address(read_word!(engine, "owner()"))
        .map_err(|_| unavailable("engine owner invalid"))?;
    let matcher_owner = word_address(read_word!(matcher, "owner()"))
        .map_err(|_| unavailable("matcher owner invalid"))?;
    if registry_owner != engine_owner || engine_owner != matcher_owner {
        return Err(unavailable("physical governance owners differ"));
    }
    let engine_registry = word_address(read_word!(engine, "registry()"))
        .map_err(|_| unavailable("engine Registry pointer invalid"))?;
    let matcher_registry = word_address(read_word!(matcher, "optionRegistry()"))
        .map_err(|_| unavailable("matcher Registry pointer invalid"))?;
    let matcher_engine = word_address(read_word!(matcher, "marginEngine()"))
        .map_err(|_| unavailable("matcher engine pointer invalid"))?;
    if !engine_registry
        .to_string()
        .eq_ignore_ascii_case(&registry.0)
        || !matcher_registry
            .to_string()
            .eq_ignore_ascii_case(&registry.0)
        || !matcher_engine.to_string().eq_ignore_ascii_case(&engine.0)
    {
        return Err(unavailable("physical contract pointers differ"));
    }
    let total_series = read_word!(registry, "totalSeries()");
    let registry_paused = read_word!(registry, "paused()") != U256::ZERO;
    let engine_paused = read_word!(engine, "paused()") != U256::ZERO;
    let matcher_paused = read_word!(matcher, "paused()") != U256::ZERO;
    let max_open_contracts = read_word!(engine, "maxOpenContracts()");
    let matching_engine_bound = read_word!(engine, "matchingEngine()") != U256::ZERO;
    let series_bound = read_word!(engine, "seriesBound()") != U256::ZERO;
    let indexed_series = repo
        .list_option_series()
        .await
        .map_err(|_| unavailable("Options series projection unavailable"))?
        .into_iter()
        .filter(|series| {
            series.onchain_chain_id == Some(state.chain_id)
                && series
                    .onchain_registry_address
                    .as_deref()
                    .is_some_and(|address| address.eq_ignore_ascii_case(&registry.0))
        })
        .count();
    if total_series == U256::ZERO && indexed_series != 0 {
        return Err(unavailable("indexed physical series absent on-chain"));
    }
    let inactive = total_series == U256::ZERO
        && indexed_series == 0
        && engine_paused
        && max_open_contracts == U256::ZERO
        && !matching_engine_bound
        && !series_bound;
    Ok(Json(Envelope {
        status: if inactive { "ok" } else { "partial" },
        data: PhysicalStackData {
            chain_id: state.chain_id,
            registry: registry.0.to_ascii_lowercase(),
            engine: engine.0.to_ascii_lowercase(),
            matcher: matcher.0.to_ascii_lowercase(),
            governance_owner: format!("{registry_owner:#x}"),
            series_count_onchain: total_series.to_string(),
            series_count_indexed: indexed_series,
            registry_paused,
            engine_paused,
            matcher_paused,
            max_open_contracts: max_open_contracts.to_string(),
            matching_engine_bound,
            economic_state: if inactive { "inactive" } else { "unverified" },
        },
        warnings: if inactive {
            Vec::new()
        } else {
            vec![Warning {
                code: "PHYSICAL_STACK_STATE_UNVERIFIED".into(),
                message: "Physical stack is no longer in its initial inactive state".into(),
                details: serde_json::json!({}),
            }]
        },
        meta: MetaBlock::new(&state, "rpc+db"),
    }))
}

pub async fn physical_positions_authorized(
    State(state): State<AppState>,
    Path(address): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Envelope<PhysicalPositionsData>>, TradingApiError> {
    authorize_account_read(&state, &address, &headers).await?;
    let unavailable = |reason: &str| {
        TradingApiError::new(
            TradingErrorCode::SourceUnavailable,
            reason.to_owned(),
            MetaBlock::new(&state, "physical_indexer"),
        )
    };
    let engine = state
        .trading_views
        .physical_engine_address
        .as_ref()
        .ok_or_else(|| unavailable("physical Options are not configured"))?;
    let config = &state.option_event_indexer_config;
    if !engine
        .0
        .eq_ignore_ascii_case(&config.margin_engine_address.0)
        && !config
            .secondary_margin_engine_address
            .as_ref()
            .is_some_and(|a| a.0.eq_ignore_ascii_case(&engine.0))
    {
        return Err(unavailable("physical engine is not indexed"));
    }
    let repo = state
        .repository
        .as_ref()
        .ok_or_else(|| unavailable("PostgreSQL unavailable"))?;
    let rpc_url = config
        .rpc_url
        .as_deref()
        .ok_or_else(|| unavailable("RPC unavailable"))?;
    let rpc = HttpJsonRpcProvider::new(rpc_url);
    let cursor = repo
        .get_option_event_indexer_state(&config.cursor_id())
        .await
        .map_err(|_| unavailable("physical indexer cursor unavailable"))?
        .ok_or_else(|| unavailable("physical indexer has not scanned"))?;
    // The API may be read-only while a separate worker maintains the cursor.
    // Refuse old projections even when their former block remains canonical.
    let latest = rpc
        .block_number()
        .await
        .map_err(|_| unavailable("chain head unavailable"))?;
    if latest.saturating_sub(cursor.last_indexed_block)
        > config.confirmation_blocks.saturating_add(5)
    {
        return Err(unavailable("physical indexer cursor is stale"));
    }
    let expected = cursor
        .last_indexed_block_hash
        .as_deref()
        .ok_or_else(|| unavailable("cursor lacks block hash"))?;
    let actual = rpc
        .block_hash(cursor.last_indexed_block)
        .await
        .map_err(|_| unavailable("canonical block unavailable"))?;
    if actual.as_deref() != Some(expected) {
        return Err(unavailable("physical indexer is behind a reorg"));
    }
    let from = &state.execution_config.executor_from_address;
    let registry = word_address(
        call_word(&rpc, from, engine, "registry()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|reason| unavailable(&reason))?;
    let option_id = call_word(&rpc, from, engine, "optionId()", &[])
        .await
        .map_err(|reason| unavailable(&reason))?;
    let underlying = word_address(
        call_word(&rpc, from, engine, "underlying()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|reason| unavailable(&reason))?;
    let quote = word_address(
        call_word(&rpc, from, engine, "quote()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|reason| unavailable(&reason))?;
    let underlying_per_contract = u128::try_from(
        call_word(&rpc, from, engine, "underlyingPerContract()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|_| unavailable("underlying amount too large"))?;
    let strike_per_contract = u128::try_from(
        call_word(&rpc, from, engine, "strikePerContract()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|_| unavailable("strike amount too large"))?;
    let window = u64::try_from(
        call_word(&rpc, from, engine, "exerciseWindow()", &[])
            .await
            .map_err(|reason| unavailable(&reason))?,
    )
    .map_err(|_| unavailable("invalid exercise window"))?;
    if window == 0
        || window > 30 * 86400
        || underlying_per_contract == 0
        || strike_per_contract == 0
    {
        return Err(unavailable("invalid covered engine configuration"));
    }
    let registry_string = format!("{registry:#x}");
    let series_id = crate::options::onchain_option_series_key(
        state.chain_id,
        &registry_string,
        &option_id.to_string(),
    )
    .map_err(|_| unavailable("invalid physical series key"))?;
    let series = repo
        .list_option_series()
        .await
        .map_err(|_| unavailable("series metadata unavailable"))?
        .into_iter()
        .find(|series| {
            series.option_series_id == series_id
                && series
                    .onchain_registry_address
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case(&registry_string))
                && series.onchain_series_id.as_deref() == Some(&option_id.to_string())
                && series.onchain_chain_id == Some(state.chain_id)
                && series
                    .underlying
                    .eq_ignore_ascii_case(&format!("{underlying:#x}"))
                && series
                    .settlement_asset
                    .eq_ignore_ascii_case(&format!("{quote:#x}"))
        })
        .ok_or_else(|| unavailable("physical series metadata unavailable"))?;
    let deadline = series
        .expiry
        .checked_add(window)
        .ok_or_else(|| unavailable("exercise deadline overflow"))?;
    let clock = chain_timestamp(rpc_url)
        .await
        .map_err(|reason| unavailable(&reason))?;
    let events = repo
        .list_physical_option_events_for_wallet(state.chain_id, &address, &engine.0)
        .await
        .map_err(|_| unavailable("physical events unavailable"))?;
    for (index, event) in events.iter().enumerate() {
        if event.event_name != "PhysicalFeePaid" {
            continue;
        }
        let next = events
            .get(index + 1)
            .ok_or_else(|| unavailable("unpaired physical fee"))?;
        if next.event_name != "CoveredFill"
            || next.tx_hash != event.tx_hash
            || next.block_hash != event.block_hash
            || next.log_index != event.log_index + 1
            || next.option_id != event.option_id
            || next.buyer != event.buyer
            || next.seller != event.seller
            || next.quantity_contracts != event.quantity_contracts
        {
            return Err(unavailable("physical fee/fill provenance mismatch"));
        }
    }
    let mut pairs: BTreeMap<(String, String), Totals> = BTreeMap::new();
    for event in &events {
        if event.option_id.as_deref() != Some(&option_id.to_string()) {
            return Err(unavailable("physical event series mismatch"));
        }
        let buyer = event
            .buyer
            .as_deref()
            .ok_or_else(|| unavailable("physical buyer missing"))?
            .to_ascii_lowercase();
        let seller = event
            .seller
            .as_deref()
            .ok_or_else(|| unavailable("physical seller missing"))?
            .to_ascii_lowercase();
        if !buyer.eq_ignore_ascii_case(&address) && !seller.eq_ignore_ascii_case(&address) {
            return Err(unavailable("wallet event scope mismatch"));
        }
        project_event(
            pairs.entry((buyer, seller)).or_default(),
            event,
            underlying_per_contract,
            strike_per_contract,
        )
        .map_err(|reason| unavailable(&reason))?;
    }
    let mut positions = Vec::new();
    for ((buyer, seller), totals) in pairs {
        let buyer_address = Address::from_str(&buyer).map_err(|_| unavailable("invalid buyer"))?;
        let seller_address =
            Address::from_str(&seller).map_err(|_| unavailable("invalid seller"))?;
        let onchain = call_word(
            &rpc,
            from,
            engine,
            "openLots(address,address)",
            &[buyer_address, seller_address],
        )
        .await
        .map_err(|reason| unavailable(&reason))?;
        if onchain != U256::from(totals.open) {
            return Err(unavailable("physical lot reconciliation pending"));
        }
        let wallet_fee = if address.eq_ignore_ascii_case(&buyer) {
            totals.buyer_fee
        } else {
            totals.seller_fee
        };
        positions.push(PhysicalPosition {
            buyer,
            seller,
            filled_contracts: totals.filled.to_string(),
            open_contracts: totals.open.to_string(),
            exercised_contracts: totals.exercised.to_string(),
            lapsed_contracts: totals.lapsed.to_string(),
            waived_contracts: totals.waived.to_string(),
            premium_native: totals.premium.to_string(),
            strike_paid_native: totals.strike_paid.to_string(),
            underlying_locked_native: totals.locked.to_string(),
            underlying_delivered_native: totals.delivered.to_string(),
            underlying_returned_native: totals.returned.to_string(),
            fee_native: (totals.fee_contracts == totals.filled).then(|| wallet_fee.to_string()),
            buyer_fee_native: (totals.fee_contracts == totals.filled)
                .then(|| totals.buyer_fee.to_string()),
            seller_fee_native: (totals.fee_contracts == totals.filled)
                .then(|| totals.seller_fee.to_string()),
            proof: totals.proof,
        });
    }
    Ok(Json(Envelope {
        status: "partial",
        data: PhysicalPositionsData {
            address, chain_id: state.chain_id, registry: registry_string, engine: engine.0.clone(),
            series_id, option_id: option_id.to_string(), underlying: format!("{underlying:#x}"),
            quote: format!("{quote:#x}"), underlying_per_contract_native: underlying_per_contract.to_string(),
            strike_per_contract_native: strike_per_contract.to_string(), expiry: series.expiry,
            exercise_deadline: deadline, chain_timestamp: clock, positions,
        },
        warnings: vec![Warning { code: "SOURCE_UNAVAILABLE".into(),
            message: "Physical quantities match the contract. Fills between one buyer and seller are fungible; per-fill lots remain non-canonical.".into(),
            details: serde_json::json!({}) }],
        meta: MetaBlock::new(&state, "chain+db"),
    }))
}
