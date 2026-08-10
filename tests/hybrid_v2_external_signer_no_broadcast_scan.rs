//! `BACKEND-HYBRID-V2-EXTERNAL-SIGNER-INTEGRATION-AND-LIVE-ORCHESTRATOR-V1`
//! Part M — Zero-broadcast reaffirmation across the external-signer
//! surface added by Package A.
//!
//! Frozen safety:
//!
//! * **BROADCAST_STRICTLY_FORBIDDEN** — the KMS bridge, the signer
//!   builder, the extended config section, and the admin route
//!   (`prepare_execution`) MUST NOT contain any broadcast-verb token.
//! * **HttpExecutionRpcClient allowlist frozen** — the seven read
//!   methods approved by Package A of
//!   `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1` remain the only
//!   allowed values, and none of them contain a send/broadcast verb.
//! * **`mock.prohibited_calls_seen()` empty** — the mock RPC harness
//!   asserts no broadcast method was ever attempted; here we assert
//!   the harness itself has the correct classification for the two
//!   forbidden methods so a future refactor of the harness that
//!   silently drops one of them fails this test.
//!
//! Verdicts:
//!   * `EXTERNAL_SIGNER_DOES_NOT_BROADCAST`
//!   * `BROADCAST_TECHNICALLY_DISABLED` (reaffirmed)
//!   * `NO_PUBLIC_SIGNING_BROADCAST_OR_CHAIN_WRITE_ACTION` (reaffirmed)

mod hybrid_v2_mock_rpc_helpers;

use std::fs;
use std::path::{Path, PathBuf};

use hybrid_v2_mock_rpc_helpers::MockRpcServer;

/// Tokens that MUST NOT appear anywhere in the new external-signer
/// integration surface added by Package A. The upstream firewall in
/// `hybrid_v2_execution_zero_broadcast_scan.rs` covers the execution
/// module; this scan explicitly names the new files so a token that
/// was somehow snuck into a file OUTSIDE the execution module (for
/// example the extended config section) still fails loudly.
const FORBIDDEN_TOKENS: &[&str] = &[
    "eth_sendTransaction",
    "eth_sendRawTransaction",
    "send_raw_transaction",
    "sendRawTransaction",
    "sendTransaction",
    "personal_sendTransaction",
];

/// Files under scan. These are the concrete files Package A added or
/// extended for the external-signer integration surface. If a source
/// file listed here disappears, the test fails loudly rather than
/// silently passing on a scan of nothing.
const FILES_UNDER_SCAN: &[&str] = &[
    "src/hybrid_v2/execution/signer_kms_bridge.rs",
    "src/hybrid_v2/execution/signer_builder.rs",
    "src/hybrid_v2/config.rs",
    "src/api/hybrid_v2_execution_admin.rs",
    "src/hybrid_v2/execution/signer.rs",
    "src/hybrid_v2/execution/orchestrator.rs",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_file(rel: &str) -> String {
    let path = crate_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "expected {} to be present under {}: {e}",
            rel,
            crate_root().display()
        )
    })
}

#[test]
fn every_external_signer_source_file_is_free_of_broadcast_verbs() {
    // Guard: every file listed above is expected to exist. A missing
    // file indicates a scope regression — this test refuses to pass
    // silently.
    for rel in FILES_UNDER_SCAN {
        let path = Path::new(rel);
        assert!(
            crate_root().join(path).exists(),
            "expected {rel} to exist relative to CARGO_MANIFEST_DIR"
        );
    }
    for rel in FILES_UNDER_SCAN {
        let content = read_file(rel);
        for tok in FORBIDDEN_TOKENS {
            assert!(
                !content.contains(tok),
                "forbidden broadcast token `{tok}` found in {rel} — the external-signer \
                 integration MUST NOT reference any send/broadcast method"
            );
        }
    }
}

#[test]
fn http_execution_rpc_client_allowlist_is_frozen_at_seven_read_methods() {
    use deopt_v2_backend::hybrid_v2::execution::rpc::ALLOWED_METHODS;
    // Package A introduced no new RPC read/write methods. The signer
    // integration relies solely on the seven read methods approved by
    // BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1 (chain id, block
    // number, block by number, tx count, call, estimate gas, fee
    // history). If a future refactor extends this list, this test
    // fails and forces the operator to audit the addition.
    assert_eq!(
        ALLOWED_METHODS.len(),
        7,
        "ExecutionRpcClient allowlist size regression: {:?}",
        ALLOWED_METHODS
    );
    for m in ALLOWED_METHODS {
        for tok in FORBIDDEN_TOKENS {
            assert!(
                !m.contains(tok),
                "allowed method {m} must not contain forbidden token {tok}"
            );
        }
    }
    // The seven methods must be the exact set — order-insensitive.
    let mut sorted: Vec<&str> = ALLOWED_METHODS.iter().copied().collect();
    sorted.sort();
    let expected: Vec<&str> = vec![
        "eth_blockNumber",
        "eth_call",
        "eth_chainId",
        "eth_estimateGas",
        "eth_feeHistory",
        "eth_getBlockByNumber",
        "eth_getTransactionCount",
    ];
    assert_eq!(sorted, expected, "unexpected ExecutionRpcClient allowlist");
}

#[tokio::test]
async fn mock_rpc_harness_still_classifies_the_two_forbidden_methods_as_prohibited() {
    // We attach an outbound `eth_sendTransaction` and
    // `eth_sendRawTransaction` request to the harness manually via its
    // `handle_send_rpc` path, then observe `prohibited_calls_seen()`
    // reports both. The harness classification is what every
    // integration test relies on; a silent removal of one of the two
    // classifications would render every "no broadcast" assertion in
    // the suite vacuous.
    let mock = MockRpcServer::start().await;
    let client = reqwest::Client::new();
    for method in ["eth_sendTransaction", "eth_sendRawTransaction"] {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": [],
        });
        // Best-effort — the harness returns an error for these
        // methods; we do not care about the response body, only that
        // the method reaches the classifier.
        let _ = client
            .post(mock.url())
            .json(&body)
            .send()
            .await
            .expect("send request to mock harness");
    }
    let seen = mock.prohibited_calls_seen();
    assert!(
        seen.contains("eth_sendTransaction"),
        "harness lost eth_sendTransaction classification: {seen:?}"
    );
    assert!(
        seen.contains("eth_sendRawTransaction"),
        "harness lost eth_sendRawTransaction classification: {seen:?}"
    );
}

#[test]
fn kms_bridge_signed_tx_output_has_no_raw_transaction_field() {
    // Structural regression: reading the `SignedTx` field names via
    // Debug should reveal only decomposed signature components, never
    // a raw rlp / broadcast payload. The primary check lives in the
    // signer module unit tests; this one guards the external-signer
    // integration surface specifically.
    use deopt_v2_backend::hybrid_v2::execution::signer::SignedTx;
    let tx = SignedTx {
        signature_r: [0u8; 32],
        signature_s: [0u8; 32],
        signature_v: 0,
        recovered_signer: [0u8; 20],
        tx_type: 2,
    };
    let s = format!("{tx:?}").to_ascii_lowercase();
    for tok in ["raw", "rlp", "broadcast", "sendtx"] {
        assert!(!s.contains(tok), "SignedTx debug leaks `{tok}`: {s}");
    }
}
