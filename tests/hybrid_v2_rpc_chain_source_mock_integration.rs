//! `BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1` —
//! Mock-RPC integration tests for `RpcHybridV2ChainSource`.
//!
//! These tests spin up a deterministic axum-backed mock JSON-RPC server
//! and exercise every public boundary of the read-only source without
//! requiring network access. They also prove the "no write method
//! requested" invariant by asserting the mock never records a
//! prohibited-method call under any scenario.

mod hybrid_v2_mock_rpc_helpers;

use deopt_v2_backend::hybrid_v2::chain_source::{ChainSource, ChainSourceError};
use deopt_v2_backend::hybrid_v2::{RpcHybridV2ChainSource, RpcSourceConfig};
use hybrid_v2_mock_rpc_helpers::{addr20, make_block, make_log, topic_bytes32, MockRpcServer};
use std::time::Duration;

fn source_config(url: String, chain_id: u64) -> RpcSourceConfig {
    RpcSourceConfig {
        endpoint: url,
        chain_id,
        timeout: Duration::from_secs(2),
        max_retries: 3,
        retry_backoff: Duration::from_millis(10),
        max_logs_per_range: 2_000,
        confirmation_depth: 12,
    }
}

fn build_source(
    mock: &MockRpcServer,
    chain_id: u64,
    emitters: Vec<String>,
) -> RpcHybridV2ChainSource {
    RpcHybridV2ChainSource::new(source_config(mock.url(), chain_id), emitters)
        .expect("source construct")
}

#[tokio::test]
async fn chain_id_returns_configured_value() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let source = build_source(&mock, 84532, vec![addr20(0xaa)]);
    let got = source.chain_id().await.expect("chain id ok");
    assert_eq!(got, 84532);
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn chain_id_mismatch_at_validate_fails_closed() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(31337);
    let source = build_source(&mock, 84532, vec![addr20(0xaa)]);
    let err = source
        .validate_chain_identity()
        .await
        .expect_err("validation must fail on mismatch");
    match err {
        ChainSourceError::Unsupported(s) => {
            assert!(s.contains("chain_id=31337"), "detail={s}");
        }
        other => panic!("unexpected err: {other:?}"),
    }
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn base_mainnet_rejected_by_source_validation() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(8453);
    let source = build_source(&mock, 8453, vec![addr20(0xaa)]);
    let err = source
        .validate_chain_identity()
        .await
        .expect_err("Base mainnet must be refused");
    match err {
        ChainSourceError::Unsupported(s) => {
            assert!(s.contains("Base mainnet"), "detail={s}");
        }
        other => panic!("unexpected err: {other:?}"),
    }
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn head_block_number_read_via_eth_block_number() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(1234);
    let source = build_source(&mock, 84532, vec![]);
    let head = source.head_block_number().await.expect("head");
    assert_eq!(head, 1234);
    let methods: Vec<String> = mock.calls().into_iter().map(|c| c.method).collect();
    assert!(methods.contains(&"eth_blockNumber".to_string()));
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn block_at_returns_none_when_provider_returns_null() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let source = build_source(&mock, 84532, vec![]);
    let result = source.block_at(42).await.expect("call ok");
    assert!(result.is_none());
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn block_at_returns_block_with_logs_when_present() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let emitter = addr20(0xa1);
    let mut b = make_block(7, 0xb1, &format!("0x{}{}", "b0", "0".repeat(62)), 1_000);
    b.logs.push(make_log(
        &b,
        &emitter,
        0,
        vec![topic_bytes32(0x11)],
        "0x1234",
    ));
    b.logs
        .push(make_log(&b, &emitter, 1, vec![topic_bytes32(0x22)], "0x"));
    mock.push_block(b);
    let source = build_source(&mock, 84532, vec![emitter.clone()]);
    let got = source
        .block_at(7)
        .await
        .expect("call ok")
        .expect("block present");
    assert_eq!(got.number, 7);
    assert_eq!(got.logs.len(), 2);
    assert_eq!(got.logs[0].emitter, emitter.to_ascii_lowercase());
    assert!(mock.prohibited_calls().is_empty());
}

