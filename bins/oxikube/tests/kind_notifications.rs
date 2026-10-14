//! The notification centre against a real cluster (E14-S05): the app on its real init order with
//! the kind context connected (its main window headless). A pod whose image cannot be pulled, in a
//! namespace of the test's own, makes the kubelet emit Warning events about a Pod (a watched kind):
//! the centre lists a notification about that pod, a toast shows it and the status-bar bell
//! counts it, and the key opens the notification panel in the right dock. Muting the cluster
//! (`notifications::MuteCluster` through the bus) stops the cluster's watch for notifications and
//! is saved in the state db; a second failing pod then adds nothing.
//!
//! `cargo test -p oxikube --features integration --test kind_notifications` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it the test returns at once. The
//! pods and the namespace are the test's own (`oxi-test-<rand>`, deleted at the end); no node is
//! touched. Other suites share the cluster, so the test only asserts about its own pods.
#![cfg(feature = "integration")]

#[path = "kind_common/mod.rs"]
mod kind_common;

use futures::executor::block_on;
use gpui::TestAppContext;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind, PostParams};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::notifications::{MUTED_KEY, Notification};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_ports::{NotificationLevel, StateKey};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::DockPosition;
use oxikube_workspace::notifications::NotificationPanel;
use serde_json::json;

use kind_common::{Launched, launch, on_cluster, wait, wait_value, workspace};

/// Creates a pod whose image cannot be pulled (no registry answers for `.invalid`): the kubelet
/// reports `Failed` / `BackOff` Warning events about it within seconds. It is not waited for.
fn create_unpullable(context: &str, namespace: &str, name: &str) {
    let pod: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": name },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{ "name": "main", "image": "registry.invalid/oxikube/none:1" }]
        }
    }))
    .expect("a pod");
    on_cluster(context, async |client| {
        let resource =
            ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", "Pod"), "pods");
        kube::Api::<DynamicObject>::namespaced_with(client, namespace, &resource)
            .create(&PostParams::default(), &pod)
            .await
            .expect("create the pod");
    });
}

/// The centre's row about the pod `name` in `namespace`, if any.
fn row_about(rows: &[std::sync::Arc<Notification>], namespace: &str, name: &str) -> bool {
    rows.iter().any(|row| {
        row.target
            .as_ref()
            .is_some_and(|t| t.name.as_ref() == name && t.namespace.as_deref() == Some(namespace))
    })
}

#[gpui::test]
fn a_warning_on_a_watched_pod_toasts_and_a_muted_cluster_stays_quiet(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    let Launched {
        mut vcx,
        cluster,
        dir,
        ..
    } = launch(cx, &context);
    let state = vcx.update(|_, cx| AppState::global(cx));
    let centre = state.services().notifications.clone();
    wait(&mut vcx, "the centre to watch the cluster", |_| {
        centre.is_watching(&cluster)
    });

    // A Warning event about a pod of the test's own: listed, toasted, counted.
    create_unpullable(&context, ns.name(), "oxi-notify-a");
    wait(&mut vcx, "a notification about the pod", |_| {
        row_about(&centre.list(), ns.name(), "oxi-notify-a")
    });
    let rows = centre.list();
    let row = rows
        .iter()
        .find(|r| row_about(std::slice::from_ref(r), ns.name(), "oxi-notify-a"))
        .expect("the row");
    assert_eq!(row.level, NotificationLevel::Warning);
    assert!(row.title.contains("oxi-notify-a"), "{}", row.title);
    assert!(centre.unread() >= 1);
    let workspace = workspace(&mut vcx);
    // Other suites' warnings share the raise limit, so any notification toast (or the summary
    // that stands for the rest) proves the path; the row's own toast is the common case.
    wait(&mut vcx, "a notification toast", |vcx| {
        vcx.update(|_, cx| {
            workspace
                .read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .iter()
                .any(|t| {
                    t.key
                        .as_deref()
                        .is_some_and(|k| k.starts_with("notification"))
                })
        })
    });

    // The key opens the centre in the right dock.
    let key = if cfg!(target_os = "macos") {
        "cmd-shift-i"
    } else {
        "ctrl-shift-i"
    };
    vcx.simulate_keystrokes(key);
    wait(&mut vcx, "the notification panel to open", |vcx| {
        vcx.update(|_, cx| {
            let ws = workspace.read(cx);
            ws.panel::<NotificationPanel>().is_some()
                && ws
                    .dock(DockPosition::Right, cx)
                    .is_some_and(|dock| dock.is_open())
        })
    });

    // Muting the cluster stops its watch and is saved; a new failing pod adds nothing.
    let bus = vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the bus");
    let muted = block_on(bus.dispatch(
        Command::NotificationsMuteCluster {
            cluster: cluster.clone(),
        },
        DispatchContext::new(Initiator::Ui, "kind test"),
    ));
    assert!(muted.is_ok(), "{muted:?}");
    wait(&mut vcx, "the muted cluster to be unwatched", |_| {
        centre.is_muted(&cluster) && !centre.is_watching(&cluster)
    });
    let id = serde_json::to_value(&cluster).expect("an id");
    wait_value(&mut vcx, "the mute to be saved", |vcx| {
        let store = vcx.update(|_, cx| AppState::global(cx).ports().state.clone());
        block_on(store.kv_get(&StateKey::new(MUTED_KEY).expect("a key")))
            .ok()
            .flatten()
            .filter(|stored| {
                stored["clusters"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&id))
            })
    });
    let before = centre.list().len();
    create_unpullable(&context, ns.name(), "oxi-notify-b");
    // The second pod's warnings arrive within seconds; give them that long to (not) land.
    let quiet_until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < quiet_until {
        vcx.run_until_parked();
        assert!(
            !row_about(&centre.list(), ns.name(), "oxi-notify-b"),
            "a muted cluster notified"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(centre.list().len(), before, "nothing new while muted");
    drop(dir);
}
