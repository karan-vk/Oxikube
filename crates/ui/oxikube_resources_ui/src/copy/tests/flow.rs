//! The flow: cluster, namespace, review, copy; and the stops on the way.

use gpui::TestAppContext;
use oxikube_workspace::ToastLevel;

use super::*;
use crate::copy::{CopyStage, NamespaceList};

#[gpui::test]
fn the_cluster_picker_lists_the_connected_clusters_and_filters_by_typing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(configmap());
    rig.begin(source);
    let picker = rig.cluster_picker().expect("the cluster picker is open");
    let names: Vec<String> = rig.vcx.update(|_, cx| {
        picker
            .read(cx)
            .delegate
            .choices()
            .iter()
            .map(|c| c.name.clone())
            .collect()
    });
    assert_eq!(names, ["a", "b"]);
    rig.vcx.update(|window, cx| window.draw(cx).clear(cx));
    for selector in [
        "picker",
        "copy-cluster-picker",
        "copy-cluster-row-0",
        "copy-cluster-row-1",
    ] {
        assert!(
            rig.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
    rig.vcx.simulate_input("b");
    rig.settle();
    let selected = rig
        .vcx
        .update(|_, cx| picker.read(cx).delegate.selected().map(|c| c.name.clone()));
    assert_eq!(selected.as_deref(), Some("b"));
}

#[gpui::test]
fn picking_a_cluster_then_a_namespace_then_copy_writes_the_object_to_the_target(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx);
    let source = rig.source(configmap());
    rig.begin(source);

    // The second row is `b`.
    rig.vcx.simulate_keystrokes("down enter");
    rig.settle();
    assert!(
        rig.cluster_picker().is_none(),
        "the cluster picker was replaced"
    );
    let picker = rig
        .namespace_picker()
        .expect("the namespace picker is open");
    let (state, names) = rig.vcx.update(|_, cx| {
        let delegate = &picker.read(cx).delegate;
        (delegate.state().clone(), delegate.names().to_vec())
    });
    assert_eq!(state, NamespaceList::Loaded);
    assert_eq!(names, ["prod", "staging"]);

    rig.vcx.simulate_input("prod");
    rig.settle();
    rig.vcx.simulate_keystrokes("enter");
    rig.settle();

    let dialog = rig.dialog().expect("the review is open");
    let (stage, line) = rig.vcx.update(|_, cx| {
        let dialog = dialog.read(cx);
        (dialog.stage(), dialog.preview().map(|p| p.label()))
    });
    assert_eq!(stage, CopyStage::Ready);
    assert_eq!(line.as_deref(), Some("ConfigMap prod/settings"));
    // Nothing is written before the button.
    assert_eq!(rig.real_writes("b"), 0);

    rig.vcx.update(|window, cx| window.draw(cx).clear(cx));
    for selector in [
        "copy-dialog",
        "copy-name",
        "copy-change",
        "copy-removed",
        "copy-confirm",
    ] {
        assert!(
            rig.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();

    assert_eq!(rig.real_writes("b"), 1);
    assert_eq!(rig.real_writes("a"), 0, "the source is only read");
    let stored = rig
        .cluster("b")
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings");
    let stored = stored.expect("the copy").to_value();
    assert_eq!(stored["metadata"]["namespace"], "prod");
    assert!(stored["metadata"].get("uid").is_none());
    assert!(rig.dialog().is_none(), "the dialog closed");
    let toasts = rig.toasts();
    assert!(
        toasts
            .iter()
            .any(|(level, text)| *level == ToastLevel::Success && text.contains("Created")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_read_only_target_is_refused_at_the_start_and_the_picker_stays_open(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.sessions
        .set_read_only(&id("b"), true)
        .expect("read-only");
    let source = rig.source(configmap());
    rig.begin(source);
    rig.vcx.simulate_keystrokes("down enter");
    rig.settle();
    assert!(rig.cluster_picker().is_some(), "still asking");
    assert!(rig.namespace_picker().is_none());
    let toasts = rig.toasts();
    assert!(
        toasts
            .iter()
            .any(|(level, text)| *level == ToastLevel::Warning && text.contains("read-only")),
        "{toasts:?}"
    );
    assert_eq!(rig.real_writes("b"), 0);
}

#[gpui::test]
fn a_cluster_scoped_kind_skips_the_namespace_and_asks_for_the_typed_name(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(json!({"apiVersion": "v1", "kind": "PersistentVolume",
        "metadata": {"name": "pv-1"}, "spec": {"capacity": {"storage": "1Gi"}}}));
    rig.begin(source);
    rig.vcx.simulate_keystrokes("down enter");
    rig.settle();
    assert!(rig.namespace_picker().is_none(), "no namespace to ask for");
    let dialog = rig.dialog().expect("straight to the review");
    let can = |rig: &mut Rig| rig.vcx.update(|_, cx| dialog.read(cx).can_copy(cx));
    assert_eq!(
        rig.vcx
            .update(|_, cx| dialog.read(cx).expected_name().map(str::to_owned))
            .as_deref(),
        Some("pv-1")
    );
    assert!(!can(&mut rig), "the typed name is missing");
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 0);
    rig.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.set_typed("pv-1", window, cx)));
    assert!(can(&mut rig));
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 1);
}

#[gpui::test]
fn escape_in_the_picker_sends_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(configmap());
    rig.begin(source);
    rig.vcx.simulate_keystrokes("escape");
    rig.settle();
    assert!(rig.cluster_picker().is_none());
    assert!(rig.state.audit_log().is_empty(), "no command, no record");
    assert_eq!(rig.real_writes("b"), 0);
}
