//! [`DiffService`]: reads two objects in parallel (connecting a cluster that is not connected),
//! normalises both and compares them.

use std::sync::Arc;

use futures::future::try_join;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::session::ClusterSessionState;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::ResourceReader;
use serde_json::Value;

use super::changes::changes;
use super::ignore::IgnoreList;
use super::keys::key_report;
use super::normalize::{DiffOptions, Fingerprinter, normalize};
use super::result::{DiffSide, ResourceDiff};
use crate::session::ClusterSessionManager;

/// One comparison: two objects of one kind and what to look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRequest {
    /// The left side.
    pub left: ResourceRef,
    /// The right side.
    pub right: ResourceRef,
    /// Compare `status` too.
    pub include_status: bool,
    /// Compare `metadata.name` and `metadata.namespace` too.
    pub compare_identity: bool,
}

impl DiffRequest {
    /// A comparison of `left` with `right` that ignores `status`, the name and the namespace.
    pub fn new(left: ResourceRef, right: ResourceRef) -> Self {
        Self {
            left,
            right,
            include_status: false,
            compare_identity: false,
        }
    }
}

/// Where a comparison is, for the view's progress line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffProgress {
    /// This cluster is not connected; connecting it.
    Connecting(ClusterId),
    /// Reading the two objects.
    Reading,
}

impl DiffProgress {
    /// The line the view shows.
    pub fn text(&self) -> String {
        match self {
            DiffProgress::Connecting(cluster) => format!("Connecting to {cluster}…"),
            DiffProgress::Reading => "Reading both objects…".to_owned(),
        }
    }
}

/// Compares objects of two clusters or namespaces. Read-only: it only reads, so it is not behind
/// the `MutationGuard`. Clone-cheap.
#[derive(Clone)]
pub struct DiffService {
    sessions: ClusterSessionManager,
    ignore: IgnoreList,
}

impl DiffService {
    /// A service over `sessions`, leaving out `ignore` (the `diff.ignore_paths` setting, kept in
    /// step by whoever owns the handle).
    pub fn new(sessions: ClusterSessionManager, ignore: IgnoreList) -> Self {
        Self { sessions, ignore }
    }

    /// Reads and compares the two objects of `request`. Run it off the UI thread
    /// (`oxikube_runtime::spawn_kube`); the work is the two reads side by side and one pass over
    /// each object.
    ///
    /// # Errors
    ///
    /// `Validation` when the two are not of one kind, `NotFound` when neither exists, the
    /// cluster's error when a read fails or a cluster cannot be connected.
    pub async fn diff(
        &self,
        request: &DiffRequest,
        progress: &(dyn Fn(DiffProgress) + Send + Sync),
    ) -> OxiResult<ResourceDiff> {
        same_kind(&request.left, &request.right)?;
        let (left_reader, right_reader) = if request.left.cluster == request.right.cluster {
            // One connect for one cluster: two concurrent connects would leave the second caller
            // with a `Connecting` state and a spurious "did not connect".
            let reader = self.reader(&request.left.cluster, progress).await?;
            (Arc::clone(&reader), reader)
        } else {
            try_join(
                self.reader(&request.left.cluster, progress),
                self.reader(&request.right.cluster, progress),
            )
            .await?
        };
        progress(DiffProgress::Reading);
        let (left, right) = try_join(
            read(left_reader.as_ref(), &request.left, &request.right),
            read(right_reader.as_ref(), &request.right, &request.right),
        )
        .await?;
        if left.is_none() && right.is_none() {
            return Err(OxiError::not_found(format!(
                "neither {} nor {} exists",
                request.left, request.right
            )));
        }
        let options = DiffOptions {
            ignore: (*self.ignore.get()).clone(),
            include_status: request.include_status,
            compare_identity: request.compare_identity,
        };
        let mut diff = compare(request, left, right, &options);
        diff.left.cluster_title = self.title(&request.left.cluster);
        diff.right.cluster_title = self.title(&request.right.cluster);
        Ok(diff)
    }

