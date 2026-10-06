//! Helpers for tests of the catalog and of the crates built on it (feature `test-support`):
//! a dispatcher that records what it was sent, and synthetic catalog entries.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::App;
use jiff::Timestamp;
use oxikube_app::CatalogEntry;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterSource, SourceId, SourceKind};

use super::dispatch::CommandDispatcher;

/// The source id of [`entry`] and [`synthetic_entries`].
pub const SOURCE_ID: &str = "default";

/// A [`CommandDispatcher`] that records every command instead of running it: the "fake bus" of
/// the catalog tests. Cheap to clone; clones share the record.
#[derive(Clone, Default)]
pub struct RecordingDispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
}

impl RecordingDispatcher {
    /// A dispatcher with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// The commands sent so far, oldest first.
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    /// Forgets the recorded commands.
    pub fn clear(&self) {
        self.sent.borrow_mut().clear();
    }
}

impl CommandDispatcher for RecordingDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.sent.borrow_mut().push(command);
    }
}

/// The id of the context `name` of the default source (what [`entry`] uses).
pub fn cluster_id(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

/// The default source: one kubeconfig file, labelled like the kube adapter labels files.
pub fn source() -> ClusterSource {
    ClusterSource {
        id: SourceId(SOURCE_ID.into()),
        kind: SourceKind::KubeconfigFile,
        label: "~/.kube/config".into(),
        path: Some("/home/me/.kube/config".into()),
    }
}

/// The context `name` of the default source, with a cluster and a user named after it.
pub fn context(name: &str) -> ClusterContext {
    ClusterContext {
        server: Some(format!("https://{name}.example:6443")),
        cluster_name: Some(format!("{name}-cluster")),
        user: Some(format!("{name}-user")),
        ..ClusterContext::new(
            cluster_id(name),
            ContextName::new(name),
            SourceId(SOURCE_ID.into()),
        )
    }
}

/// A catalog entry for the context `name`: not a favourite, never used.
pub fn entry(name: &str) -> CatalogEntry {
    CatalogEntry {
        context: context(name),
        source: Some(source()),
        favourite: false,
        last_used: None,
    }
}

/// `count` distinct entries named like real clusters (`prod-eu-17`, `staging-us-4`, ...), spread
/// over a few kubeconfig files, with every seventh a favourite and every third used at some point.
pub fn synthetic_entries(count: usize) -> Vec<CatalogEntry> {
    const ENVS: [&str; 5] = ["prod", "staging", "dev", "qa", "sandbox"];
    const REGIONS: [&str; 6] = ["eu", "us", "ap", "sa", "af", "me"];
    (0..count)
        .map(|i| {
            let name = format!(
                "{}-{}-{i}",
                ENVS[i % ENVS.len()],
                REGIONS[(i / ENVS.len()) % REGIONS.len()]
            );
            let mut entry = entry(&name);
            let file = i % 4;
            entry.context.source = SourceId(format!("file-{file}"));
            entry.source = Some(ClusterSource {
                id: SourceId(format!("file-{file}")),
                kind: SourceKind::KubeconfigFile,
                label: format!("~/.kube/config.d/team-{file}.yaml"),
                path: None,
            });
            entry.favourite = i % 7 == 0;
            entry.last_used = (i % 3 == 0)
                .then(|| Timestamp::from_second(1_700_000_000 + i as i64 * 60).expect("in range"));
            entry
        })
        .collect()
}
