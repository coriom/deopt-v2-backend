//! PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — V2 on-chain readers and
//! executor-readiness preflight (§§5-8, §§17-19).
//!
//! # What this module owns
//!
//! * The **trait interface** to `PerpEngineV2` /
//!   `PerpMatchingEngineV2` / `CollateralVault` reads that the V2
//!   executor path depends on. Every V2 read routes through a trait
//!   (`V2EngineReader`, `V2MatchingEngineReader`, `V2VaultReader`) so
//!   test doubles can drive the preflight deterministically without
//!   a live chain.
//!
//! * The **preflight aggregator** (`V2Preflight`) which composes those
//!   reads into a single fail-closed check that must pass before any
//!   V2 intent becomes eligible for eth_call simulation.
//!
//! # What this module does NOT own
//!
//! * **Real RPC implementations** of the above traits. Those live
//!   alongside the existing `RpcNonceReader` / `RpcMarkPriceReader`
//!   in `api/perps_cosign.rs` and will be wired in the follow-up
//!   milestone `PERPS_V2_BACKEND_ANVIL_E2E_V1` (spec §16), where the
//!   full local Anvil V2 deployment fixture stands up and lets us
//!   test the wire-level RPC codec end-to-end.
//!
//! * **Broadcast**. This milestone stops at simulation_ok. No raw
//!   transaction is signed, no `sendRawTransaction` is issued, no
//!   `LocalKeystore` is loaded, no arming gate is toggled.
//!
//! # Migration-state invariant (§6)
//!
//! `PerpEngineV2` starts every deployment in `MIGRATION_OPEN` and
//! transitions irrevocably to `MIGRATION_SEALED` only through
//! `sealMigration(snapshotHash)`. Backend V2 readiness REQUIRES
//! `MIGRATION_SEALED`; there is no bypass on real chains. A candidate
//! whose target engine is `MIGRATION_OPEN` fails preflight before
//! reaching simulation.
//!
//! # Clearing invariants (§§7-8)
//!
//! Two independent checks:
//!
//! 1. **Identity**. On-chain `engine.clearingAccount()` MUST equal
//!    the operator-configured `PERP_CLEARING_ACCOUNT_V2_ADDRESS`.
//!    Contract code MUST exist at that address in a live-chain
//!    context (deferred to the RPC impl).
//!
//! 2. **Operational floor**. Vault settlement-asset balance held by
//!    the clearing account MUST be ≥ `PERPS_V2_CLEARING_MIN_BALANCE_RAW`.
//!    This is an OPERATOR SAFETY NET, not an exact per-trade
//!    solvency proof. The exact per-trade requirement is validated
//!    by `eth_call` simulation at the deployed V2 PME
//!    (`PerpMatchingEngineV2._executeSingle` → `PerpEngineV2.applyTrade`).

use crate::error::{BackendError, Result};
use crate::execution::config::ExecutionConfig;
use crate::execution::perp_trade::PerpsProtocolVersion;
use crate::execution::rpc::{EthCallProvider, EthCallRequest};
use crate::signing::eip712::parse_evm_address;
use crate::types::AccountId;
use alloy_sol_types::{sol, SolCall};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 (§5) — canonical
// ABI declarations for the V2 read surface. The `sol!` macro
// computes each function's 4-byte selector at compile time from
// its exact Solidity signature; a signature drift here produces a
// selector mismatch on the deployed contract and the read reverts
// with `execution reverted` at the JSON-RPC layer.
//
// Wrapped in a nested module so identifiers do not collide with the
// V2 `executeTrade` codec in `abi.rs`.
pub mod v2_abi {
    use alloy_sol_types::sol;

    sol! {
        // PerpEngineV2 reads.
        function migrationState() external view returns (uint8);
        function migrationSnapshotHash() external view returns (bytes32);
        function clearingAccount() external view returns (address);

        // PerpMatchingEngineV2 reads.
        function isExecutor(address) external view returns (bool);
        function nonces(address) external view returns (uint256);
        function perpEngine() external view returns (address);
        function paused() external view returns (bool);

        // CollateralVault reads. `balances` is a public mapping so
        // Solidity auto-generates `balances(address user, address
        // token) returns (uint256)`.
        function balances(address, address) external view returns (uint256);
    }
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — canonical mirror of
/// `PerpEngineV2`'s `MigrationState` enum. Wire order matches
/// Solidity `enum MigrationState { Open, Sealed }` at
/// `src/perp/PerpEngineTradingV2.sol` (sol HEAD `2e9ad6f`).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    Open = 0,
    Sealed = 1,
}

impl MigrationState {
    /// Decode from the u8 wire form returned by
    /// `PerpEngineV2.migrationState()` (Solidity emits enum values as
    /// their zero-indexed uint8). Any other value is an ABI drift
    /// and must fail closed — production must never silently accept
    /// an unknown state as either `Open` or `Sealed`.
    pub fn parse_u8(byte: u8) -> Result<Self> {
        match byte {
            0 => Ok(Self::Open),
            1 => Ok(Self::Sealed),
            other => Err(BackendError::Config(format!(
                "unknown PerpEngineV2 migrationState byte: {other} (expected 0=Open or 1=Sealed)"
            ))),
        }
    }

    pub const fn is_sealed(self) -> bool {
        matches!(self, Self::Sealed)
    }
}

