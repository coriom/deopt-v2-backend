//! Row types + patch struct for the pre-broadcast execution
//! persistence surface added by
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`.
//!
//! The Postgres + in-memory `HybridV2ProjectionStore` implementations
//! extend their trait surface (in `src/hybrid_v2/persistence.rs`) with:
//!
//!   * `insert_execution_request` — idempotent INSERT keyed on the
//!     canonical execution id.
//!   * `get_execution_request` — point read.
//!   * `update_execution_phase` — conditional forward update guarded
//!     by both the state-machine matrix and the paired
//!     `plan_hash`/`calldata_hash` immutability rule.
//!   * `append_execution_attempt` — append-only history writer.
//!   * `list_execution_requests_by_deployment` — bounded pagination.
//!   * `reserve_executor_nonce` — atomic UNIQUE-based reservation.
//!   * `mark_executor_nonce_abandoned` — historical transition.
//!   * `get_reserved_nonces_for` — restart enumeration.
//!
//! Every field on `ExecutionRequestRow` maps 1:1 to the equally-named
//! column in `migrations/0049_hybrid_v2_execution.sql`.

use crate::hybrid_v2::execution::state::ExecutionPhase;

/// One row of `hybrid_v2_execution_requests`. All String fields
/// carrying 0x-prefixed hex use lowercase; address fields use EIP-55
/// checksum (the shape is only enforced at row insert time by the
/// upstream builder — the store does not re-check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRequestRow {
    pub canonical_execution_id: String,
    pub deployment_id: i64,
    pub chain_id: i64,
    pub execution_kind: String,
    pub buyer_order_hash: String,
    pub seller_order_hash: String,
    pub buyer_subkey: String,
    pub seller_subkey: String,
    pub series_id: String,
    /// NUMERIC(78,0) in Postgres. Serialized/deserialized as the
    /// unpadded decimal string to sidestep the sqlx `BigDecimal`
    /// feature bloat.
    pub fill_quantity_1e8: String,
    pub premium_amount: String,
    pub fee_schedule_epoch: Option<i64>,
    pub source_matched_execution_id: Option<String>,

    // Plan surface
    pub target_contract: String,
    pub selector: String,
    pub calldata_hash: Option<String>,
    pub plan_hash: Option<String>,
    pub tx_value_wei: String,

    // Simulation surface
    pub simulation_block_number: Option<i64>,
    pub simulation_block_hash: Option<String>,
    pub simulation_gas_estimate: Option<i64>,
    pub simulation_result_json: Option<serde_json::Value>,

    // Signature surface
    pub signer_identity: Option<String>,
    pub signing_payload_hash: Option<String>,
    pub signature_r: Option<String>,
    pub signature_s: Option<String>,
    pub signature_v: Option<i16>,
    pub recovered_signer: Option<String>,

    // Gas & nonce
    pub gas_limit: Option<i64>,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
    pub reserved_nonce: Option<i64>,

    // State
    pub phase: ExecutionPhase,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub retry_count: i32,

    // Lock correlation
    pub holder_epoch: Option<i64>,

    // External-signer idempotency key (Part L). 0x-prefixed 16-byte
    // hex derived from
    //   keccak256("HV2_SIGNER_IDEMPOTENCY_V1" ‖ expected_signer_address
    //             ‖ canonical_execution_id ‖ plan_hash
    //             ‖ signing_payload_hash)[..16]
    // Immutable once set — enforced by the migration 0050 trigger and
    // by the store's `update_execution_phase` implementation.
    pub signer_request_idempotency_key: Option<String>,

    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Sparse patch applied atomically with a phase transition. Only
/// `Some(_)` fields are written; `None` leaves the DB column
/// untouched. `plan_hash` and `calldata_hash` are additionally
/// enforced immutable-once-set by both the SQL trigger
/// (`hybrid_v2_execution_requests_plan_immutability_tr`) and the
/// in-memory store's `update_execution_phase` implementation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionRequestPatch {
    pub target_contract: Option<String>,
    pub selector: Option<String>,
    pub calldata_hash: Option<String>,
    pub plan_hash: Option<String>,
    pub tx_value_wei: Option<String>,
    pub simulation_block_number: Option<i64>,
    pub simulation_block_hash: Option<String>,
    pub simulation_gas_estimate: Option<i64>,
    pub simulation_result_json: Option<serde_json::Value>,
    pub signer_identity: Option<String>,
    pub signing_payload_hash: Option<String>,
    pub signature_r: Option<String>,
    pub signature_s: Option<String>,
    pub signature_v: Option<i16>,
    pub recovered_signer: Option<String>,
    pub gas_limit: Option<i64>,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
    pub reserved_nonce: Option<i64>,
    pub failure_class: Option<String>,
    pub failure_detail: Option<String>,
    pub holder_epoch: Option<i64>,
    pub retry_count: Option<i32>,
    /// Persisted external-signer idempotency key. Once written, the
    /// migration 0050 trigger blocks any divergent overwrite.
    pub signer_request_idempotency_key: Option<String>,
}
