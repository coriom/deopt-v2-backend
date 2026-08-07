//! Zero-broadcast source-scan (Part P of
//! `BACKEND-HYBRID-V2-SIGNER-AND-EXECUTION-V1`).
//!
//! Runtime enforcement of `BROADCAST_IS_DISABLED` via a filesystem
//! sweep: read every `.rs` file under `src/hybrid_v2/execution/` and
//! assert that no forbidden token appears. This complements the
//! compile-time firewall (the `ExecutionRpcClient` trait has no
//! `send_*` method) with a defensive property that will fail loudly
//! the day someone tries to sneak a broadcast helper into the module.
//!
//! Exceptions are minimal and explicit: the test itself, plus the
//! wire-protocol allow-and-deny lists inside `rpc.rs` (which contain
//! the forbidden method names as strings for the `check_method`
//! defence).

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

/// Files exempt from the token sweep — files whose entire purpose is
/// to REFUSE these method names (the enforcement lives inside them).
const ALLOWED_FILES: &[&str] = &[
    // The trait module explicitly lists forbidden method names in
    // `is_send_or_sign_method` as the runtime allowlist defence.
    "src/hybrid_v2/execution/rpc.rs",
    // Module-level doc comment references forbidden methods to
    // document the invariant BROADCAST_IS_DISABLED. The invariant
    // itself is enforced by the trait shape and this scan.
    "src/hybrid_v2/execution/mod.rs",
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