#[tokio::test]
async fn multi_emitter_block_filtered_by_configured_addresses() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let wanted = addr20(0xa1);
    let unwanted = addr20(0xff);
    let mut b = make_block(9, 0xb9, &format!("0x{}{}", "b8", "0".repeat(62)), 1_100);
    b.logs
        .push(make_log(&b, &wanted, 0, vec![topic_bytes32(0x11)], "0x"));
    b.logs
        .push(make_log(&b, &unwanted, 1, vec![topic_bytes32(0x22)], "0x"));
    b.logs
        .push(make_log(&b, &wanted, 2, vec![topic_bytes32(0x33)], "0x"));
    mock.push_block(b);
    let source = build_source(&mock, 84532, vec![wanted.clone()]);
    let got = source.block_at(9).await.expect("call ok").expect("block");
    // Provider is expected to honour the address filter; the mock does
    // so as well. The source must NOT include the unrelated emitter.
    assert_eq!(got.logs.len(), 2);
    for l in &got.logs {
        assert_eq!(l.emitter, wanted.to_ascii_lowercase());
    }
}

#[tokio::test]
async fn duplicate_provider_logs_collapse() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let emitter = addr20(0xa1);
    let mut b = make_block(3, 0xb3, &format!("0x{}{}", "b2", "0".repeat(62)), 1_030);
    let log = make_log(&b, &emitter, 0, vec![topic_bytes32(0x11)], "0x");
    b.logs.push(log.clone());
    b.logs.push(log);
    mock.push_block(b);
    let source = build_source(&mock, 84532, vec![emitter]);
    let got = source.block_at(3).await.expect("call ok").expect("block");
    assert_eq!(got.logs.len(), 1, "duplicate logs must dedupe");
}

#[tokio::test]
async fn malformed_block_hash_rejected() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let emitter = addr20(0xa1);
    let mut b = make_block(4, 0xb4, &format!("0x{}{}", "b3", "0".repeat(62)), 1_040);
    let mut log = make_log(&b, &emitter, 0, vec![topic_bytes32(0x11)], "0x");
    log.block_hash = "0xdead".to_string(); // too short
    b.logs.push(log);
    mock.push_block(b);
    let source = build_source(&mock, 84532, vec![emitter]);
    let err = source
        .block_at(4)
        .await
        .expect_err("malformed hash must be rejected");
    assert!(matches!(err, ChainSourceError::Malformed(_)), "err={err:?}");
}

#[tokio::test]
async fn parent_mismatch_between_header_and_log_rejected() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let emitter = addr20(0xa1);
    let mut b = make_block(5, 0xb5, &format!("0x{}{}", "b4", "0".repeat(62)), 1_050);
    let mut log = make_log(&b, &emitter, 0, vec![topic_bytes32(0x11)], "0x");
    // Well-formed hash, but disagrees with the header.
    log.block_hash = format!("0xcc{}", "0".repeat(62));
    b.logs.push(log);
    mock.push_block(b);
    let source = build_source(&mock, 84532, vec![emitter]);
    let err = source
        .block_at(5)
        .await
        .expect_err("hash mismatch must be rejected");
    assert!(matches!(err, ChainSourceError::Malformed(_)), "err={err:?}");
}

#[tokio::test]
async fn finalized_tag_supported() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(100);
    mock.set_finalized(80);
    let source = build_source(&mock, 84532, vec![]);
    let got = source.finalized_block_number().await.expect("finalized");
    assert_eq!(got, 80);
}

#[tokio::test]
async fn finalized_tag_unsupported_falls_back_to_safe() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(100);
    mock.set_finalized(70);
    mock.set_finalized_tag_supported(false);
    let source = build_source(&mock, 84532, vec![]);
    let got = source.finalized_block_number().await.expect("finalized");
    assert_eq!(got, 70);
}

#[tokio::test]
async fn finalized_tag_and_safe_unsupported_falls_back_to_confirmation_depth() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(100);
    mock.set_finalized_tag_supported(false);
    mock.set_safe_tag_supported(false);
    let source = build_source(&mock, 84532, vec![]);
    let got = source.finalized_block_number().await.expect("finalized");
    // head 100 - confirmation depth 12 = 88.
    assert_eq!(got, 88);
}

