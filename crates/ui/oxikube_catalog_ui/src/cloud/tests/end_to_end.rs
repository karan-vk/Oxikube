//! The section over the real `CloudDiscoveryService` on fakes: nothing but the fake port ever
//! runs a "CLI", and only through spawned tasks.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::cloud::{CloudDiscoveryService, DiscoveryDeps};
use oxikube_app::sources::MemorySourceList;
use oxikube_app::store::Spawner;
use oxikube_app::{ClusterSessionManager, KubeconfigSourcesService};
use oxikube_domain::cloud::CloudProvider;
use oxikube_ports::{CloudDiscoveryPort, UserSource};
use oxikube_testkit::{
    CloudCall, FakeClockPort, FakeCloudDiscoveryPort, FakeClusterConnectorPort,
    FakeClusterSourcePort, FakeFsPort, FakeStatePort,
};
use oxikube_workspace::OpenOptions;
use oxikube_workspace::test_support::open_workspace;

use super::Harness;
use crate::cloud::test_support::{aws_scope, eks};
use crate::cloud::{CloudDeps, CloudView, ServiceBackend};

const DIR: &str = "/config/kubeconfigs";

struct Rig {
    port: Arc<FakeCloudDiscoveryPort>,
    fs: Arc<FakeFsPort>,
    service: CloudDiscoveryService,
}

fn rig(cx: &TestAppContext, names: &[&str]) -> Rig {
    let scope = aws_scope("prod", "eu-west-1");
    let port = Arc::new(
        FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
            .with_scopes([scope.clone()])
            .with_clusters(names.iter().map(|name| eks(&scope, name))),
    );
    let fs = Arc::new(FakeFsPort::new());
    let source = Arc::new(FakeClusterSourcePort::new());
    let clock = Arc::new(FakeClockPort::new(
        jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp"),
    ));
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        source.clone(),
        clock.clone(),
    );
    let sources = KubeconfigSourcesService::new(
        source.clone(),
        fs.clone(),
        Arc::new(MemorySourceList::new([UserSource::default_source()])),
        PathBuf::from(DIR),
    );
    let executor = cx.executor();
    let spawner: Arc<dyn Spawner> = Arc::new(move |task| executor.spawn(task).detach());
    let service = CloudDiscoveryService::new(DiscoveryDeps {
        providers: vec![port.clone() as Arc<dyn CloudDiscoveryPort>],
        state: Arc::new(FakeStatePort::new()),
        fs: fs.clone(),
        sources,
        source,
        sessions,
        clock,
        spawner,
    });
    Rig { port, fs, service }
}

#[gpui::test]
fn refresh_add_and_remove_work_end_to_end_off_the_ui_thread(cx: &mut TestAppContext) {
    let rig = rig(cx, &["web"]);
    let backend = Rc::new(ServiceBackend::new(rig.service.clone()));
    let mut h = Harness::open(cx, backend);

    // The first refresh ran through spawned tasks and listed the cluster.
    assert!(h.is_laid_out("cloud-card-aws"));
    assert!(h.is_laid_out("cloud-row-0"));
    assert!(!h.is_laid_out("cloud-card-gcp"), "no adapter, no card");

    // Add writes the managed file (owner-only, exec auth) and marks the row.
    h.click("cloud-add-0");
    h.vcx.run_until_parked();
    let file = rig
        .fs
        .file(rig.service.managed_path(CloudProvider::Aws))
        .expect("the managed kubeconfig was written");
    let text = String::from_utf8(file).expect("utf-8");
    assert!(text.contains("exec:"), "{text}");
    assert!(!text.contains("token:"), "no static token: {text}");
    assert!(h.is_laid_out("cloud-in-catalog-0:true"));

    // Remove asks, then deletes the file with its last cluster.
    h.click("cloud-remove-0");
    h.click("cloud-confirm-0");
    h.vcx.run_until_parked();
    assert!(
        rig.fs
            .file(rig.service.managed_path(CloudProvider::Aws))
            .is_none()
    );
    assert!(h.is_laid_out("cloud-in-catalog-0:false"));
    assert!(h.is_laid_out("cloud-add-0"));
}

#[gpui::test]
fn the_port_is_never_called_on_the_ui_thread(cx: &mut TestAppContext) {
    let rig = rig(cx, &["web"]);
    let (workspace, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let deps = CloudDeps {
        backend: Rc::new(ServiceBackend::new(rig.service.clone())),
        clock: Arc::new(FakeClockPort::new(
            jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp"),
        )),
    };
    let calls = || rig.port.recorded_calls().len();
    vcx.update(|window, cx| {
        let view = cx.new(|cx| CloudView::new(deps, cx));
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(view.clone()), OpenOptions::default(), window, cx);
        });
        view.update(cx, |view, cx| {
            view.refresh_provider(CloudProvider::Aws, cx);
        });
        assert_eq!(calls(), 0, "a click only spawns; the CLI has not run yet");
    });
    vcx.run_until_parked();
    assert!(
        rig.port
            .recorded_calls()
            .iter()
            .any(|call| matches!(call, CloudCall::ListClusters(_))),
        "the spawned task ran the listing: {:?}",
        rig.port.recorded_calls()
    );
}
