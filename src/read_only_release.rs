//! VPS read-only release guard. This is an opt-in, additional fail-closed
//! boundary; it does not change the normal local development configuration.

use crate::error::{BackendError, Result};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

pub fn validate_from_env() -> Result<bool> {
    let mode = std::env::var("DEOPT_READ_ONLY_RELEASE").unwrap_or_default();
    if !mode.is_empty() && !mode.eq_ignore_ascii_case("true") && !mode.eq_ignore_ascii_case("false")
    {
        return Err(BackendError::Config(
            "invalid DEOPT_READ_ONLY_RELEASE value".into(),
        ));
    }
    let enabled = mode.eq_ignore_ascii_case("true");
    if !enabled {
        return Ok(false);
    }
    validate_pairs(std::env::vars())?;
    Ok(true)
}

fn validate_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Result<()> {
    let vars: std::collections::HashMap<String, String> = pairs.into_iter().collect();
    if vars.get("CHAIN_ID").map(String::as_str) != Some("84532") {
        return Err(BackendError::Config(
            "read-only release requires explicit CHAIN_ID=84532".into(),
        ));
    }
    if vars.get("PERSISTENCE_ENABLED").map(String::as_str) != Some("true") {
        return Err(BackendError::Config(
            "read-only release requires PERSISTENCE_ENABLED=true".into(),
        ));
    }
    for (name, value) in &vars {
        let upper = name.to_ascii_uppercase();
        if upper.ends_with("_CHAIN_ID") && value != "84532" {
            return Err(BackendError::Config(format!(
                "read-only release refuses non-Base-Sepolia chain variable {name}"
            )));
        }
        if upper.contains("PRIVATE_KEY")
            || upper.contains("SIGNER_KEY")
            || upper.contains("MNEMONIC")
            || upper.contains("KEYSTORE")
        {
            return Err(BackendError::Config(format!(
                "read-only release refuses signing material variable {name}"
            )));
        }
        if upper.ends_with("_ENABLED")
            && !matches!(upper.as_str(), "PERSISTENCE_ENABLED" | "PERPS_READ_ENABLED")
            && value.eq_ignore_ascii_case("true")
        {
            return Err(BackendError::Config(format!(
                "read-only release refuses enabled capability {name}"
            )));
        }
    }
    Ok(())
}

pub async fn reject_http_writes(request: Request, next: Next) -> Response {
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return (StatusCode::METHOD_NOT_ALLOWED, "read-only release").into_response();
    }
    // Wallet history is not authenticated by the legacy route. CORS is no
    // access control, so expose only the bounded market/status surface here.
    let path = request.uri().path();
    if !matches!(path, "/health" | "/ready" | "/perps/markets")
        && !path.starts_with("/perps/markets/")
    {
        return (StatusCode::FORBIDDEN, "read-only release route unavailable").into_response();
    }
    next.run(request).await
}

use axum::response::IntoResponse;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    fn base() -> Vec<(String, String)> {
        vec![
            ("CHAIN_ID".into(), "84532".into()),
            ("PERSISTENCE_ENABLED".into(), "true".into()),
            ("PERPS_READ_ENABLED".into(), "true".into()),
        ]
    }

    #[test]
    fn refuses_wrong_chain_writes_and_signing_material() {
        assert!(validate_pairs(base()).is_ok());
        for (key, value) in [
            ("CHAIN_ID", "8453"),
            ("EXECUTION_ENABLED", "true"),
            ("HV2_BROADCAST_ENABLED", "true"),
            ("INDEXER_ENABLED", "true"),
            ("EXECUTOR_PRIVATE_KEY", "dummy"),
            ("PERPS_CHAIN_ID", "8453"),
        ] {
            let mut vars = base();
            vars.push((key.into(), value.into()));
            assert!(validate_pairs(vars).is_err(), "{key}");
        }
    }

    #[tokio::test]
    async fn rejects_every_http_mutation_method() {
        let app = Router::new()
            .route("/health", get(|| async { "ok" }).post(|| async { "bad" }))
            .route(
                "/accounts/:wallet/perps/canonical-fees",
                get(|| async { "private" }),
            )
            .layer(axum::middleware::from_fn(reject_http_writes));
        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let history = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/accounts/0x123/perps/canonical-fees")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(history.status(), StatusCode::FORBIDDEN);
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/health")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        }
    }
}
