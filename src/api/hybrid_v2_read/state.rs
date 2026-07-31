//! `HybridV2ApiState` — shared state consumed by the read API router.
//!
//! Holds a registry of configured Hybrid V2 deployments, each with its own
//! `IndexerRuntime` in a read-lock. The router borrows the projection state
//! + cursor + readiness from the runtime under a read lock.

use crate::hybrid_v2::manifest::ManifestParams;
use crate::hybrid_v2::runtime::IndexerRuntime;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// One live deployment plus its projection runtime.
pub struct DeploymentEntry {
    pub deployment_id: u64,
    pub manifest: ManifestParams,
    pub runtime: RwLock<IndexerRuntime>,
}

impl DeploymentEntry {
    pub fn new(runtime: IndexerRuntime) -> Self {
        Self {
            deployment_id: runtime.deployment_id,
            manifest: runtime.manifest.clone(),
            runtime: RwLock::new(runtime),
        }
    }
}

/// Router state. Cloneable via `Arc` — every handler holds a shared reference.
#[derive(Clone)]
pub struct HybridV2ApiState {
    inner: Arc<HybridV2ApiStateInner>,
}

struct HybridV2ApiStateInner {
    deployments: BTreeMap<u64, Arc<DeploymentEntry>>,
}

impl HybridV2ApiState {
    pub fn new(deployments: Vec<Arc<DeploymentEntry>>) -> Self {
        let mut map = BTreeMap::new();
        for entry in deployments {
            map.insert(entry.deployment_id, entry);
        }
        Self {
            inner: Arc::new(HybridV2ApiStateInner { deployments: map }),
        }
    }

    pub fn empty() -> Self {
        Self {
            inner: Arc::new(HybridV2ApiStateInner {
                deployments: BTreeMap::new(),
            }),
        }
    }

    pub fn get(&self, deployment_id: u64) -> Option<Arc<DeploymentEntry>> {
        self.inner.deployments.get(&deployment_id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<DeploymentEntry>> {
        self.inner.deployments.values().cloned().collect()
    }

    /// Return the sole configured deployment when exactly one exists.
    /// Every subaccount route accepts an implicit deployment when
    /// exactly one is configured; otherwise callers must pass an
    /// explicit `deployment_id` query.
    pub fn resolve_single(&self) -> Option<Arc<DeploymentEntry>> {
        if self.inner.deployments.len() == 1 {
            self.inner.deployments.values().next().cloned()
        } else {
            None
        }
    }
}
