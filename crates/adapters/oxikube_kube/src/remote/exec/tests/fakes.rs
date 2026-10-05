//! A scripted [`Pods`] that records what the exec adapter does to the cluster.

use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::remote::exec::pods::{PodShape, PodStamp, Pods};
use crate::remote::exec::wait::Container;

/// One call made on [`FakePods`].
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Call {
    Create { namespace: String, manifest: Value },
    Delete { namespace: String, name: String },
    List { selector: String },
    Patch { pod: String, body: Value },
    Wait { pod: String, container: Container },
}

/// An error to fail with: its kind and message.
pub(super) type Fail = Mutex<Option<(ErrorKind, &'static str)>>;

fn fail(failure: &Fail) -> OxiResult<()> {
    match *failure.lock() {
        Some((kind, message)) => Err(OxiError::new(kind, message)),
        None => Ok(()),
    }
}

pub(super) struct FakePods {
    calls: Mutex<Vec<Call>>,
    /// Fails every `wait_running` when set.
    pub(super) wait_error: Fail,
    /// Fails every `delete` when set.
    pub(super) delete_error: Fail,
    /// Pods `list` returns.
    pub(super) listed: Mutex<Vec<PodStamp>>,
    /// Fails `add_ephemeral_container` when set.
    pub(super) patch_error: Fail,
    deleted: mpsc::UnboundedSender<String>,
}

impl FakePods {
    /// The fake and the receiver that yields the name of each pod it deletes.
    pub(super) fn new() -> (Self, mpsc::UnboundedReceiver<String>) {
        let (deleted, rx) = mpsc::unbounded_channel();
        (
            Self {
                calls: Mutex::default(),
                wait_error: Mutex::default(),
                delete_error: Mutex::default(),
                listed: Mutex::default(),
                patch_error: Mutex::default(),
                deleted,
            },
            rx,
        )
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }

    pub(super) fn deletions(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Delete { name, .. } => Some(name),
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl Pods for FakePods {
    async fn create(&self, namespace: &str, manifest: &Value) -> OxiResult<String> {
        self.calls.lock().push(Call::Create {
            namespace: namespace.into(),
            manifest: manifest.clone(),
        });
        Ok("oxikube-node-shell-abc12".into())
    }

    async fn delete(&self, namespace: &str, name: &str) -> OxiResult<()> {
        self.calls.lock().push(Call::Delete {
            namespace: namespace.into(),
            name: name.into(),
        });
        let _ = self.deleted.send(name.to_owned());
        fail(&self.delete_error)
    }

    async fn list(&self, _namespace: &str, label_selector: &str) -> OxiResult<Vec<PodStamp>> {
        self.calls.lock().push(Call::List {
            selector: label_selector.into(),
        });
        Ok(self.listed.lock().clone())
    }

    async fn shape(&self, _namespace: &str, _name: &str) -> OxiResult<Option<PodShape>> {
        Ok(None)
    }

    async fn add_ephemeral_container(
        &self,
        _namespace: &str,
        pod: &str,
        patch: &Value,
    ) -> OxiResult<()> {
        self.calls.lock().push(Call::Patch {
            pod: pod.into(),
            body: patch.clone(),
        });
        fail(&self.patch_error)
    }

    async fn wait_running(
        &self,
        _namespace: &str,
        pod: &str,
        container: &Container,
        _timeout: Duration,
    ) -> OxiResult<()> {
        self.calls.lock().push(Call::Wait {
            pod: pod.into(),
            container: container.clone(),
        });
        fail(&self.wait_error)
    }
}
