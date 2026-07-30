//! Canonical event topic catalogue loaded from the pinned Solidity
//! snapshot `resources/hybrid-v2/event-topics-v1.json`.
//!
//! Frozen behavior:
//! - Topic 0 is derived on the Solidity side via `keccak256(signature)`.
//! - The backend NEVER re-derives topic 0 from a signature string; the
//!   pinned JSON is the single source of truth.
//! - `TopicCatalogue::lookup` returns the canonical (contract, event)
//!   for a topic. Unknown topic 0 is a decoder failure — the caller
//!   MUST fail readiness (per canonicality rule).

use crate::hybrid_v2::snapshot::PinnedSnapshots;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
struct TopicRow {
    contract: String,
    event: String,
    signature: String,
    topic0: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawCatalogue {
    #[serde(rename = "schemaVersion")]
    schema_version: u16,
    topics: Vec<TopicRow>,
}

#[derive(Debug, Clone)]
pub struct TopicEntry {
    pub contract: &'static str,
    pub event: &'static str,
    pub signature: &'static str,
    pub topic0: [u8; 32],
    pub topic0_hex_lower: &'static str,
}

pub struct TopicCatalogue {
    pub schema_version: u16,
    entries: Vec<TopicEntry>,
    by_topic: HashMap<[u8; 32], usize>,
    by_event: HashMap<&'static str, usize>,
}

impl TopicCatalogue {
    /// Return the process-wide singleton.
    pub fn get() -> &'static TopicCatalogue {
        static CELL: OnceLock<TopicCatalogue> = OnceLock::new();
        CELL.get_or_init(Self::load)
    }

    fn load() -> TopicCatalogue {
        let raw: RawCatalogue = serde_json::from_str(PinnedSnapshots::get().event_topics_v1)
            .expect("event-topics-v1.json snapshot must parse — regenerate from Solidity");
        let mut entries = Vec::with_capacity(raw.topics.len());
        let mut by_topic = HashMap::with_capacity(raw.topics.len());
        let mut by_event = HashMap::with_capacity(raw.topics.len());
        for row in raw.topics.into_iter() {
            let topic0 = parse_hex32(&row.topic0)
                .expect("topic0 must be 0x + 64 hex chars in pinned snapshot");
            // Leak into 'static — the catalogue lives for the lifetime of
            // the process, so this is cheap and lets consumers hold &str.
            let contract_static: &'static str = Box::leak(row.contract.into_boxed_str());
            let event_static: &'static str = Box::leak(row.event.into_boxed_str());
            let sig_static: &'static str = Box::leak(row.signature.into_boxed_str());
            let hex_static: &'static str = Box::leak(row.topic0.into_boxed_str());
            let entry = TopicEntry {
                contract: contract_static,
                event: event_static,
                signature: sig_static,
                topic0,
                topic0_hex_lower: hex_static,
            };
            let idx = entries.len();
            entries.push(entry);
            by_topic.insert(topic0, idx);
            by_event.insert(event_static, idx);
        }
        TopicCatalogue {
            schema_version: raw.schema_version,
            entries,
            by_topic,
            by_event,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn lookup_by_topic0(&self, topic0: &[u8; 32]) -> Option<&TopicEntry> {
        self.by_topic.get(topic0).map(|i| &self.entries[*i])
    }

    pub fn lookup_by_event(&self, event: &str) -> Option<&TopicEntry> {
        self.by_event.get(event).map(|i| &self.entries[*i])
    }

    pub fn entries(&self) -> &[TopicEntry] {
        &self.entries
    }
}

/// Parse a `0x` + 64-hex-char string into a fixed-32-byte array.
pub fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let byte_str = &hex[i * 2..i * 2 + 2];
        out[i] = u8::from_str_radix(byte_str, 16).ok()?;
    }
    Some(out)
}