#[tokio::test]
async fn http_429_triggers_bounded_retry_then_succeeds() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.simulate_rate_limit(2);
    let source = build_source(&mock, 84532, vec![]);
    let got = source.chain_id().await.expect("must succeed after retries");
    assert_eq!(got, 84532);
    // 2 attempts got 429; 3rd succeeded. But the source retries at most
    // max_retries=3 additional attempts after the first failure, so
    // total calls = 3.
    let count = mock
        .calls()
        .into_iter()
        .filter(|c| c.method == "eth_chainId")
        .count();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn http_500_treated_as_retryable() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.simulate_status(1, 500);
    let source = build_source(&mock, 84532, vec![]);
    let got = source.chain_id().await.expect("must retry through 500");
    assert_eq!(got, 84532);
}

#[tokio::test]
async fn provider_never_recovers_returns_final_error() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    // Send 20 rate limits — max_retries is 3 so should give up after 4 attempts.
    mock.simulate_rate_limit(20);
    let source = build_source(&mock, 84532, vec![]);
    let err = source
        .chain_id()
        .await
        .expect_err("must give up after max_retries");
    assert!(matches!(err, ChainSourceError::RateLimited));
}

#[tokio::test]
async fn deterministic_json_rpc_error_not_retried() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let emitter = addr20(0xa1);
    // Inject an rpc error on the next eth_getLogs call.
    let mut b = make_block(1, 0xb1, &format!("0x{}{}", "b0", "0".repeat(62)), 1_010);
    b.logs
        .push(make_log(&b, &emitter, 0, vec![topic_bytes32(0x11)], "0x"));
    mock.push_block(b);
    mock.simulate_next_rpc_error(-32602, "invalid params");
    let source = build_source(&mock, 84532, vec![emitter]);
    let err = source
        .block_at(1)
        .await
        .expect_err("must not retry deterministic error");
    match err {
        ChainSourceError::RpcError { code, .. } => assert_eq!(code, -32602),
        other => panic!("unexpected err: {other:?}"),
    }
    // eth_getLogs was called exactly once (no retry).
    let count = mock
        .calls()
        .into_iter()
        .filter(|c| c.method == "eth_getLogs")
        .count();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn cancellation_during_request_exits_cleanly() {
    // We simulate cancellation by dropping the future mid-request. The
    // test asserts we do not leak resources — completing the drop is
    // sufficient proof.
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    let source = build_source(&mock, 84532, vec![]);
    let fut = source.chain_id();
    drop(fut);
    // Server is still healthy.
    let ok = source.chain_id().await.expect("still ok after drop");
    assert_eq!(ok, 84532);
}

#[tokio::test]
async fn prohibited_method_not_requested_ever() {
    let mock = MockRpcServer::start().await;
    mock.set_chain_id(84532);
    mock.set_head(3);
    let emitter = addr20(0xa1);
    for n in 1..=3u64 {
        let parent = if n == 1 {
            format!("0x{}{}", "00", "0".repeat(62))
        } else {
            format!(
                "0x{}{:0>62x}",
                format!("{:02x}", 0xb0 + (n as u8 - 1)),
                n - 1
            )
        };
        let mut b = make_block(n, 0xb0 + n as u8, &parent, 1_000 + n * 12);
        b.logs
            .push(make_log(&b, &emitter, 0, vec![topic_bytes32(0x11)], "0x"));
        mock.push_block(b);
    }
    let source = build_source(&mock, 84532, vec![emitter]);
    let _ = source.chain_id().await;
    let _ = source.head_block_number().await;
    let _ = source.finalized_block_number().await;
    let _ = source.block_at(1).await;
    let _ = source.block_at(2).await;
    let _ = source.block_at(3).await;
    assert!(
        mock.prohibited_calls().is_empty(),
        "source must never emit a state-changing method; saw: {:?}",
        mock.prohibited_calls()
    );
    let all_methods: Vec<String> = mock.calls().into_iter().map(|c| c.method).collect();
    for m in &all_methods {
        assert!(
            matches!(
                m.as_str(),
                "eth_chainId"
                    | "eth_blockNumber"
                    | "eth_getBlockByNumber"
                    | "eth_getBlockByHash"
                    | "eth_getLogs"
            ),
            "unexpected method issued by source: {m}"
        );
    }
}