pub type ReaderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — read-only interface to
/// `PerpEngineV2`. Every V2 preflight bind goes through here so
/// tests can drive migration state / clearing address deterministically
/// without deploying to Anvil.
pub trait V2EngineReader: Send + Sync {
    /// `PerpEngineV2.migrationState() -> uint8`
    fn read_migration_state<'a>(&'a self) -> ReaderFuture<'a, MigrationState>;

    /// `PerpEngineV2.migrationSnapshotHash() -> bytes32`
    ///
    /// Returned as raw 32 bytes. Zero hash on an OPEN migration is
    /// expected; the preflight checks a non-zero hash only when
    /// `read_migration_state == Sealed`.
    fn read_migration_snapshot_hash<'a>(&'a self) -> ReaderFuture<'a, [u8; 32]>;

    /// `PerpEngineV2.clearingAccount() -> address`
    ///
    /// Returned as an `AccountId` for consistency with the rest of
    /// the backend's address plumbing.
    fn read_clearing_account<'a>(&'a self) -> ReaderFuture<'a, AccountId>;
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — read-only interface to
/// `PerpMatchingEngineV2`. Same trait-based abstraction as
/// `V2EngineReader`.
pub trait V2MatchingEngineReader: Send + Sync {
    /// `PerpMatchingEngineV2.isExecutor(address) -> bool`
    fn read_is_executor<'a>(&'a self, runtime: &'a AccountId) -> ReaderFuture<'a, bool>;

    /// `PerpMatchingEngineV2.nonces(address) -> uint256`
    ///
    /// V2 PME nonces start FRESH at zero (see
    /// `PERPS_V2_BACKEND_COMPAT_FOUNDATION_V1.md` — safe because
    /// EIP-712 domain "2" + distinct verifyingContract make V1
    /// signatures cryptographically un-replayable on V2).
    fn read_v2_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128>;

    /// `PerpMatchingEngineV2.perpEngine() -> address` — used to
    /// prove PME↔Engine linkage matches configured pair.
    fn read_configured_engine<'a>(&'a self) -> ReaderFuture<'a, AccountId>;

    /// `PerpMatchingEngineV2.paused() -> bool` — future-guard;
    /// implementation may return `false` if the contract does not
    /// expose a paused flag at read time.
    fn read_paused<'a>(&'a self) -> ReaderFuture<'a, bool>;
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — read-only interface to
/// `CollateralVault` for the settlement-asset balance held by the
/// clearing account. This is intentionally decoupled from the V2
/// engine reader because the Vault is a SHARED contract across V1
/// and V2, and this read must equally work while V1 is still active.
pub trait V2VaultReader: Send + Sync {
    /// `CollateralVault.internalBalance(clearingAccount, settlementAsset) -> uint256`
    ///
    /// Returned as raw base units (mUSDC has 6 decimals). Bounded to
    /// u128 to reject any value that overflows the backend's
    /// canonical arithmetic width; that's a defensive check — no
    /// realistic settlement balance is anywhere near 2^128.
    fn read_clearing_settlement_balance<'a>(
        &'a self,
        clearing_account: &'a AccountId,
        settlement_asset: &'a AccountId,
    ) -> ReaderFuture<'a, u128>;
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — outcome of the composite V2
/// preflight (§§17-19). Every field records what was observed at
/// preflight time so the failure mode is grepable in
/// production logs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct V2PreflightReport {
    pub active_version: PerpsProtocolVersion,
    pub intent_version: PerpsProtocolVersion,
    pub configured_engine: AccountId,
    pub configured_pme: AccountId,
    pub configured_clearing: AccountId,
    pub on_chain_clearing_account: Option<AccountId>,
    pub migration_state: Option<MigrationState>,
    pub migration_snapshot_hash: Option<[u8; 32]>,
    pub pme_is_executor: Option<bool>,
    pub pme_configured_engine: Option<AccountId>,
    pub pme_paused: Option<bool>,
    pub clearing_balance_raw: Option<u128>,
    pub clearing_min_balance_raw: u128,
    pub outcome: V2PreflightOutcome,
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — one-hot preflight verdict.
/// The `Denied` payload names the exact failed invariant so
/// downstream logs / metrics can distinguish "no clearing" from
/// "clearing too small" etc.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "reason")]
pub enum V2PreflightOutcome {
    Ready,
    Denied(V2PreflightDenial),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "code", content = "detail")]
