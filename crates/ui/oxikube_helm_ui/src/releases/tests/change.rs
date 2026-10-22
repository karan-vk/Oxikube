//! Upgrade, rollback and uninstall from the detail: the header's upgrade, the history row, its
//! values preview, the header's uninstall with keep-history, and what the gate hides. The view only sends commands; the
//! guard's behaviour is tested in `oxikube_app::helm_ops`.

use gpui::TestAppContext;
use oxikube_app::helm_ops::DiffKind;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, Values};
use serde_json::json;

use super::tabs::web;
use super::{Harness, cluster, full};
use crate::releases::Preview;

fn open(cx: &mut TestAppContext) -> Harness {
    let mut data = web();
    // Revision 2 ran with fewer replicas and an older image than revision 3 (the latest).
    data.values_at.insert(
        ("web".into(), 2),
        Values::from_json(json!({"replicaCount": 1, "auth": {"adminPassword": "hunter2"}})),
    );
    let mut h = Harness::open(cx, data, full());
    h.select(0);
    h.click("helm-release-tab:history");
    h
}

#[gpui::test]
fn a_history_row_previews_the_values_it_would_restore(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert!(h.is_laid_out("helm-release-rollback-row:2"));
    assert!(h.is_laid_out("helm-release-rollback-row:1"));
    assert!(
        !h.is_laid_out("helm-release-rollback-row:3"),
        "the newest revision is what runs now"
    );
    assert!(!h.is_laid_out("helm-release-rollback-pane"));

    h.click("helm-release-revision:2");
    assert!(h.is_laid_out("helm-release-rollback-pane"));
    assert!(h.is_laid_out("helm-release-rollback"));
    let (revision, preview) = h.read(|v, _| {
        let (revision, preview) = v.preview().expect("a revision is selected");
        (revision, preview.clone())
    });
    assert_eq!(revision, 2);
    let Preview::Ready(lines) = preview else {
        panic!("not ready: {preview:?}")
    };
    let changed: Vec<_> = lines
        .iter()
        .filter(|l| l.kind != DiffKind::Same)
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert_eq!(
        changed,
        [
            (DiffKind::Removed, "replicaCount: 3"),
            (DiffKind::Added, "replicaCount: 1"),
        ]
    );
    assert!(h.is_laid_out("helm-release-preview-diff"));
    // The values are the Values tab's: credentials stay masked in the preview.
    let shown = format!("{lines:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[gpui::test]
fn the_rollback_button_sends_helm_rollback_for_the_selected_revision(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-rollback-row:1");
    assert!(
        h.bus.sent().is_empty(),
        "picking a revision changes nothing"
    );
    h.click("helm-release-rollback");
    assert_eq!(
        h.bus.sent(),
        [Command::HelmRollback {
            cluster: cluster(),
            namespace: "apps".into(),
            release: "web".into(),
            revision: 1,
        }]
    );
}

#[gpui::test]
fn the_same_values_are_said_so_and_cancel_clears_the_selection(cx: &mut TestAppContext) {
    let mut h = open(cx);
    // Revision 1 has no override: it reads the same values as now.
    h.click("helm-release-revision:1");
    assert!(h.is_laid_out("helm-release-preview-same"));
    h.click("helm-release-rollback-cancel");
    assert!(!h.is_laid_out("helm-release-rollback-pane"));
    assert!(h.read(|v, _| v.preview().is_none()));
    assert!(h.bus.sent().is_empty());
}

#[gpui::test]
fn the_header_uninstalls_and_keep_history_is_part_of_the_command(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert!(h.is_laid_out("helm-release-uninstall"));
    h.click("helm-release-uninstall");
    h.click("helm-release-keep-history");
    assert!(h.read(|v, _| v.keeps_history()));
    h.click("helm-release-uninstall");
    let sent = h.bus.sent();
    let uninstall = |keep_history| Command::HelmUninstall {
        cluster: cluster(),
        namespace: "apps".into(),
        release: "web".into(),
        keep_history,
    };
    assert_eq!(sent, [uninstall(false), uninstall(true)]);
}

#[gpui::test]
fn the_header_opens_the_upgrade_tab_for_the_open_release(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert!(h.is_laid_out("helm-release-upgrade"));
    h.click("helm-release-upgrade");
    assert_eq!(
        h.bus.sent(),
        [Command::HelmUpgradeRelease {
            namespace: Some("apps".into()),
            release: Some("web".into()),
        }]
    );
}

#[gpui::test]
fn without_helm_there_is_nothing_to_roll_back_or_uninstall_with(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, web(), HelmCapabilities::none());
    h.select(0);
    h.click("helm-release-tab:history");
    assert!(h.is_laid_out("helm-release-revision:2"));
    assert!(!h.is_laid_out("helm-release-rollback-row:2"));
    assert!(!h.is_laid_out("helm-release-uninstall"));
    assert!(!h.is_laid_out("helm-release-upgrade"));
    assert!(h.is_laid_out("helm-release-read-only"));
    // Even a stray call selects nothing and sends nothing.
    let view = h.view.clone();
    h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.select_revision(2, cx);
            v.rollback_selected(cx);
            v.uninstall(cx);
            v.upgrade(cx);
        })
    });
    assert!(h.bus.sent().is_empty());
    assert!(h.read(|v, _| v.preview().is_none()));
}

#[gpui::test]
fn the_selection_goes_when_the_release_moves_to_a_new_revision(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-revision:2");
    let view = h.view.clone();
    h.vcx.update(|_, cx| {
        view.update(cx, |v, _| {
            v.detail.as_mut().expect("open").invalidate();
        })
    });
    assert!(h.read(|v, _| v.preview().is_none()));
}
