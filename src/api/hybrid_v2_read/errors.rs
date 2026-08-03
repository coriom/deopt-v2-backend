//! Structured error responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    InvalidAddress,
    InvalidSubKey,
    InvalidDeployment,
    DeploymentNotFound,
    SubaccountNotFound,
    Account0Invalid,
    InvalidCursor,
    StaleCursor,
    InvalidFilterCombination,
    PageLimitExceeded,
    IndexerNotReady,
    ManifestMismatch,
    ReconciliationDrift,
    UnsupportedConsistency,
    MalformedCanonicalData,
    InternalInconsistency,
    OrderNotFound,
    UnknownRoute,
}

impl ApiErrorCode {
    pub fn http_status(&self) -> StatusCode {
        use ApiErrorCode::*;
        match self {
            InvalidAddress
            | InvalidSubKey
            | InvalidDeployment
            | InvalidCursor
            | InvalidFilterCombination
            | UnsupportedConsistency
            | Account0Invalid => StatusCode::BAD_REQUEST,
            PageLimitExceeded => StatusCode::BAD_REQUEST,
            DeploymentNotFound | SubaccountNotFound | OrderNotFound | UnknownRoute => {
                StatusCode::NOT_FOUND
            }
            IndexerNotReady | ManifestMismatch | ReconciliationDrift => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            StaleCursor => StatusCode::CONFLICT,
            MalformedCanonicalData | InternalInconsistency => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    pub fn as_str(&self) -> &'static str {
        use ApiErrorCode::*;
        match self {
            InvalidAddress => "INVALID_ADDRESS",
            InvalidSubKey => "INVALID_SUBKEY",
            InvalidDeployment => "INVALID_DEPLOYMENT",
            DeploymentNotFound => "DEPLOYMENT_NOT_FOUND",
            SubaccountNotFound => "SUBACCOUNT_NOT_FOUND",
            Account0Invalid => "ACCOUNT_0_INVALID",
            InvalidCursor => "INVALID_CURSOR",
            StaleCursor => "STALE_CURSOR",
            InvalidFilterCombination => "INVALID_FILTER_COMBINATION",
            PageLimitExceeded => "PAGE_LIMIT_EXCEEDED",
            IndexerNotReady => "INDEXER_NOT_READY",
            ManifestMismatch => "MANIFEST_MISMATCH",
            ReconciliationDrift => "RECONCILIATION_DRIFT",
            UnsupportedConsistency => "UNSUPPORTED_CONSISTENCY",
            MalformedCanonicalData => "MALFORMED_CANONICAL_DATA",
            InternalInconsistency => "INTERNAL_INCONSISTENCY",
            OrderNotFound => "ORDER_NOT_FOUND",
            UnknownRoute => "UNKNOWN_ROUTE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub retryable: bool,
    pub detail: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: matches!(
                code,
                ApiErrorCode::IndexerNotReady
                    | ApiErrorCode::ManifestMismatch
                    | ApiErrorCode::ReconciliationDrift
                    | ApiErrorCode::StaleCursor
            ),
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }
    pub fn body(&self) -> ApiErrorBody {
        ApiErrorBody {
            code: self.code.as_str(),
            message: self.message.clone(),
            retryable: self.retryable,
            detail: self.detail.clone(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.code.http_status();
        (status, Json(self.body())).into_response()
    }
}

// `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-2B-HANDLER-SWAP-V1` — fail-closed
// mapping of the store-boundary error taxonomy into the HTTP API error
// model. Every branch preserves the existing structured-body contract and
// never leaks raw SQL / credentials / URLs.
impl From<crate::hybrid_v2::read_store::ReadStoreError> for ApiError {
    fn from(err: crate::hybrid_v2::read_store::ReadStoreError) -> Self {
        use crate::hybrid_v2::read_store::ReadStoreError;
        match err {
            ReadStoreError::Backend { detail } => ApiError::new(
                ApiErrorCode::InternalInconsistency,
                format!("read store backend unavailable: {}", detail),
            ),
            ReadStoreError::MalformedRow { detail } => ApiError::new(
                ApiErrorCode::MalformedCanonicalData,
                format!("persisted canonical row malformed: {}", detail),
            ),
            ReadStoreError::InvalidCursor { detail } => ApiError::new(
                ApiErrorCode::InvalidCursor,
                format!("cursor rejected by store: {}", detail),
            ),
            ReadStoreError::StaleCursor {
                expected_hash,
                actual_hash,
            } => ApiError::new(
                ApiErrorCode::StaleCursor,
                format!(
                    "cursor bound to orphaned indexed head (expected {}, got {}); restart pagination",
                    expected_hash, actual_hash
                ),
            ),
            ReadStoreError::LimitExceeded { max, requested } => ApiError::new(
                ApiErrorCode::PageLimitExceeded,
                format!("limit {} exceeds max {}", requested, max),
            ),
            ReadStoreError::InvalidFilter { detail } => ApiError::new(
                ApiErrorCode::InvalidFilterCombination,
                format!("filter rejected by store: {}", detail),
            ),
        }
    }
}
