//! `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` — config bounds tests
//! for `ReorgRecoveryConfig::validate` (fail-closed on bad bounds).

use deopt_v2_backend::hybrid_v2::reorg_recovery::ReorgRecoveryConfig;

#[test]
fn defaults_validate() {
    let cfg = ReorgRecoveryConfig::default();
    cfg.validate().expect("defaults must validate");
    assert_eq!(cfg.max_reorg_depth, 64);
    assert_eq!(cfg.max_replacement_blocks, 256);
    assert_eq!(cfg.ancestor_search_max_headers, 128);
    assert_eq!(cfg.retry_max, 5);
    assert_eq!(cfg.retry_backoff_ms, 500);
    assert!(!cfg.allow_finalized_boundary_crossing);
}

#[test]
fn max_reorg_depth_bounds() {
    let mut cfg = ReorgRecoveryConfig::default();
    cfg.max_reorg_depth = 0;
    assert!(cfg.validate().is_err(), "zero must be rejected");
    cfg.max_reorg_depth = 513;
    assert!(cfg.validate().is_err(), "above 512 must be rejected");
    for v in [1u64, 8, 64, 512] {
        cfg.max_reorg_depth = v;
        cfg.validate().expect("in-range must validate");
    }
}

#[test]
fn max_replacement_blocks_bounds() {
    let mut cfg = ReorgRecoveryConfig::default();
    cfg.max_replacement_blocks = 0;
    assert!(cfg.validate().is_err());
    cfg.max_replacement_blocks = 4097;
    assert!(cfg.validate().is_err());
    for v in [1u32, 32, 256, 4096] {
        cfg.max_replacement_blocks = v;
        cfg.validate().expect("in-range must validate");
    }
}

#[test]
fn ancestor_search_max_headers_bounds() {
    let mut cfg = ReorgRecoveryConfig::default();
    cfg.ancestor_search_max_headers = 0;
    assert!(cfg.validate().is_err());
    cfg.ancestor_search_max_headers = 1025;
    assert!(cfg.validate().is_err());
    for v in [1u32, 32, 128, 1024] {
        cfg.ancestor_search_max_headers = v;
        cfg.validate().expect("in-range must validate");
    }
}

#[test]
fn retry_max_bounds() {
    let mut cfg = ReorgRecoveryConfig::default();
    cfg.retry_max = 21;
    assert!(cfg.validate().is_err(), "above 20 must be rejected");
    for v in [0u32, 1, 5, 20] {
        cfg.retry_max = v;
        cfg.validate().expect("in-range must validate");
    }
}

#[test]
fn retry_backoff_ms_bounds() {
    let mut cfg = ReorgRecoveryConfig::default();
    cfg.retry_backoff_ms = 49;
    assert!(cfg.validate().is_err());
    cfg.retry_backoff_ms = 60_001;
    assert!(cfg.validate().is_err());
    for v in [50u64, 500, 30_000, 60_000] {
        cfg.retry_backoff_ms = v;
        cfg.validate().expect("in-range must validate");
    }
}
