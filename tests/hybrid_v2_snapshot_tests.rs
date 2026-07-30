//! BACKEND WP-01 snapshot drift detector.
//!
//! Verifies the pinned Solidity JSON snapshots (`resources/hybrid-v2/*`)
//! are byte-for-byte identical to what `source-metadata.json` records.
//! Any drift fails this test — the operator MUST refresh both the JSON
//! and the metadata together after a Solidity release.

use deopt_v2_backend::hybrid_v2::{PinnedSnapshots, SourceMetadata};

#[test]
fn source_metadata_parses() {
    let m: SourceMetadata = PinnedSnapshots::get()
        .parse_source_metadata()
        .expect("source-metadata.json must parse");
    assert_eq!(m.solidity.commit, "f080272");
    assert_eq!(m.solidity.repository, "deopt-v2-sol");
    assert_eq!(m.hybrid_v2.architecture_version, 1);
    assert_eq!(m.hybrid_v2.storage_version, 1);
    assert_eq!(m.hybrid_v2.event_version, 1);
    assert_eq!(m.hybrid_v2.manifest_schema_version, 1);
}

#[test]
fn every_pinned_snapshot_matches_recorded_sha256() {
    let snapshots = PinnedSnapshots::get();
    let metadata = snapshots
        .parse_source_metadata()
        .expect("source-metadata.json must parse");

    let computed = snapshots.compute_snapshot_hashes();
    for (file, expected) in &metadata.snapshots {
        let actual = computed
            .get(file.as_str())
            .unwrap_or_else(|| panic!("no computed hash for {}", file));
        assert_eq!(
            actual, expected,
            "SHA-256 drift on {}: pinned={}, computed={}",
            file, expected, actual
        );
    }
    // Also verify every computed key is recorded in the metadata (no
    // orphan snapshot file).
    for file in computed.keys() {
        assert!(
            metadata.snapshots.contains_key(*file),
            "computed hash for {} but no entry in source-metadata.json",
            file
        );
    }
}

#[test]
fn topic_catalogue_loads_every_pinned_event() {
    let catalogue = deopt_v2_backend::hybrid_v2::TopicCatalogue::get();
    assert_eq!(catalogue.schema_version, 1);
    assert!(
        catalogue.len() >= 40,
        "expected at least 40 canonical Hybrid V2 events, got {}",
        catalogue.len()
    );
    // Spot-check a few well-known signatures.
    let dep = catalogue
        .lookup_by_event("Deposit")
        .expect("Deposit topic missing");
    assert_eq!(
        dep.topic0_hex_lower,
        "0x774b18d5c0a5d41384d85f57a6cb80146a2d07b438593f2a31d5984d31ca2588"
    );
    let fin = catalogue
        .lookup_by_event("RecoveryFinalized")
        .expect("RecoveryFinalized topic missing");
    assert_eq!(
        fin.topic0_hex_lower,
        "0x5d8bf05ccd48556b2a1560dfb9a6b16238cd8b21e91ac6120f6a59de06a1e44a"
    );
}
