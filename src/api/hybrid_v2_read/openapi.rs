//! OpenAPI 3.1 specification for the Hybrid V2 public read API.
//!
//! Frozen as a static JSON. Every route in `router.rs` is documented.
//! Uint256 economic amounts are documented as decimal strings. Cursor is
//! documented as an opaque base64url token. Structured error schema is
//! `ApiErrorBody`.

const OPENAPI_JSON: &str = r##"{
  "openapi": "3.1.0",
  "info": {
    "title": "DeOpt V2 — Hybrid V2 public read API",
    "version": "1.0.0",
    "description": "Read-only projection of the canonical Hybrid V2 subaccount state. EXPERIMENTAL — NOT SECURITY APPROVED. No signing, no chain writes, no order submission."
  },
  "servers": [
    { "url": "https://api.example/hybrid-v2", "description": "example (non-live)" }
  ],
  "components": {
    "schemas": {
      "CanonicalityLevel": {
        "type": "string",
        "enum": ["FINALIZED", "INDEXED_CANONICAL", "INDEXER_BEHIND", "NOT_READY"]
      },
      "ConsistencyMode": {
        "type": "string",
        "enum": ["INDEXED", "FINALIZED"]
      },
      "CanonicalityMetadata": {
        "type": "object",
        "required": [
          "deployment_id", "chain_id", "manifest_hash", "indexed_block",
          "indexed_block_hash", "finalized_block", "observed_head_block",
          "indexer_lag", "reconciliation_status", "canonicality_level",
          "consistency_mode", "generated_at_ms"
        ],
        "properties": {
          "deployment_id": { "type": "integer", "format": "int64" },
          "chain_id": { "type": "integer", "format": "int64" },
          "manifest_hash": { "type": "string", "description": "0x + 64 lowercase hex" },
          "indexed_block": { "type": "integer", "format": "int64" },
          "indexed_block_hash": { "type": "string" },
          "finalized_block": { "type": "integer", "format": "int64" },
          "observed_head_block": { "type": "integer", "format": "int64" },
          "indexer_lag": { "type": "integer", "format": "int64" },
          "reconciliation_status": { "type": "string" },
          "canonicality_level": { "$ref": "#/components/schemas/CanonicalityLevel" },
          "consistency_mode": { "$ref": "#/components/schemas/ConsistencyMode" },
          "generated_at_ms": { "type": "integer", "format": "int64" }
        }
      },
      "ApiErrorBody": {
        "type": "object",
        "required": ["code", "message", "retryable"],
        "properties": {
          "code": { "type": "string", "description": "Stable machine-readable code" },
          "message": { "type": "string" },
          "retryable": { "type": "boolean" },
          "detail": {}
        }
      },
      "DecimalUint": {
        "type": "string",
        "description": "uint128/uint256 as a base-10 decimal string; never a JSON number",
        "example": "1000000000000000000"
      },
      "SignedPpm": {
        "type": "string",
        "description": "Signed parts-per-million as a base-10 decimal string",
        "example": "-125"
      },
      "Address": {
        "type": "string",
        "description": "Ethereum address, lowercase 0x + 40 hex chars"
      },
      "Bytes32": {
        "type": "string",
        "description": "bytes32, lowercase 0x + 64 hex chars"
      },
      "DeploymentListRow": {
        "type": "object",
        "required": [
          "deployment_id", "chain_id", "manifest_hash", "manifest_address",
          "deployment_version", "activation_status", "max_collateral_tokens",
          "max_active_series", "ready"
        ],
        "properties": {
          "deployment_id": { "type": "integer" },
          "chain_id": { "type": "integer" },
          "manifest_hash": { "$ref": "#/components/schemas/Bytes32" },
          "manifest_address": { "$ref": "#/components/schemas/Address" },
          "deployment_version": { "type": "integer" },
          "activation_status": { "type": "string" },
          "max_collateral_tokens": { "type": "integer" },
          "max_active_series": { "type": "integer" },
          "ready": { "type": "boolean" }
        }
      },
      "DeploymentStatus": {
        "type": "object",
        "required": ["deployment_id", "chain_id", "manifest_hash", "ready", "metadata"],
        "properties": {
          "deployment_id": { "type": "integer" },
          "chain_id": { "type": "integer" },
          "manifest_hash": { "$ref": "#/components/schemas/Bytes32" },
          "ready": { "type": "boolean" },
          "ready_reason": { "type": ["string", "null"] },
          "indexed_block": { "type": "integer" },
          "indexed_block_hash": { "type": "string" },
          "finalized_block": { "type": "integer" },
          "observed_head_block": { "type": "integer" },
          "indexer_lag": { "type": "integer" },
          "decode_failures": { "type": "integer" },
          "projection_failures": { "type": "integer" },
          "reorg_count": { "type": "integer" },
          "max_reorg_depth_seen": { "type": "integer" },
          "rebuild_status": { "type": "string" },
          "reconciliation_status": { "type": "string" },
          "metadata": { "$ref": "#/components/schemas/CanonicalityMetadata" }
        }
      },
      "OwnerSubaccountRow": {
        "type": "object",
        "properties": {
          "subaccount_id": { "type": "integer" },
          "subkey": { "$ref": "#/components/schemas/Bytes32" },
          "materialised_via_created": { "type": "boolean" },
          "materialised_via_lazy": { "type": "boolean" },
          "recovery_state": { "type": "string" },
          "finalized": { "type": "boolean" },
          "balance_token_count": { "type": "integer" },
          "reservation_count": { "type": "integer" },
          "active_series_count": { "type": "integer" }
        }
      },
      "OwnerSubaccountsResponse": {
        "type": "object",
        "required": ["owner", "subaccounts"],
        "properties": {
          "owner": { "$ref": "#/components/schemas/Address" },
          "subaccounts": {
            "type": "array",
            "items": { "$ref": "#/components/schemas/OwnerSubaccountRow" }
          }
        }
      },
      "SubaccountSummary": {
        "type": "object",
        "properties": {
          "subkey": { "$ref": "#/components/schemas/Bytes32" },
          "owner": { "$ref": "#/components/schemas/Address" },
          "subaccount_id": { "type": "integer" },
          "recovery_state": { "type": "string" },
          "finalized": { "type": "boolean" },
          "balance_token_count": { "type": "integer" },
          "reservation_count": { "type": "integer" },
          "active_series_count": { "type": "integer" },
          "open_order_count": { "type": "integer" }
        }
      },
      "CollateralRow": {
        "type": "object",
        "properties": {
          "token": { "$ref": "#/components/schemas/Address" },
          "universe_index": { "type": ["integer", "null"] },
          "enabled": { "type": "boolean" },
          "balance": { "$ref": "#/components/schemas/DecimalUint" },
          "aggregate_reserved": { "$ref": "#/components/schemas/DecimalUint" },
          "available": { "$ref": "#/components/schemas/DecimalUint" }
        }
      },
      "ReservationRow": {
        "type": "object",
        "properties": {
          "subkey": { "$ref": "#/components/schemas/Bytes32" },
          "token": { "$ref": "#/components/schemas/Address" },
          "engine": { "$ref": "#/components/schemas/Address" },
          "reserved": { "$ref": "#/components/schemas/DecimalUint" }
        }
      },
      "PositionRow": {
        "type": "object",
        "properties": {
          "series_id": { "$ref": "#/components/schemas/Bytes32" },
          "long_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "short_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "active": { "type": "boolean" },
          "last_event_block": { "type": "integer" }
        }
      },
      "OrderRow": {
        "type": "object",
        "properties": {
          "order_hash": { "$ref": "#/components/schemas/Bytes32" },
          "owner": { "$ref": "#/components/schemas/Address" },
          "subkey": { "$ref": "#/components/schemas/Bytes32" },
          "series_id": { "type": ["string", "null"] },
          "side": { "type": "integer" },
          "time_in_force": { "type": "integer" },
          "total_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "filled_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "remaining_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "cancelled": { "type": "boolean" },
          "terminal": { "type": "boolean" }
        }
      },
      "MatchedExecutionRow": {
        "type": "object",
        "properties": {
          "buyer_order_hash": { "$ref": "#/components/schemas/Bytes32" },
          "seller_order_hash": { "$ref": "#/components/schemas/Bytes32" },
          "buyer_subkey": { "$ref": "#/components/schemas/Bytes32" },
          "seller_subkey": { "$ref": "#/components/schemas/Bytes32" },
          "series_id": { "$ref": "#/components/schemas/Bytes32" },
          "matched_qty_1e8": { "$ref": "#/components/schemas/DecimalUint" },
          "premium_amount": { "$ref": "#/components/schemas/DecimalUint" },
          "fee_amount": { "$ref": "#/components/schemas/DecimalUint" },
          "rebate_amount": { "$ref": "#/components/schemas/DecimalUint" },
          "completion_status": { "type": "string", "enum": ["INCOMPLETE", "COMPLETE", "INVALIDATED_BY_REORG"] }
        }
      },
      "FeeEventRow": {
        "type": "object",
        "properties": {
          "kind": { "type": "string" },
          "payer_subkey": { "type": ["string", "null"] },
          "receiver_subkey": { "type": ["string", "null"] },
          "token": { "$ref": "#/components/schemas/Address" },
          "amount": { "$ref": "#/components/schemas/DecimalUint" },
          "execution_id": { "type": ["string", "null"] }
        }
      },
      "HistoryEvent": {
        "type": "object",
        "required": ["event_id", "deployment_id", "chain_id", "block_number", "block_hash", "tx_hash", "log_index", "payload"],
        "properties": {
          "event_id": { "type": "string" },
          "deployment_id": { "type": "integer" },
          "chain_id": { "type": "integer" },
          "block_number": { "type": "integer" },
          "block_hash": { "type": "string" },
          "tx_hash": { "type": "string" },
          "log_index": { "type": "integer" },
          "timestamp_ms": { "type": "integer" },
          "finalized": { "type": "boolean" },
          "direction": { "type": "string", "enum": ["INBOUND", "OUTBOUND", "INTERNAL", "METADATA"] },
          "owner": { "type": ["string", "null"] },
          "subaccount_id": { "type": ["integer", "null"] },
          "subkey": { "type": ["string", "null"] },
          "related_order_hash": { "type": ["string", "null"] },
          "related_execution_id": { "type": ["string", "null"] },
          "payload": { "type": "object", "description": "Tagged union keyed by `family`" }
        }
      }
    },
    "parameters": {
      "DeploymentId": { "in": "query", "name": "deployment_id", "schema": { "type": "integer" } },
      "Consistency": { "in": "query", "name": "consistency", "schema": { "$ref": "#/components/schemas/ConsistencyMode" } },
      "Cursor": { "in": "query", "name": "cursor", "schema": { "type": "string" }, "description": "Opaque base64url cursor" },
      "Limit": { "in": "query", "name": "limit", "schema": { "type": "integer", "minimum": 1, "maximum": 1000 } }
    },
    "responses": {
      "ApiError": {
        "description": "Structured error",
        "content": {
          "application/json": {
            "schema": { "$ref": "#/components/schemas/ApiErrorBody" }
          }
        }
      }
    }
  },
  "paths": {
    "/subaccounts/deployments": {
      "get": {
        "summary": "List configured Hybrid V2 deployments",
        "responses": {
          "200": { "description": "OK",
            "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/DeploymentListRow" } } } }
          }
        }
      }
    },
    "/subaccounts/deployments/{deployment_id}/status": {
      "get": {
        "summary": "Deployment + indexer status. ALWAYS readable, even when not ready.",
        "parameters": [
          { "in": "path", "name": "deployment_id", "required": true, "schema": { "type": "integer" } }
        ],
        "responses": {
          "200": { "description": "OK",
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeploymentStatus" } } }
          },
          "404": { "$ref": "#/components/responses/ApiError" }
        }
      }
    },
    "/accounts/{owner}/hybrid-v2/subaccounts": {
      "get": {
        "summary": "List Hybrid V2 subaccounts owned by an address",
        "parameters": [
          { "in": "path", "name": "owner", "required": true, "schema": { "$ref": "#/components/schemas/Address" } },
          { "$ref": "#/components/parameters/DeploymentId" },
          { "$ref": "#/components/parameters/Consistency" }
        ],
        "responses": {
          "200": { "description": "OK",
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/OwnerSubaccountsResponse" } } }
          },
          "400": { "$ref": "#/components/responses/ApiError" },
          "503": { "$ref": "#/components/responses/ApiError" }
        }
      }
    },
    "/subaccounts/{subkey}": { "get": { "summary": "Subaccount summary", "responses": { "200": { "description": "OK" }, "404": { "$ref": "#/components/responses/ApiError" } } } },
    "/subaccounts/{subkey}/collateral": { "get": { "summary": "Collateral balances", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/reservations": { "get": { "summary": "Reservations per engine", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/positions": { "get": { "summary": "Active Options positions", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/orders": { "get": { "summary": "Order lifecycle rows", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/executions": { "get": { "summary": "Complete matched executions (INCOMPLETE hidden)", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/fees": { "get": { "summary": "Fee / rebate / premium journal", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/recovery": { "get": { "summary": "Recovery + escape state", "responses": { "200": { "description": "OK" } } } },
    "/subaccounts/{subkey}/history": { "get": { "summary": "Subaccount-scoped normalized history", "responses": { "200": { "description": "OK" } } } },
    "/accounts/{owner}/hybrid-v2/history": { "get": { "summary": "Owner-scoped normalized history", "responses": { "200": { "description": "OK" } } } },
    "/hybrid-v2/orders/{order_hash}": { "get": { "summary": "Single order lifecycle row", "responses": { "200": { "description": "OK" }, "404": { "$ref": "#/components/responses/ApiError" } } } },
    "/hybrid-v2/history": { "get": { "summary": "Normalized tagged history feed", "responses": { "200": { "description": "OK" } } } }
  }
}"##;

pub fn openapi_spec_json() -> String {
    OPENAPI_JSON.to_string()
}
