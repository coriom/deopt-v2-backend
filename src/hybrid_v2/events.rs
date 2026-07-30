//! Typed Hybrid V2 canonical events.
//!
//! Every variant maps 1:1 to a canonical Solidity event emitted by a
//! Hybrid V2 module (per `contract-spec/13_EVENTS_AND_RECONSTRUCTION.md`
//! and the pinned `event-topics-v1.json`).
//!
//! The reducer only ever consumes typed events — raw log decoding
//! happens once at the decoder boundary.

use serde::{Deserialize, Serialize};

/// Discrete canonical event kinds emitted by Hybrid V2 modules.
///
/// The kind is stored as `event_kind` in `hybrid_v2_decoded_events` and
/// is the routing key for the projection reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventKind {
    // Registry
    SubaccountCreated,
    SubaccountLazyRegistered,

    // Vault core / capability
    Deposit,
    Withdraw,
    InternalTransfer,
    CollateralLocked,
    CollateralUnlocked,
    EngineCapabilityChanged,
    EngineGuardianRevoked,
    GuardianChanged,
    OrphanedLockReleased,
    SupportedTokenAdded,
    SupportedTokenRemoved,
    CollateralTokenEnteredUniverse,
    OptionPremiumTransferred,
    OptionFeeCharged,
    OptionRebatePaid,
    RecoveryFinalizationWithdrawn,
    PauseFlagChanged,
    BadDebtSocialized,
    ProtocolSubaccountsInitialized,
    EscapeControllerInitialized,
    RecoveryFinalizerInitialized,

    // Options ledger
    OptionPositionOpened,
    OptionPositionModified,
    OptionPositionClosed,
    OptionExercised,
    OptionSettled,
    OptionPositionLiquidated,

    // Options engine
    OptionOrderPairExecuted,
    OptionOrderFilled,
    OptionOrderCancelled,
    OptionSubaccountMinValidOrderNonceAdvanced,

    // Escape controller
    RecoveryRequested,
    RecoveryActivated,
    RecoveryCancelled,
    RecoveryEpochIncremented,
    RecoveryPauseSet,

    // Recovery finalizer
    RecoveryFinalized,

    // Replay + epoch controller
    IntentConsumed,
    NonceCancelled,
    OwnerRecoveryEpochAdvanced,
    SubaccountRecoveryEpochAdvanced,

    // Risk module
    RiskParamsSet,
    RiskModuleActivated,
    LiquidationTriggered,

    // Manifest
    DeploymentManifestDeclared,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::SubaccountCreated => "SubaccountCreated",
            EventKind::SubaccountLazyRegistered => "SubaccountLazyRegistered",
            EventKind::Deposit => "Deposit",
            EventKind::Withdraw => "Withdraw",
            EventKind::InternalTransfer => "InternalTransfer",
            EventKind::CollateralLocked => "CollateralLocked",
            EventKind::CollateralUnlocked => "CollateralUnlocked",
            EventKind::EngineCapabilityChanged => "EngineCapabilityChanged",
            EventKind::EngineGuardianRevoked => "EngineGuardianRevoked",
            EventKind::GuardianChanged => "GuardianChanged",
            EventKind::OrphanedLockReleased => "OrphanedLockReleased",
            EventKind::SupportedTokenAdded => "SupportedTokenAdded",
            EventKind::SupportedTokenRemoved => "SupportedTokenRemoved",
            EventKind::CollateralTokenEnteredUniverse => "CollateralTokenEnteredUniverse",
            EventKind::OptionPremiumTransferred => "OptionPremiumTransferred",
            EventKind::OptionFeeCharged => "OptionFeeCharged",
            EventKind::OptionRebatePaid => "OptionRebatePaid",
            EventKind::RecoveryFinalizationWithdrawn => "RecoveryFinalizationWithdrawn",
            EventKind::PauseFlagChanged => "PauseFlagChanged",
            EventKind::BadDebtSocialized => "BadDebtSocialized",
            EventKind::ProtocolSubaccountsInitialized => "ProtocolSubaccountsInitialized",
            EventKind::EscapeControllerInitialized => "EscapeControllerInitialized",
            EventKind::RecoveryFinalizerInitialized => "RecoveryFinalizerInitialized",
            EventKind::OptionPositionOpened => "OptionPositionOpened",
            EventKind::OptionPositionModified => "OptionPositionModified",
            EventKind::OptionPositionClosed => "OptionPositionClosed",
            EventKind::OptionExercised => "OptionExercised",
            EventKind::OptionSettled => "OptionSettled",
            EventKind::OptionPositionLiquidated => "OptionPositionLiquidated",
            EventKind::OptionOrderPairExecuted => "OptionOrderPairExecuted",
            EventKind::OptionOrderFilled => "OptionOrderFilled",
            EventKind::OptionOrderCancelled => "OptionOrderCancelled",
            EventKind::OptionSubaccountMinValidOrderNonceAdvanced => {
                "OptionSubaccountMinValidOrderNonceAdvanced"
            }
            EventKind::RecoveryRequested => "RecoveryRequested",
            EventKind::RecoveryActivated => "RecoveryActivated",
            EventKind::RecoveryCancelled => "RecoveryCancelled",
            EventKind::RecoveryEpochIncremented => "RecoveryEpochIncremented",
            EventKind::RecoveryPauseSet => "RecoveryPauseSet",
            EventKind::RecoveryFinalized => "RecoveryFinalized",
            EventKind::IntentConsumed => "IntentConsumed",
            EventKind::NonceCancelled => "NonceCancelled",
            EventKind::OwnerRecoveryEpochAdvanced => "OwnerRecoveryEpochAdvanced",
            EventKind::SubaccountRecoveryEpochAdvanced => "SubaccountRecoveryEpochAdvanced",
            EventKind::RiskParamsSet => "RiskParamsSet",
            EventKind::RiskModuleActivated => "RiskModuleActivated",
            EventKind::LiquidationTriggered => "LiquidationTriggered",
            EventKind::DeploymentManifestDeclared => "DeploymentManifestDeclared",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        for kind in Self::all() {
            if kind.as_str() == s {
                return Some(*kind);
            }
        }
        None
    }

    pub fn all() -> &'static [EventKind] {
        &[
            EventKind::SubaccountCreated,
            EventKind::SubaccountLazyRegistered,
            EventKind::Deposit,
            EventKind::Withdraw,
            EventKind::InternalTransfer,
            EventKind::CollateralLocked,
            EventKind::CollateralUnlocked,
            EventKind::EngineCapabilityChanged,
            EventKind::EngineGuardianRevoked,
            EventKind::GuardianChanged,
            EventKind::OrphanedLockReleased,
            EventKind::SupportedTokenAdded,
            EventKind::SupportedTokenRemoved,
            EventKind::CollateralTokenEnteredUniverse,
            EventKind::OptionPremiumTransferred,
            EventKind::OptionFeeCharged,
            EventKind::OptionRebatePaid,
            EventKind::RecoveryFinalizationWithdrawn,
            EventKind::PauseFlagChanged,
            EventKind::BadDebtSocialized,
            EventKind::ProtocolSubaccountsInitialized,
            EventKind::EscapeControllerInitialized,
            EventKind::RecoveryFinalizerInitialized,
            EventKind::OptionPositionOpened,
            EventKind::OptionPositionModified,
            EventKind::OptionPositionClosed,
            EventKind::OptionExercised,
            EventKind::OptionSettled,
            EventKind::OptionPositionLiquidated,
            EventKind::OptionOrderPairExecuted,
            EventKind::OptionOrderFilled,
            EventKind::OptionOrderCancelled,
            EventKind::OptionSubaccountMinValidOrderNonceAdvanced,
            EventKind::RecoveryRequested,
            EventKind::RecoveryActivated,
            EventKind::RecoveryCancelled,
            EventKind::RecoveryEpochIncremented,
            EventKind::RecoveryPauseSet,
            EventKind::RecoveryFinalized,
            EventKind::IntentConsumed,
            EventKind::NonceCancelled,
            EventKind::OwnerRecoveryEpochAdvanced,
            EventKind::SubaccountRecoveryEpochAdvanced,
            EventKind::RiskParamsSet,
            EventKind::RiskModuleActivated,
            EventKind::LiquidationTriggered,
            EventKind::DeploymentManifestDeclared,
        ]
    }
}

/// A decoded Hybrid V2 event. Carries the discriminator plus enough
/// data to feed the reducer. The full raw payload lives in the
/// per-row `payload_json` column of `hybrid_v2_decoded_events` — this
/// struct only captures the projection-relevant fields, keeping the
/// canonical evidence in the raw log journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridV2Event {
    pub kind: EventKind,
    pub event_version: u16,
    pub subkey: Option<String>,
    pub owner: Option<String>,
    pub subaccount_id: Option<u32>,
    pub token: Option<String>,
    pub engine: Option<String>,
    pub execution_id: Option<String>,
    pub order_hash: Option<String>,
    pub series_id: Option<String>,
    /// Full canonical payload as JSON (all indexed + non-indexed fields).
    /// The reducer reads specific fields from here as needed.
    pub payload: serde_json::Value,
}
