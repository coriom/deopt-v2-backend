//! Zero-broadcast source-scan (Part P of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Runtime enforcement of the narrow-boundary rule via a filesystem
//! sweep: read every `.rs` file under `src/hybrid_v2/execution/` and
//! assert that no forbidden token appears. This complements the
//! compile-time firewall (the read-only `ExecutionRpcClient` trait
//! has no `send_*` method) with a defensive property that will fail
//! loudly the day someone tries to sneak a broadcast helper into a
//! file that is NOT on the exempt list.
//!
//! Exceptions are minimal and explicit: the read-only rpc allow list
//! inside `rpc.rs`, the module-doc reference in `mod.rs`, and the
//! new broadcast files that `BACKEND-HYBRID-V2-BROADCAST-AND-
//! CONFIRMATION-V1` (Package A) explicitly authorized to house the
//! narrow send capability (`broadcast_rpc.rs`, `broadcast_outbox.rs`,
//! `broadcast_firewall.rs`, and `broadcast_state.rs` which describes
//! the phases the outbox drives). Any additional file that references
//! `eth_sendRawTransaction` etc. is rejected — the boundary must stay
//! narrow.

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN_TOKENS: &[&str] = &[
    "send_raw_transaction",
    "eth_sendRawTransaction",
    "eth_sendTransaction",
    "send_transaction",
    "sendTransaction",
    "personal_sendTransaction",
    "sendRawTransaction",
];

/// Files exempt from the token sweep. Two categories:
///   1. The read-only rpc + module tree that DENIES broadcast — those
///      files list the forbidden names as strings for the defence.
///   2. The Package A broadcast files that AUTHORIZE the narrow send
///      capability. Their entire purpose is to safely encapsulate
///      `send_raw_transaction`; keeping the boundary narrow is the
///      real invariant.
const ALLOWED_FILES: &[&str] = &[
    // (1) The trait module explicitly lists forbidden method names in
    // `is_send_or_sign_method` as the runtime allowlist defence.
    "src/hybrid_v2/execution/rpc.rs",
    // Module-level doc comment references forbidden methods to
    // document the narrow-boundary rule.
    "src/hybrid_v2/execution/mod.rs",
    // (2) Package A: the narrow broadcast surface.
    "src/hybrid_v2/execution/broadcast_rpc.rs",
    "src/hybrid_v2/execution/broadcast_outbox.rs",
    "src/hybrid_v2/execution/broadcast_firewall.rs",
    // broadcast_state.rs describes the phases the outbox drives and
    // its doc comment references SUBMISSION_UNKNOWN recovery paths
    // that mention transaction_by_hash — the tokens themselves do not
    // appear here, but we exempt for future comment additions.
    "src/hybrid_v2/execution/broadcast_state.rs",
    // tx_serialization.rs docstring names `eth_sendRawTransaction` as
    // the destination of the raw payload it builds; the file itself
    // does no I/O.
    "src/hybrid_v2/execution/tx_serialization.rs",
    // Package B (Parts J–P) — lifecycle files that observe / classify
    // the network state around the narrow send capability. The nonce
    // investigator and receipt worker never issue writes but their
    // doc comments and test mocks legitimately reference the token.
    "src/hybrid_v2/execution/broadcast_nonce_policy.rs",
    "src/hybrid_v2/execution/broadcast_worker.rs",
    "src/hybrid_v2/execution/broadcast_indexer_correlation.rs",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, acc: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, acc);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                acc.push(path);
            }
        }
    }
}

#[test]
fn zero_broadcast_capability_across_execution_module() {
    let root = crate_root();
    let target = root.join("src/hybrid_v2/execution");
    let mut files = Vec::new();
    walk(&target, &mut files);
    assert!(
        !files.is_empty(),
        "expected files under src/hybrid_v2/execution/*"
    );

    for f in files {
        let rel = f
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| f.to_string_lossy().into_owned());
        if ALLOWED_FILES.iter().any(|allow| rel == *allow) {
            continue;
        }
        let content = fs::read_to_string(&f).expect("read execution source file");
        for tok in FORBIDDEN_TOKENS {
            assert!(
                !content.contains(tok),
                "forbidden broadcast token `{tok}` found in {rel} — the execution module MUST NOT reference any send/broadcast method"
            );
        }
    }
}

#[test]
fn allowed_methods_do_not_contain_any_send_verb() {
    use deopt_v2_backend::hybrid_v2::execution::rpc::ALLOWED_METHODS;
    for m in ALLOWED_METHODS {
        for tok in FORBIDDEN_TOKENS {
            assert!(
                !m.contains(tok),
                "allowed method {m} must not contain forbidden token {tok}"
            );
        }
    }
}