    /// The reader of `cluster`'s session, connecting the session first when it is not connected.
    async fn reader(
        &self,
        cluster: &ClusterId,
        progress: &(dyn Fn(DiffProgress) + Send + Sync),
    ) -> OxiResult<Arc<dyn ResourceReader>> {
        if let Some(reader) = self.connected_reader(cluster) {
            return Ok(reader);
        }
        progress(DiffProgress::Connecting(cluster.clone()));
        let state = self.sessions.connect(cluster).await?;
        self.connected_reader(cluster)
            .ok_or_else(|| not_connected(cluster, &state))
    }

    /// The cluster as its session names it, else its id.
    fn title(&self, cluster: &ClusterId) -> String {
        self.sessions
            .get(cluster)
            .map_or_else(|| cluster.to_string(), |session| session.title().to_owned())
    }

    fn connected_reader(&self, cluster: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        let session = self.sessions.get(cluster)?;
        session
            .is_connected()
            .then(|| session.resources())
            .flatten()
    }
}

fn not_connected(cluster: &ClusterId, state: &ClusterSessionState) -> OxiError {
    match state {
        ClusterSessionState::AuthRequired { reason } => {
            OxiError::auth(format!("{cluster} needs credentials: {reason}"), false)
        }
        ClusterSessionState::Error { reason } => {
            OxiError::network(format!("{cluster} did not connect: {reason}"))
        }
        other => OxiError::network(format!("{cluster} did not connect ({:?})", other.phase())),
    }
}

/// The two refs must name one kind; a different version is read at the right side's.
fn same_kind(left: &ResourceRef, right: &ResourceRef) -> OxiResult<()> {
    if left.gvk.group != right.gvk.group || left.gvk.kind != right.gvk.kind {
        return Err(OxiError::validation(format!(
            "{} cannot be compared with {}: pick an object of the same kind",
            left.gvk, right.gvk
        )));
    }
    Ok(())
}

/// Reads `target`'s object at `at`'s apiVersion (the second pick's, so both sides are read at one
/// version the user chose), mapping "this server does not serve it" to a clear message.
async fn read(
    reader: &dyn ResourceReader,
    target: &ResourceRef,
    at: &ResourceRef,
) -> OxiResult<Option<Resource>> {
    reader
        .get_opt(&at.gvk, target.namespace(), &target.name)
        .await
        .map_err(|err| match err.kind() {
            oxikube_domain::ErrorKind::Unsupported => OxiError::unsupported(format!(
                "{} does not serve {}: choose an object of an apiVersion both clusters serve",
                target.cluster, at.gvk
            )),
            _ => err,
        })
}

/// The comparison of two objects already read. Pure: normalising and diffing only.
pub fn compare(
    request: &DiffRequest,
    left: Option<Resource>,
    right: Option<Resource>,
    options: &DiffOptions,
) -> ResourceDiff {
    let fingerprints = Fingerprinter::new();
    let side = |target: &ResourceRef, object: Option<Resource>| DiffSide {
        target: target.clone(),
        cluster_title: target.cluster.to_string(),
        object: object.map(|object| normalize(&object.to_value(), options, &fingerprints)),
    };
    let left = side(&request.left, left);
    let right = side(&request.right, right);
    // A missing side compares as an empty object: everything of the other side is added.
    let empty = Value::Object(serde_json::Map::new());
    let (l, r) = (
        left.object.as_ref().unwrap_or(&empty),
        right.object.as_ref().unwrap_or(&empty),
    );
    let paths = changes(l, r);
    let keys = key_report(
        left.object.as_ref().unwrap_or(&Value::Null),
        right.object.as_ref().unwrap_or(&Value::Null),
    );
    ResourceDiff {
        left,
        right,
        changes: paths,
        keys,
        include_status: options.include_status,
        compare_identity: options.compare_identity,
    }
}
