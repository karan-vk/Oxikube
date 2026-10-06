//! Tests of the SQLite state store, all on temp directories.

mod audit;
mod parity;
mod port;
mod recovery;

use std::path::PathBuf;

use futures::executor::block_on;
use jiff::Timestamp;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{StateKey, StateTable};
use tempfile::TempDir;

use crate::SqliteState;

pub(super) fn key(k: &str) -> StateKey {
    StateKey::new(k).unwrap()
}

pub(super) fn table(t: &str) -> StateTable {
    StateTable::new(t).unwrap()
}

/// A temp dir holding `state.db`.
pub(super) struct Fixture {
    pub dir: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.path().join("state.db")
    }

    pub fn open(&self) -> SqliteState {
        block_on(self.open_async())
    }

    pub async fn open_async(&self) -> SqliteState {
        SqliteState::open(self.path()).await.expect("opens")
    }
}

pub(super) fn cluster(name: &str) -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::new(name))
}

pub(super) fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

pub(super) fn record(ctx: &str, cmd: &str, at: &str, outcome: AuditOutcome) -> AuditRecord {
    let target = ResourceRef::namespaced(
        cluster(ctx),
        Gvk::from_api_version("v1", "Pod"),
        "default",
        "web-0",
    );
    AuditRecord::new(ts(at), "alice", Initiator::Ui, cmd, target, false, outcome)
}
