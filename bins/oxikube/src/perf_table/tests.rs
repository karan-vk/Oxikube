//! `--perf-table` over fakes, on the real init path: it connects the seeded context through the
//! command bus, opens the pods table in the cluster tab and scrolls it.

use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_domain::Resource;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_domain::session::SessionPhase;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{TestPorts, pod};
use oxikube_workspace::ClusterTab;

use super::{TableDrive, start};
use crate::app_state::AppState;
use crate::startup::{StartupEnv, init, window};

const PODS: usize = 300;

fn pods_kind() -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("", "v1", "Pod"),
        preferred: true,
        plural: "pods".into(),
        singular: "pod".into(),
        short_names: vec!["po".into()],
        categories: vec!["all".into()],
        verbs: VerbSet::from_names(["get", "list", "watch"]),
        namespaced: true,
    }
}

fn load_pod(i: usize) -> Resource {
    pod().namespace("load").name(format!("load-{i:04}")).build()
}

/// The pods table in the seeded cluster's tab, once there is one.
fn pods_table(vcx: &mut VisualTestContext) -> Option<Entity<ResourceTable>> {
    vcx.update(|window, cx| {
        let main = window::main_view(window, cx)?;
        let workspace = main.read(cx).workspace().clone();
        let tab: Entity<ClusterTab> = workspace
            .read(cx)
            .items_of_type::<ClusterTab>()
            .into_iter()
            .next()?;
        let inner = tab.read(cx).workspace().clone();
        inner
            .read(cx)
            .items_of_type::<ResourceTable>()
            .into_iter()
            .next()
    })
}

#[gpui::test]
fn it_connects_the_context_opens_its_pods_table_and_scrolls_it(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let cluster_ports = ports.connector.ports_for(&TestPorts::cluster_id());
    cluster_ports.discovery.set_kinds([pods_kind()]);
    for i in 0..PODS {
        cluster_ports.resources.insert(load_pod(i));
    }
    cx.update(|cx| init(cx, StartupEnv::test_with(&ports)))
        .expect("the init order runs");
    let handle = cx
        .update(|cx| window::open_main_window(cx, |content, _| content))
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();

    let drive = TableDrive {
        context: TestPorts::CONTEXT.to_owned(),
        scroll: 10,
    };
    vcx.update(|_, cx| start(drive, handle.into(), cx));

    // Run the app's clock until the table has scrolled (each step polls or scrolls at most every
    // 50 ms, the scroll every frame interval).
    let mut scrolled = None;
    for _ in 0..400 {
        vcx.executor().advance_clock(Duration::from_millis(10));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        if let Some(table) = pods_table(&mut vcx) {
            let visible = vcx.update(|_, cx| table.read(cx).table().visible_rows(cx));
            if visible.start > 0 {
                scrolled = Some((table, visible));
                break;
            }
        }
    }
    let (table, visible) = scrolled.expect("the drive opened the pods table and scrolled it");

    let state = vcx.update(|_, cx| AppState::global(cx));
    let phase = state
        .services()
        .sessions
        .get(&TestPorts::cluster_id())
        .map(|s| s.phase());
    assert_eq!(
        phase,
        Some(SessionPhase::Ready),
        "connected through the bus"
    );
    let rows = vcx.update(|_, cx| table.read(cx).read_rows(cx, |d| d.rows().len()));
    assert_eq!(rows, PODS, "the table lists the cluster's pods");
    assert!(
        visible.start > 0 && visible.end <= PODS,
        "scrolled down: {visible:?}"
    );
}

#[gpui::test]
fn an_unknown_context_leaves_the_app_as_it_was(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    cx.update(|cx| init(cx, StartupEnv::test_with(&ports)))
        .expect("the init order runs");
    let handle = cx
        .update(|cx| window::open_main_window(cx, |content, _| content))
        .expect("the main window opens");
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    vcx.run_until_parked();
    let drive = TableDrive {
        context: "no-such-context".to_owned(),
        scroll: 3,
    };
    vcx.update(|_, cx| start(drive, handle.into(), cx));
    vcx.executor().advance_clock(Duration::from_secs(1));
    vcx.run_until_parked();
    assert!(pods_table(&mut vcx).is_none());
    let state = vcx.update(|_, cx| AppState::global(cx));
    assert!(
        state.services().sessions.sessions().is_empty(),
        "nothing connected"
    );
}