pub enum V2PreflightDenial {
    /// The persisted intent's protocol_version was V1 but this
    /// preflight is V2-only. Callers must not invoke the V2
    /// preflight for a V1 intent — the check is fail-closed to
    /// surface the bug early.
    IntentIsV1,
    /// `PERPS_ACTIVE_ENGINE_VERSION != v2` at preflight time. V2
    /// broadcast requires the runtime to be actively on V2.
    ActiveVersionNotV2,
    /// One of the V2 addresses is unconfigured or zero.
    ConfigMissing(&'static str),
    /// `PerpEngineV2.clearingAccount()` returned an address that
    /// does not equal `PERP_CLEARING_ACCOUNT_V2_ADDRESS`.
    ClearingAccountMismatch {
        expected: AccountId,
        observed: AccountId,
    },
    /// `PerpEngineV2.migrationState()` returned `Open`. V2
    /// broadcast is refused until sealMigration completes.
    MigrationOpen,
    /// `PerpEngineV2.migrationSnapshotHash()` is zero while the
    /// engine reports `Sealed`. This is a Solidity-side invariant
    /// violation (sealMigration rejects a zero snapshotHash) and
    /// must fail closed rather than proceeding on partial state.
    SealedButSnapshotHashZero,
    /// `PerpMatchingEngineV2.perpEngine()` does not equal
    /// `PERP_ENGINE_V2_ADDRESS`. Someone (mis)configured the PME
    /// against a different engine.
    PmeEngineLinkageMismatch {
        expected: AccountId,
        observed: AccountId,
    },
    /// The configured runtime executor is not authorised on the V2
    /// PME. `setExecutor(runtime, true)` was not issued.
    ExecutorNotAuthorized(AccountId),
    /// V2 PME reports `paused()`.
    PmePaused,
    /// Vault settlement-asset balance held by the clearing account
    /// is below the configured operational floor. This is an
    /// operator safety net; the exact per-trade requirement is
    /// validated by eth_call simulation.
    ClearingBalanceBelowFloor { observed: u128, floor: u128 },
    /// One of the upstream RPC reads failed. The error string is
    /// preserved for grep-ability in logs.
    UpstreamRpcError(String),
}

impl V2PreflightOutcome {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// PERPS_V2_BACKEND_EXECUTOR_PATH_V1 — composite preflight. All
/// reads route through the traits, so tests supply
/// deterministic mocks and prove every denial-branch fires under
/// the right conditions.
///
/// The order of checks is intentional: cheap config checks first,
/// then chain reads. A denial short-circuits — no downstream RPC is
/// issued once an earlier check fails.
pub async fn v2_preflight_check(
    config: &ExecutionConfig,
    intent_version: PerpsProtocolVersion,
    runtime_executor: &AccountId,
    settlement_asset: &AccountId,
    engine: &(dyn V2EngineReader + Send + Sync),
    pme: &(dyn V2MatchingEngineReader + Send + Sync),
    vault: &(dyn V2VaultReader + Send + Sync),
) -> V2PreflightReport {
    let placeholder_addr =
        || AccountId::new("0x0000000000000000000000000000000000000000");
    let mut report = V2PreflightReport {
        active_version: config.perps_active_engine_version,
        intent_version,
        configured_engine: config
            .perp_engine_v2_address
            .clone()
            .unwrap_or_else(placeholder_addr),
        configured_pme: config
            .perp_matching_engine_v2_address
            .clone()
            .unwrap_or_else(placeholder_addr),
        configured_clearing: config
            .perp_clearing_account_v2_address
            .clone()
            .unwrap_or_else(placeholder_addr),
        on_chain_clearing_account: None,
        migration_state: None,
        migration_snapshot_hash: None,
        pme_is_executor: None,
        pme_configured_engine: None,
        pme_paused: None,
        clearing_balance_raw: None,
        clearing_min_balance_raw: config.perps_v2_clearing_min_balance_raw,
        outcome: V2PreflightOutcome::Ready,
    };

    // Cheap config checks first.
    if intent_version == PerpsProtocolVersion::V1 {
        report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::IntentIsV1);
        return report;
    }
    if config.perps_active_engine_version != PerpsProtocolVersion::V2 {
        report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::ActiveVersionNotV2);
        return report;
    }
    for (opt, name) in [
        (
            config.perp_engine_v2_address.as_ref(),
            "PERP_ENGINE_V2_ADDRESS",
        ),
        (
            config.perp_matching_engine_v2_address.as_ref(),
            "PERP_MATCHING_ENGINE_V2_ADDRESS",
        ),
        (
            config.perp_clearing_account_v2_address.as_ref(),
            "PERP_CLEARING_ACCOUNT_V2_ADDRESS",
        ),
    ] {
        match opt {
            None => {
                report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::ConfigMissing(name));
                return report;
            }
            Some(addr)
                if addr
                    .0
                    .eq_ignore_ascii_case("0x0000000000000000000000000000000000000000") =>
            {
                report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::ConfigMissing(name));
                return report;
            }
            _ => {}
        }
    }

    // Migration state — cheapest single read, ordered first because
    // a `MIGRATION_OPEN` engine cannot execute anything so we can
    // short-circuit before we ever read clearing state.
    match engine.read_migration_state().await {
        Ok(state) => {
            report.migration_state = Some(state);
            if !state.is_sealed() {
                report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::MigrationOpen);
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // Snapshot hash must be non-zero when sealed (Sol-side invariant).
    match engine.read_migration_snapshot_hash().await {
        Ok(hash) => {
            report.migration_snapshot_hash = Some(hash);
            if hash == [0u8; 32] {
                report.outcome =
                    V2PreflightOutcome::Denied(V2PreflightDenial::SealedButSnapshotHashZero);
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // Clearing identity — engine.clearingAccount() must match config.
    match engine.read_clearing_account().await {
        Ok(observed) => {
            report.on_chain_clearing_account = Some(observed.clone());
            let expected = report.configured_clearing.clone();
            if !observed.0.eq_ignore_ascii_case(&expected.0) {
                report.outcome = V2PreflightOutcome::Denied(
                    V2PreflightDenial::ClearingAccountMismatch { expected, observed },
                );
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // PME↔Engine linkage.
    match pme.read_configured_engine().await {
        Ok(observed) => {
            report.pme_configured_engine = Some(observed.clone());
            let expected = report.configured_engine.clone();
            if !observed.0.eq_ignore_ascii_case(&expected.0) {
                report.outcome = V2PreflightOutcome::Denied(
                    V2PreflightDenial::PmeEngineLinkageMismatch { expected, observed },
                );
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // PME.isExecutor(runtime executor).
    match pme.read_is_executor(runtime_executor).await {
        Ok(authorized) => {
            report.pme_is_executor = Some(authorized);
            if !authorized {
                report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::ExecutorNotAuthorized(
                    runtime_executor.clone(),
                ));
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // PME.paused().
    match pme.read_paused().await {
        Ok(paused) => {
            report.pme_paused = Some(paused);
            if paused {
                report.outcome = V2PreflightOutcome::Denied(V2PreflightDenial::PmePaused);
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    // Clearing balance vs floor.
    match vault
        .read_clearing_settlement_balance(&report.configured_clearing, settlement_asset)
        .await
    {
        Ok(balance) => {
            report.clearing_balance_raw = Some(balance);
            if balance < report.clearing_min_balance_raw {
                report.outcome =
                    V2PreflightOutcome::Denied(V2PreflightDenial::ClearingBalanceBelowFloor {
                        observed: balance,
                        floor: report.clearing_min_balance_raw,
                    });
                return report;
            }
        }
        Err(error) => {
            report.outcome =
                V2PreflightOutcome::Denied(V2PreflightDenial::UpstreamRpcError(error.to_string()));
            return report;
        }
    }

    report.outcome = V2PreflightOutcome::Ready;
    report
}

// ────────────────────────────────────────────────────────────────
// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 (§5)
// Real live-chain RPC-backed implementations of the reader traits.
// ────────────────────────────────────────────────────────────────

fn decode_word32(bytes: &[u8], name: &str) -> Result<[u8; 32]> {
    if bytes.len() != 32 {
        return Err(BackendError::Config(format!(
            "{name} return length {} != 32",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn decode_address(bytes: &[u8], name: &str) -> Result<AccountId> {
    let word = decode_word32(bytes, name)?;
    // Solidity encodes address in a 32-byte word, right-padded — the
    // 12 high bytes MUST be zero for a valid address encoding.
    for byte in &word[..12] {
        if *byte != 0 {
            return Err(BackendError::Config(format!(
                "{name} address-word upper 12 bytes non-zero"
            )));
        }
    }
    let mut out = String::from("0x");
    for byte in &word[12..] {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(AccountId::new(out))
}

fn decode_bool(bytes: &[u8], name: &str) -> Result<bool> {
    let word = decode_word32(bytes, name)?;
    // Solidity `bool` encodes as a 32-byte word with value 0 or 1.
    match word[31] {
        0 => {
            for byte in &word[..31] {
                if *byte != 0 {
                    return Err(BackendError::Config(format!(
                        "{name} bool-word non-canonical (upper bytes non-zero)"
                    )));
                }
            }
            Ok(false)
        }
        1 => {
            for byte in &word[..31] {
                if *byte != 0 {
                    return Err(BackendError::Config(format!(
                        "{name} bool-word non-canonical (upper bytes non-zero)"
                    )));
                }
            }
            Ok(true)
        }
        other => Err(BackendError::Config(format!(
            "{name} bool-word LSB = {other} (expected 0 or 1)"
        ))),
    }
}

fn decode_uint8(bytes: &[u8], name: &str) -> Result<u8> {
    let word = decode_word32(bytes, name)?;
    for byte in &word[..31] {
        if *byte != 0 {
            return Err(BackendError::Config(format!(
                "{name} uint8-word overflows (upper bytes non-zero)"
            )));
        }
    }
    Ok(word[31])
}

fn encode_address_arg(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address);
    word
}

/// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — live-chain
/// [`V2EngineReader`] backed by any [`EthCallProvider`]. Selectors
/// are compiled from the exact Solidity signatures via `sol!`, so
/// any Solidity drift produces a compile-time change to the
/// selector constant.
pub struct RpcV2EngineReader<R> {
    pub rpc: R,
    pub engine: AccountId,
}

impl<R> RpcV2EngineReader<R> {
    pub fn new(rpc: R, engine: AccountId) -> Self {
        Self { rpc, engine }
    }
}

impl<R> V2EngineReader for RpcV2EngineReader<R>
where
    R: EthCallProvider + Send + Sync + 'static,
{
    fn read_migration_state<'a>(&'a self) -> ReaderFuture<'a, MigrationState> {
        Box::pin(async move {
            let data = v2_abi::migrationStateCall::SELECTOR.to_vec();
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.engine.clone(),
                    to: self.engine.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            let byte = decode_uint8(&out.output, "migrationState")?;
            MigrationState::parse_u8(byte)
        })
    }

    fn read_migration_snapshot_hash<'a>(&'a self) -> ReaderFuture<'a, [u8; 32]> {
        Box::pin(async move {
            let data = v2_abi::migrationSnapshotHashCall::SELECTOR.to_vec();
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.engine.clone(),
                    to: self.engine.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_word32(&out.output, "migrationSnapshotHash")
        })
    }

    fn read_clearing_account<'a>(&'a self) -> ReaderFuture<'a, AccountId> {
        Box::pin(async move {
            let data = v2_abi::clearingAccountCall::SELECTOR.to_vec();
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.engine.clone(),
                    to: self.engine.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_address(&out.output, "clearingAccount")
        })
    }
}

/// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — live-chain
/// [`V2MatchingEngineReader`] backed by any [`EthCallProvider`].
pub struct RpcV2MatchingEngineReader<R> {
    pub rpc: R,
    pub pme: AccountId,
}

impl<R> RpcV2MatchingEngineReader<R> {
    pub fn new(rpc: R, pme: AccountId) -> Self {
        Self { rpc, pme }
    }
}

impl<R> V2MatchingEngineReader for RpcV2MatchingEngineReader<R>
where
    R: EthCallProvider + Send + Sync + 'static,
{
    fn read_is_executor<'a>(&'a self, runtime: &'a AccountId) -> ReaderFuture<'a, bool> {
        Box::pin(async move {
            let runtime_bytes = parse_evm_address(runtime)?;
            let mut data = Vec::with_capacity(4 + 32);
            data.extend_from_slice(&v2_abi::isExecutorCall::SELECTOR);
            data.extend_from_slice(&encode_address_arg(&runtime_bytes));
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.pme.clone(),
                    to: self.pme.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_bool(&out.output, "isExecutor")
        })
    }

    fn read_v2_nonce<'a>(&'a self, trader: &'a AccountId) -> ReaderFuture<'a, u128> {
        Box::pin(async move {
            let trader_bytes = parse_evm_address(trader)?;
            let mut data = Vec::with_capacity(4 + 32);
            data.extend_from_slice(&v2_abi::noncesCall::SELECTOR);
            data.extend_from_slice(&encode_address_arg(&trader_bytes));
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.pme.clone(),
                    to: self.pme.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            crate::api::perps_cosign::decode_uint256_low128(&out.output, "nonces")
        })
    }

    fn read_configured_engine<'a>(&'a self) -> ReaderFuture<'a, AccountId> {
        Box::pin(async move {
            let data = v2_abi::perpEngineCall::SELECTOR.to_vec();
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.pme.clone(),
                    to: self.pme.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_address(&out.output, "perpEngine")
        })
    }

    fn read_paused<'a>(&'a self) -> ReaderFuture<'a, bool> {
        Box::pin(async move {
            let data = v2_abi::pausedCall::SELECTOR.to_vec();
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.pme.clone(),
                    to: self.pme.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            decode_bool(&out.output, "paused")
        })
    }
}

/// PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 — live-chain
/// [`V2VaultReader`] backed by any [`EthCallProvider`]. Reads
/// `CollateralVault.balances(user, token)` — the public mapping
/// getter that returns the user's deposited settlement-asset
/// balance held by the vault. For the V2 clearing preflight
/// `user` is the `PerpClearingAccountV2` address (the clearing
/// account's own deposit).
pub struct RpcV2VaultReader<R> {
    pub rpc: R,
    pub vault: AccountId,
}

impl<R> RpcV2VaultReader<R> {
    pub fn new(rpc: R, vault: AccountId) -> Self {
        Self { rpc, vault }
    }
}

impl<R> V2VaultReader for RpcV2VaultReader<R>
where
    R: EthCallProvider + Send + Sync + 'static,
{
    fn read_clearing_settlement_balance<'a>(
        &'a self,
        clearing_account: &'a AccountId,
        settlement_asset: &'a AccountId,
    ) -> ReaderFuture<'a, u128> {
        Box::pin(async move {
            let clearing_bytes = parse_evm_address(clearing_account)?;
            let asset_bytes = parse_evm_address(settlement_asset)?;
            let mut data = Vec::with_capacity(4 + 64);
            data.extend_from_slice(&v2_abi::balancesCall::SELECTOR);
            data.extend_from_slice(&encode_address_arg(&clearing_bytes));
            data.extend_from_slice(&encode_address_arg(&asset_bytes));
            let out = self
                .rpc
                .eth_call(EthCallRequest {
                    from: self.vault.clone(),
                    to: self.vault.clone(),
                    data,
                    value: 0,
                    gas_limit: None,
                })
                .await?;
            crate::api::perps_cosign::decode_uint256_low128(&out.output, "balances")
        })
    }
}

// ────────────────────────────────────────────────────────────────
// Tests — deterministic mock-driven proof of every denial branch.
// ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::config::ExecutionConfig;

    // Mock impls for each reader trait. Every field is optional; if
    // set, the mock returns that value. If unset, the mock returns
    // a "not configured for this test" error so we surface any
    // accidental fall-through.

    #[derive(Default)]
    struct MockEngine {
        state: Option<MigrationState>,
        snapshot: Option<[u8; 32]>,
        clearing: Option<AccountId>,
    }
    impl V2EngineReader for MockEngine {
        fn read_migration_state<'a>(&'a self) -> ReaderFuture<'a, MigrationState> {
            let s = self.state;
            Box::pin(async move { s.ok_or_else(|| err("state")) })
        }
        fn read_migration_snapshot_hash<'a>(&'a self) -> ReaderFuture<'a, [u8; 32]> {
            let h = self.snapshot;
            Box::pin(async move { h.ok_or_else(|| err("snapshot")) })
        }
        fn read_clearing_account<'a>(&'a self) -> ReaderFuture<'a, AccountId> {
            let c = self.clearing.clone();
            Box::pin(async move { c.ok_or_else(|| err("clearing")) })
        }
    }

    #[derive(Default)]
    struct MockPme {
        is_executor: Option<bool>,
        engine: Option<AccountId>,
        paused: Option<bool>,
        nonce: Option<u128>,
    }
    impl V2MatchingEngineReader for MockPme {
        fn read_is_executor<'a>(&'a self, _: &'a AccountId) -> ReaderFuture<'a, bool> {
            let b = self.is_executor;
            Box::pin(async move { b.ok_or_else(|| err("is_executor")) })
        }
        fn read_v2_nonce<'a>(&'a self, _: &'a AccountId) -> ReaderFuture<'a, u128> {
            let n = self.nonce;
            Box::pin(async move { n.ok_or_else(|| err("nonce")) })
        }
        fn read_configured_engine<'a>(&'a self) -> ReaderFuture<'a, AccountId> {
            let e = self.engine.clone();
            Box::pin(async move { e.ok_or_else(|| err("engine")) })
        }
        fn read_paused<'a>(&'a self) -> ReaderFuture<'a, bool> {
            let p = self.paused;
            Box::pin(async move { p.ok_or_else(|| err("paused")) })
        }
    }

    #[derive(Default)]
    struct MockVault {
        balance: Option<u128>,
    }
    impl V2VaultReader for MockVault {
        fn read_clearing_settlement_balance<'a>(
            &'a self,
            _: &'a AccountId,
            _: &'a AccountId,
        ) -> ReaderFuture<'a, u128> {
            let b = self.balance;
            Box::pin(async move { b.ok_or_else(|| err("balance")) })
        }
    }

    fn err(what: &str) -> BackendError {
        BackendError::Config(format!("mock: {what} unset"))
    }

    fn base_config() -> ExecutionConfig {
        let mut c = ExecutionConfig::disabled();
        c.perps_active_engine_version = PerpsProtocolVersion::V2;
        c.perp_engine_v2_address = Some(AccountId::new(
            "0x0000000000000000000000000000000000000E01",
        ));
        c.perp_matching_engine_v2_address = Some(AccountId::new(
            "0x0000000000000000000000000000000000000E02",
        ));
        c.perp_clearing_account_v2_address = Some(AccountId::new(
            "0x0000000000000000000000000000000000000E03",
        ));
        c.perps_v2_clearing_min_balance_raw = 10_000_000;
        c
    }

    fn runtime() -> AccountId {
        AccountId::new("0x000000000000000000000000000000000000E999")
    }
    fn settlement() -> AccountId {
        AccountId::new("0x000000000000000000000000000000000000ABCD")
    }

    fn happy_engine(clearing: &AccountId) -> MockEngine {
        MockEngine {
            state: Some(MigrationState::Sealed),
            snapshot: Some({
                let mut h = [0u8; 32];
                h[31] = 0x42;
                h
            }),
            clearing: Some(clearing.clone()),
        }
    }
    fn happy_pme(engine: &AccountId) -> MockPme {
        MockPme {
            is_executor: Some(true),
            engine: Some(engine.clone()),
            paused: Some(false),
            nonce: Some(0),
        }
    }
    fn happy_vault(balance: u128) -> MockVault {
        MockVault {
            balance: Some(balance),
        }
    }

    #[tokio::test]
    async fn happy_path_returns_ready() {
        let config = base_config();
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(config.perps_v2_clearing_min_balance_raw + 1);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(report.outcome, V2PreflightOutcome::Ready);
        assert!(report.outcome.is_ready());
    }

    #[tokio::test]
    async fn intent_v1_is_refused_v2_preflight() {
        let config = base_config();
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V1,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(report.outcome, V2PreflightOutcome::Denied(V2PreflightDenial::IntentIsV1));
    }

    #[tokio::test]
    async fn active_version_v1_is_refused_v2_preflight() {
        let mut config = base_config();
        config.perps_active_engine_version = PerpsProtocolVersion::V1;
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ActiveVersionNotV2)
        );
    }

    #[tokio::test]
    async fn missing_v2_engine_is_refused() {
        let mut config = base_config();
        config.perp_engine_v2_address = None;
        let engine = MockEngine::default();
        let pme = MockPme::default();
        let vault = MockVault::default();
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ConfigMissing(
                "PERP_ENGINE_V2_ADDRESS"
            ))
        );
    }

    #[tokio::test]
    async fn zero_v2_clearing_address_is_refused() {
        let mut config = base_config();
        config.perp_clearing_account_v2_address = Some(AccountId::new(
            "0x0000000000000000000000000000000000000000",
        ));
        let engine = MockEngine::default();
        let pme = MockPme::default();
        let vault = MockVault::default();
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ConfigMissing(
                "PERP_CLEARING_ACCOUNT_V2_ADDRESS"
            ))
        );
    }

    #[tokio::test]
    async fn migration_open_is_refused() {
        let config = base_config();
        let mut engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        engine.state = Some(MigrationState::Open);
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::MigrationOpen)
        );
    }

    #[tokio::test]
    async fn sealed_but_zero_snapshot_hash_is_refused() {
        let config = base_config();
        let mut engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        engine.snapshot = Some([0u8; 32]);
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::SealedButSnapshotHashZero)
        );
    }

    #[tokio::test]
    async fn clearing_account_mismatch_is_refused() {
        let config = base_config();
        let mut engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        engine.clearing = Some(AccountId::new("0x00000000000000000000000000000000000000FF"));
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert!(matches!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ClearingAccountMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn pme_engine_linkage_mismatch_is_refused() {
        let config = base_config();
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let mut pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        pme.engine = Some(AccountId::new("0x00000000000000000000000000000000000000FE"));
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert!(matches!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::PmeEngineLinkageMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn executor_not_authorized_is_refused() {
        let config = base_config();
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let mut pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        pme.is_executor = Some(false);
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert!(matches!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ExecutorNotAuthorized(_))
        ));
    }

    #[tokio::test]
    async fn pme_paused_is_refused() {
        let config = base_config();
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let mut pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        pme.paused = Some(true);
        let vault = happy_vault(100_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::PmePaused)
        );
    }

    #[tokio::test]
    async fn clearing_balance_below_floor_is_refused() {
        let config = base_config(); // floor = 10_000_000
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(9_999_999);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert!(matches!(
            report.outcome,
            V2PreflightOutcome::Denied(V2PreflightDenial::ClearingBalanceBelowFloor { .. })
        ));
    }

    #[tokio::test]
    async fn clearing_balance_at_floor_is_ready() {
        let config = base_config(); // floor = 10_000_000
        let engine = happy_engine(config.perp_clearing_account_v2_address.as_ref().unwrap());
        let pme = happy_pme(config.perp_engine_v2_address.as_ref().unwrap());
        let vault = happy_vault(10_000_000);
        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &runtime(),
            &settlement(),
            &engine,
            &pme,
            &vault,
        )
        .await;
        assert_eq!(report.outcome, V2PreflightOutcome::Ready);
    }

    #[tokio::test]
    async fn migration_state_u8_parses_and_rejects() {
        assert_eq!(MigrationState::parse_u8(0).unwrap(), MigrationState::Open);
        assert_eq!(MigrationState::parse_u8(1).unwrap(), MigrationState::Sealed);
        assert!(MigrationState::parse_u8(2).is_err());
        assert!(MigrationState::parse_u8(255).is_err());
    }

    // ── PERPS_V2_BACKEND_RPC_SIMULATION_INTEGRATION_V1 (§5) —
    // real-RPC reader unit tests. The mock provider records the
    // outbound eth_call parameters (target contract, calldata) and
    // returns programmed bytes; assertions cover both the outbound
    // selector/argument encoding and the inbound decoding.

    use crate::execution::rpc::{EthCallProvider, EthCallRequest, EthCallSuccess};
    use crate::execution::rpc::RpcFuture;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockRpc {
        // (target, calldata) captured for assertions.
        pub captured: Arc<Mutex<Vec<(AccountId, Vec<u8>)>>>,
        // Programmed responses in insertion order.
        pub queued: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl MockRpc {
        fn queue(&self, response: Vec<u8>) {
            self.queued.lock().unwrap().push(response);
        }
        fn captured_len(&self) -> usize {
            self.captured.lock().unwrap().len()
        }
    }
    impl EthCallProvider for MockRpc {
        fn eth_call(&self, request: EthCallRequest) -> RpcFuture<'_, EthCallSuccess> {
            let target = request.to.clone();
            let data = request.data.clone();
            let queued = self.queued.clone();
            let captured = self.captured.clone();
            Box::pin(async move {
                captured.lock().unwrap().push((target, data));
                let mut q = queued.lock().unwrap();
                if q.is_empty() {
                    return Err(BackendError::Config("mock rpc: no queued response".into()));
                }
                let output = q.remove(0);
                Ok(EthCallSuccess {
                    block_number: Some(1),
                    output,
                })
            })
        }
    }

    // Encoding helpers for tests
    fn u256_word(low128: u128) -> Vec<u8> {
        let mut w = [0u8; 32];
        w[16..].copy_from_slice(&low128.to_be_bytes());
        w.to_vec()
    }
    fn addr_word(addr: &str) -> Vec<u8> {
        let stripped = addr.strip_prefix("0x").unwrap();
        let mut w = [0u8; 32];
        for (i, chunk) in stripped.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).unwrap();
            w[12 + i] = u8::from_str_radix(s, 16).unwrap();
        }
        w.to_vec()
    }
    fn bool_word(b: bool) -> Vec<u8> {
        let mut w = [0u8; 32];
        w[31] = if b { 1 } else { 0 };
        w.to_vec()
    }

    const ENGINE_ADDR: &str = "0x0000000000000000000000000000000000000E01";
    const PME_ADDR: &str = "0x0000000000000000000000000000000000000E02";
    const CLEARING_ADDR: &str = "0x0000000000000000000000000000000000000E03";
    const VAULT_ADDR: &str = "0x0000000000000000000000000000000000000E04";
    const ASSET_ADDR: &str = "0x0000000000000000000000000000000000000E05";
    const RUNTIME_ADDR: &str = "0x0000000000000000000000000000000000000E99";
    const TRADER_ADDR: &str = "0xff287410852B9328437eaC353720e5476bC5F837";

    #[tokio::test]
    async fn rpc_engine_reader_migration_state_sealed() {
        let rpc = MockRpc::default();
        let mut sealed = [0u8; 32];
        sealed[31] = 1;
        rpc.queue(sealed.to_vec());
        let reader = RpcV2EngineReader::new(rpc.clone(), AccountId::new(ENGINE_ADDR));
        let state = reader.read_migration_state().await.unwrap();
        assert_eq!(state, MigrationState::Sealed);
        let captured = rpc.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0.0, ENGINE_ADDR);
        assert_eq!(&captured[0].1[..4], &v2_abi::migrationStateCall::SELECTOR);
    }

    #[tokio::test]
    async fn rpc_engine_reader_migration_state_open_and_invalid() {
        let rpc = MockRpc::default();
        rpc.queue([0u8; 32].to_vec()); // Open
        let reader = RpcV2EngineReader::new(rpc.clone(), AccountId::new(ENGINE_ADDR));
        assert_eq!(reader.read_migration_state().await.unwrap(), MigrationState::Open);

        let mut bad = [0u8; 32];
        bad[31] = 2; // invalid enum value
        rpc.queue(bad.to_vec());
        assert!(reader.read_migration_state().await.is_err());
    }

    #[tokio::test]
    async fn rpc_engine_reader_snapshot_hash_and_clearing_account() {
        let rpc = MockRpc::default();
        let mut hash = [0u8; 32];
        hash[31] = 0x42;
        rpc.queue(hash.to_vec());
        rpc.queue(addr_word(CLEARING_ADDR));
        let reader = RpcV2EngineReader::new(rpc.clone(), AccountId::new(ENGINE_ADDR));

        let h = reader.read_migration_snapshot_hash().await.unwrap();
        assert_eq!(h, hash);

        let ca = reader.read_clearing_account().await.unwrap();
        assert!(ca.0.eq_ignore_ascii_case(CLEARING_ADDR));

        let captured = rpc.captured.lock().unwrap();
        assert_eq!(&captured[0].1[..4], &v2_abi::migrationSnapshotHashCall::SELECTOR);
        assert_eq!(&captured[1].1[..4], &v2_abi::clearingAccountCall::SELECTOR);
    }

    #[tokio::test]
    async fn rpc_pme_reader_is_executor_and_nonces_and_engine_and_paused() {
        let rpc = MockRpc::default();
        rpc.queue(bool_word(true));
        rpc.queue(u256_word(42));
        rpc.queue(addr_word(ENGINE_ADDR));
        rpc.queue(bool_word(false));

        let reader = RpcV2MatchingEngineReader::new(rpc.clone(), AccountId::new(PME_ADDR));
        assert!(reader
            .read_is_executor(&AccountId::new(RUNTIME_ADDR))
            .await
            .unwrap());
        assert_eq!(
            reader.read_v2_nonce(&AccountId::new(TRADER_ADDR)).await.unwrap(),
            42
        );
        assert!(reader
            .read_configured_engine()
            .await
            .unwrap()
            .0
            .eq_ignore_ascii_case(ENGINE_ADDR));
        assert!(!reader.read_paused().await.unwrap());

        let captured = rpc.captured.lock().unwrap();
        assert_eq!(captured.len(), 4);
        assert_eq!(&captured[0].1[..4], &v2_abi::isExecutorCall::SELECTOR);
        // isExecutor calldata = selector || 32-byte runtime address
        assert_eq!(captured[0].1.len(), 36);
        assert_eq!(&captured[1].1[..4], &v2_abi::noncesCall::SELECTOR);
        assert_eq!(&captured[2].1[..4], &v2_abi::perpEngineCall::SELECTOR);
        assert_eq!(&captured[3].1[..4], &v2_abi::pausedCall::SELECTOR);
    }

    #[tokio::test]
    async fn rpc_vault_reader_balances() {
        let rpc = MockRpc::default();
        rpc.queue(u256_word(500_000_000));
        let reader = RpcV2VaultReader::new(rpc.clone(), AccountId::new(VAULT_ADDR));
        let balance = reader
            .read_clearing_settlement_balance(
                &AccountId::new(CLEARING_ADDR),
                &AccountId::new(ASSET_ADDR),
            )
            .await
            .unwrap();
        assert_eq!(balance, 500_000_000);
        let captured = rpc.captured.lock().unwrap();
        assert_eq!(&captured[0].1[..4], &v2_abi::balancesCall::SELECTOR);
        // balances calldata = selector || 32-byte user || 32-byte token
        assert_eq!(captured[0].1.len(), 4 + 64);
    }

    #[tokio::test]
    async fn rpc_readers_reject_non_canonical_encodings() {
        // Non-canonical bool: upper bytes nonzero.
        let rpc = MockRpc::default();
        let mut bad_bool = [0xffu8; 32];
        bad_bool[31] = 1;
        rpc.queue(bad_bool.to_vec());
        let reader = RpcV2MatchingEngineReader::new(rpc.clone(), AccountId::new(PME_ADDR));
        assert!(reader
            .read_is_executor(&AccountId::new(RUNTIME_ADDR))
            .await
            .is_err());

        // Address with non-zero upper bytes.
        let rpc2 = MockRpc::default();
        let mut bad_addr = addr_word(ENGINE_ADDR);
        bad_addr[0] = 0xaa; // corrupt the high-byte
        rpc2.queue(bad_addr);
        let reader2 = RpcV2EngineReader::new(rpc2.clone(), AccountId::new(ENGINE_ADDR));
        assert!(reader2.read_clearing_account().await.is_err());
    }

    #[tokio::test]
    async fn rpc_end_to_end_preflight_against_mock() {
        // Compose the real readers with the mock provider and run the
        // full preflight aggregator. Proves the RpcV2* impls satisfy
        // the trait interfaces exactly as the mocks did.
        let rpc = MockRpc::default();
        // Order matches the preflight's call sequence:
        //  1. read_migration_state
        //  2. read_migration_snapshot_hash
        //  3. read_clearing_account
        //  4. read_configured_engine (PME)
        //  5. read_is_executor
        //  6. read_paused
        //  7. read_clearing_settlement_balance
        let mut sealed = [0u8; 32];
        sealed[31] = 1;
        rpc.queue(sealed.to_vec()); // migrationState -> Sealed
        let mut hash = [0u8; 32];
        hash[31] = 0x42;
        rpc.queue(hash.to_vec()); // migrationSnapshotHash -> non-zero
        rpc.queue(addr_word(CLEARING_ADDR)); // clearingAccount
        rpc.queue(addr_word(ENGINE_ADDR)); // pme.perpEngine
        rpc.queue(bool_word(true)); // isExecutor -> true
        rpc.queue(bool_word(false)); // paused -> false
        rpc.queue(u256_word(50_000_000)); // vault balance >= floor

        let engine_reader = RpcV2EngineReader::new(rpc.clone(), AccountId::new(ENGINE_ADDR));
        let pme_reader = RpcV2MatchingEngineReader::new(rpc.clone(), AccountId::new(PME_ADDR));
        let vault_reader = RpcV2VaultReader::new(rpc.clone(), AccountId::new(VAULT_ADDR));

        let mut config = ExecutionConfig::disabled();
        config.perps_active_engine_version = PerpsProtocolVersion::V2;
        config.perp_engine_v2_address = Some(AccountId::new(ENGINE_ADDR));
        config.perp_matching_engine_v2_address = Some(AccountId::new(PME_ADDR));
        config.perp_clearing_account_v2_address = Some(AccountId::new(CLEARING_ADDR));
        config.perps_v2_clearing_min_balance_raw = 10_000_000;

        let report = v2_preflight_check(
            &config,
            PerpsProtocolVersion::V2,
            &AccountId::new(RUNTIME_ADDR),
            &AccountId::new(ASSET_ADDR),
            &engine_reader,
            &pme_reader,
            &vault_reader,
        )
        .await;
        assert_eq!(report.outcome, V2PreflightOutcome::Ready);
        assert_eq!(rpc.captured_len(), 7);
    }
}
