//! The view's cards and clicks: tiles open the kind's list, a top row opens its pod, the info
//! card reads the version and the distribution and hides the server's credentials, the warnings
//! come from the hook, and a forbidden feed says "no access".

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::{FixedWarnings, MetricsKind, WarningRow};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{MetricsOutcome, ServerVersion};
use oxikube_testkit::{node, pod};

use super::{Fixture, Setup, cluster, pod_sample};

#[gpui::test]
fn a_tile_opens_its_kinds_list_and_a_top_row_opens_its_pod(cx: &mut TestAppContext) {
    let mut f = Fixture::open(
        cx,
        Setup {
            objects: vec![
                node().name("n1").build(),
                pod()
                    .namespace("shop")
                    .name("web-0")
                    .running()
                    .node("n1")
                    .build(),
            ],
            metrics: Some(vec![(
                MetricsKind::Pods,
                MetricsOutcome::Available(vec![pod_sample("shop", "web-0", 300, 100)]),
            )]),
            ..Setup::default()
        },
    );
    f.tick();
    f.click("overview-tile-nodes-ready");
    f.click("overview-tile-pods-pending");
    f.click("overview-top-pods-cpu-0");
    assert_eq!(
        f.recorder.sent(),
        [
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Node"),
            },
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::ResourceOpen {
                target: ResourceRef::namespaced(
                    cluster(),
                    Gvk::new("", "v1", "Pod"),
                    "shop",
                    "web-0"
                ),
            },
        ]
    );
}

#[gpui::test]
fn the_info_card_reads_version_distribution_and_a_clean_server(cx: &mut TestAppContext) {
    let mut f = Fixture::open(
        cx,
        Setup {
            objects: vec![node().name("n1").build(), node().name("n2").build()],
            ..Setup::default()
        },
    );
    f.tick();
    let info = f.read(|o| o.info());
    assert_eq!(info.context, "prod");
    assert_eq!(info.nodes, "2");
    assert_eq!(
        info.server, "https://prod.example:6443/",
        "credentials and query cut out"
    );
    assert_eq!(info.version, "v1.33.0", "the server's version, read once");
    assert_eq!(
        info.distribution, "unknown",
        "nothing identifies the cluster"
    );
    assert!(f.drawn("overview-info-server"));
}

#[gpui::test]
fn a_k3s_version_names_the_distribution(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, Setup::default());
    // Before the overview opened, the fake answers its default; script the next read and reconnect.
    f.ports
        .discovery
        .script()
        .server_version
        .push_ok(ServerVersion {
            major: "1".into(),
            minor: "31".into(),
            git_version: "v1.31.1+k3s1".into(),
            platform: "linux/arm64".into(),
        });
    f.sessions.disconnect(&cluster()).expect("disconnect");
    f.tick();
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.tick();
    let info = f.read(|o| o.info());
    assert_eq!(info.version, "v1.31.1+k3s1");
    assert_eq!(info.distribution, "k3s");
}

#[gpui::test]
fn warnings_come_from_the_hook_and_say_when_there_are_none(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, Setup::default());
    f.tick();
    assert!(f.drawn("overview-warnings-empty"));

    let rows = vec![WarningRow {
        namespace: Some("shop".into()),
        object: "Pod/web-0".into(),
        reason: "BackOff".into(),
        message: "Back-off restarting failed container".into(),
        count: 4,
    }];
    let mut f = Fixture::open(
        cx,
        Setup {
            warnings: Arc::new(FixedWarnings(rows.clone())),
            ..Setup::default()
        },
    );
    f.tick();
    assert_eq!(f.read(|o| o.warnings().to_vec()), rows);
    assert!(f.drawn("overview-warning-0"));
}

#[gpui::test]
fn a_forbidden_pod_feed_shows_no_access_not_zero(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, Setup::default());
    // Re-open the feeds with both watches refused (the executor picks which feed asks first).
    for _ in 0..2 {
        f.ports
            .resources
            .script()
            .watch
            .push_err(OxiError::forbidden(
                "forbidden: cannot list at the cluster scope",
            ));
    }
    let overview = f.overview.clone();
    f.vcx
        .update(|_, cx| overview.update(cx, |o, cx| o.set_tab_shown(false, cx)));
    f.tick();
    f.vcx
        .update(|_, cx| overview.update(cx, |o, cx| o.set_tab_shown(true, cx)));
    f.tick();
    let tiles = f.read(|o| o.shown().tiles.clone());
    let running = tiles.iter().find(|t| t.id == "pods-running").expect("tile");
    assert_eq!(running.value, "no access");
    assert!(
        running
            .hover
            .as_deref()
            .is_some_and(|h| h.contains("forbidden"))
    );
}

#[gpui::test]
fn top_consumers_say_when_they_cover_only_the_namespace_selection(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, Setup::default());
    f.tick();
    assert_eq!(f.read(|o| o.top_scope().map(str::to_owned)), None);
    assert!(!f.drawn("overview-top-scope"), "all namespaces: no note");

    f.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::single("default"))
        .expect("the session is open");
    f.tick();
    assert_eq!(
        f.read(|o| o.top_scope().map(str::to_owned)).as_deref(),
        Some("Only the selected namespace: default")
    );
    assert!(f.drawn("overview-top-scope"));

    f.sessions
        .set_namespace_selection(&cluster(), NamespaceSelection::All)
        .expect("the session is open");
    f.tick();
    assert!(!f.drawn("overview-top-scope"));
}
