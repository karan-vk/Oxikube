//! With a release source the picker and the current release read through the session's native
//! reader (the Releases page's, cached per connection), never `helm list -A` / `history` / `get
//! values`; `helm` only renders and upgrades.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{Release, ReleaseStatus};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::HelmPort;
use oxikube_testkit::{FakeHelmPort, HelmCall};
use parking_lot::Mutex;

use super::{Setup, now, revision, values, web};
use crate::helm::KindScopes;
use crate::helm_releases::ReleaseSource;
use crate::session::SessionUpdate;
use crate::store::ResourceStore;

/// A source whose cluster is connected while `native` is set, watching `apps` only.
struct Native {
    native: Mutex<Option<Arc<FakeHelmPort>>>,
}

impl ReleaseSource for Native {
    fn helm(&self, _: &ClusterId) -> Option<Arc<dyn HelmPort>> {
        self.native
            .lock()
            .clone()
            .map(|port| port as Arc<dyn HelmPort>)
    }

    fn namespaces(&self, _: &ClusterId) -> NamespaceSelection {
        NamespaceSelection::Set(["demo".to_owned()].into_iter().collect())
    }

    fn store(&self, _: &ClusterId) -> Option<ResourceStore> {
        None
    }

    fn kinds(&self, _: &ClusterId) -> BoxFuture<'static, OxiResult<Arc<KindScopes>>> {
        Box::pin(async { Ok(Arc::new(KindScopes::default())) })
    }

    fn session_updates(&self) -> BoxStream<'static, SessionUpdate> {
        Box::pin(futures::stream::pending())
    }
}

fn setup() -> (Setup, Arc<FakeHelmPort>, Arc<Native>) {
    let mut s = Setup::new(false);
    let native = Arc::new(FakeHelmPort::native_only());
    let source = Arc::new(Native {
        native: Mutex::new(Some(native.clone())),
    });
    s.flow = s.flow.clone().with_release_source(source.clone());
    (s, native, source)
}

fn cli_reads(s: &Setup) -> Vec<HelmCall> {
    s.helm
        .recorded_calls()
        .into_iter()
        .filter(|c| {
            matches!(
                c,
                HelmCall::ListReleases(..) | HelmCall::History(_) | HelmCall::Values { .. }
            )
        })
        .collect()
}

#[test]
fn the_picker_lists_the_tabs_namespaces_through_the_native_reader() {
    let (s, native, _) = setup();
    native.script().list_releases.push_ok(vec![Release::summary(
        web(),
        oxikube_domain::helm::ChartRef::new("nginx", "15.0.0"),
        3,
        ReleaseStatus::Deployed,
        jiff::Timestamp::UNIX_EPOCH,
    )]);
    let found = now(s.flow.releases(&web().cluster)).expect("listed");
    assert_eq!(found.len(), 1);
    let only_demo = NamespaceSelection::Set(["demo".to_owned()].into_iter().collect());
    assert_eq!(
        native.recorded_calls(),
        [HelmCall::ListReleases(web().cluster, only_demo)]
    );
    assert!(cli_reads(&s).is_empty(), "no `helm list -A`");
}

#[test]
fn the_current_release_reads_natively() {
    let (s, native, _) = setup();
    native.script().history.push_ok(vec![revision(3, "15.0.0")]);
    native.script().values.push_ok(values("replicaCount: 2\n"));
    let current = now(s.flow.current(&web())).expect("read");
    assert_eq!(current.revision, 3);
    assert!(current.values.expose().contains("replicaCount: 2"));
    assert!(cli_reads(&s).is_empty(), "no `helm history` / `get values`");
}

#[test]
fn a_disconnected_cluster_is_a_network_error() {
    let (s, _, source) = setup();
    *source.native.lock() = None;
    let error = now(s.flow.releases(&web().cluster)).unwrap_err();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Network);
    assert!(now(s.flow.current(&web())).is_err());
    assert!(cli_reads(&s).is_empty());
}
