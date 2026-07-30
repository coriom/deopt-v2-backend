//! Pinned Solidity manifest / topic / error / abi snapshots.
//!
//! The five JSON files under `resources/hybrid-v2/` are byte-for-byte
//! copies of the tracked files under `deployment-manifest/` in the
//! Solidity repository. `SourceMetadata` records the Solidity source
//! commit and the expected SHA-256 of every snapshot; the
//! `verify_pinned_snapshots` test in `tests/hybrid_v2_snapshot_tests.rs`
//! fails on any drift.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Byte content of every pinned snapshot, compiled into the binary via
/// `include_str!`. Reading these fields is infallible — the compilation
/// itself is the freshness proof.
pub struct PinnedSnapshots {
    pub manifest_schema_v1: &'static str,
    pub event_topics_v1: &'static str,
    pub error_selectors_v1: &'static str,
    pub abi_surface_v1: &'static str,
    pub base_sepolia_template_v1: &'static str,
    pub source_metadata_json: &'static str,
}

impl PinnedSnapshots {
    /// The single canonical instance. All Hybrid V2 code paths reference
    /// this constant — never re-read the JSON files at runtime.
    pub const fn get() -> &'static PinnedSnapshots {
        &BUNDLED
    }
}

/// Every field in `resources/hybrid-v2/source-metadata.json`, decoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub note: String,
    pub solidity: SoliditySource,
    #[serde(rename = "hybridV2")]
    pub hybrid_v2: HybridV2Versions,
    pub snapshots: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoliditySource {
    pub repository: String,
    pub commit: String,
    pub milestone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridV2Versions {
    #[serde(rename = "architectureVersion")]
    pub architecture_version: u16,
    #[serde(rename = "storageVersion")]
    pub storage_version: u16,
    #[serde(rename = "eventVersion")]
    pub event_version: u16,
    #[serde(rename = "manifestSchemaVersion")]
    pub manifest_schema_version: u16,
}

impl PinnedSnapshots {
    pub fn parse_source_metadata(&self) -> Result<SourceMetadata, serde_json::Error> {
        serde_json::from_str(self.source_metadata_json)
    }

    /// Compute the current SHA-256 of every embedded snapshot. Used by
    /// the drift-detection test to compare against `source-metadata.json`.
    pub fn compute_snapshot_hashes(&self) -> BTreeMap<&'static str, String> {
        let mut out = BTreeMap::new();
        out.insert(
            "manifest-schema-v1.json",
            hex_sha256(self.manifest_schema_v1.as_bytes()),
        );
        out.insert(
            "event-topics-v1.json",
            hex_sha256(self.event_topics_v1.as_bytes()),
        );
        out.insert(
            "error-selectors-v1.json",
            hex_sha256(self.error_selectors_v1.as_bytes()),
        );
        out.insert(
            "abi-surface-v1.json",
            hex_sha256(self.abi_surface_v1.as_bytes()),
        );
        out.insert(
            "base-sepolia-template-v1.json",
            hex_sha256(self.base_sepolia_template_v1.as_bytes()),
        );
        out
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut hex = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

static BUNDLED: PinnedSnapshots = PinnedSnapshots {
    manifest_schema_v1: include_str!("../../resources/hybrid-v2/manifest-schema-v1.json"),
    event_topics_v1: include_str!("../../resources/hybrid-v2/event-topics-v1.json"),
    error_selectors_v1: include_str!("../../resources/hybrid-v2/error-selectors-v1.json"),
    abi_surface_v1: include_str!("../../resources/hybrid-v2/abi-surface-v1.json"),
    base_sepolia_template_v1: include_str!(
        "../../resources/hybrid-v2/base-sepolia-template-v1.json"
    ),
    source_metadata_json: include_str!("../../resources/hybrid-v2/source-metadata.json"),
};
