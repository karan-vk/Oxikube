//! The guard's dry run in the confirmation dialog (E19-S07): `oxikube_editor::init` installs the
//! E10 diff view in the workspace's `ConfirmDialog`, which draws each object the server would
//! write; the diff ignores server-owned fields and never shows a Secret value; the banner and
//! warning variants; and a 1 MB object is diffed on the background executor while the dialog is
//! drawn (its frame times: the `dry_run_bench` example).

use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use oxikube_app::PreviewChange;
use oxikube_app::apply::Change;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_editor::diff::LineKind;
use oxikube_editor::view::DryRunDiff;
use oxikube_ui::root::Root;
use oxikube_workspace::{ConfirmDialog, ConfirmPrompt, PreviewNote};
use serde_json::{Value, json};

fn target(kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube")),
        Gvk::new("", "v1", kind),
        "shop",
        name,
    )
}

fn change(kind: &str, live: Value, result: Value) -> PreviewChange {
    PreviewChange {
        target: target(kind, "web"),
        change: Change::Update,
        live: Some(Arc::new(live)),
        result: Arc::new(result),
    }
}

fn config_map(mode: &str, version: &str, size: usize) -> Value {
    let mut data = serde_json::Map::new();
    data.insert("mode".into(), json!(mode));
    // `size` bytes of other keys, so the object is as large as a test needs.
    let mut i = 0;
    let mut bytes = 0;
    while bytes < size {
        let value = format!("value-{i:08}-{}", "x".repeat(48));
        bytes += value.len() + 16;
        data.insert(format!("key-{i:08}"), json!(value));
        i += 1;
    }
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": "web",
            "namespace": "shop",
            "resourceVersion": version,
            "managedFields": [{"manager": "kubectl", "operation": "Update", "time": version}]
        },
        "data": data
    })
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
    });
}

/// A window whose root is a confirmation dialog for `prompt`.
fn open_dialog(
    cx: &mut TestAppContext,
    prompt: ConfirmPrompt,
) -> (Entity<ConfirmDialog>, VisualTestContext) {
    init(cx);
    let mut dialog = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| ConfirmDialog::new(prompt, |_, _, _| {}, |_| {}, window, cx));
        dialog = Some(view.clone());
        Root::new(view, window, cx)
    });
    let vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    (dialog.expect("built"), vcx)
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.debug_bounds(selector).is_some()
}

/// The lines of `diff` that changed, as `(kind, text)`.
fn changed_lines(diff: &oxikube_editor::diff::ManifestDiff) -> Vec<(LineKind, String)> {
    diff.inline_rows()
        .iter()
        .filter_map(|row| match *row {
            oxikube_editor::diff::InlineRow::Line { kind, old, new } => match (kind, old, new) {
                (LineKind::Removed, Some(n), _) => Some((kind, diff.old_line(n).to_owned())),
                (LineKind::Added, _, Some(n)) => Some((kind, diff.new_line(n).to_owned())),
                _ => None,
            },
            oxikube_editor::diff::InlineRow::Gap(_) => None,
        })
        .collect()
}

#[gpui::test]
fn the_dialog_draws_each_previewed_object_with_the_diff_view(cx: &mut TestAppContext) {
    let prompt = ConfirmPrompt::new("Suspend it?", "Suspend CronJob shop/web on kind-oxikube")
        .with_preview(Some(PreviewNote::Rehearsed(
            "Dry run: ConfigMap shop/web can be patched".into(),
        )))
        .with_changes(
            vec![change(
                "ConfigMap",
                config_map("green", "7", 0),
                config_map("blue", "8", 0),
            )],
            2,
        )
        .with_warnings(vec!["unknown field \"spec.foo\"".into()]);
    let (dialog, mut vcx) = open_dialog(cx, prompt);
    assert_eq!(
        vcx.read(|cx| dialog.read(cx).preview_headings()),
        ["Update ConfigMap shop/web"]
    );
    for selector in [
        "confirm-preview",
        "confirm-changes",
        "confirm-change-0",
        "dry-run-diff",
        "dry-run-diff-summary",
        "confirm-changes-omitted",
        "confirm-warning-0",
    ] {
        assert!(drawn(&mut vcx, selector), "{selector} is drawn");
    }
}

#[gpui::test]
fn the_diff_ignores_server_fields_and_never_shows_a_secret_value(cx: &mut TestAppContext) {
    init(cx);
    // What the guard hands over: the values are its keyed fingerprints already.
    let secret = |password: &str, version: &str| {
        json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "web", "namespace": "shop", "resourceVersion": version, "uid": "u-1"},
            "data": {"user": "(hidden 0a0a0a0a)", "password": password},
            "status": {"ignored": true}
        })
    };
    let change = change(
        "Secret",
        secret("(hidden 11111111)", "7"),
        secret("(hidden 22222222)", "8"),
    );
    let window = cx.add_window(|_, cx| DryRunDiff::new(&change, cx));
    let view = window.root(cx).expect("the view");
    cx.run_until_parked();
    let lines = cx.read(|cx| changed_lines(view.read(cx).diff().expect("computed")));
    assert_eq!(lines.len(), 2, "only the password changes: {lines:?}");
    for (_, text) in &lines {
        assert!(text.contains("password"), "{text}");
        assert!(!text.contains("resourceVersion") && !text.contains("status"));
    }
}

#[gpui::test]
fn the_banner_variants_are_drawn(cx: &mut TestAppContext) {
    let prompt = ConfirmPrompt::new("Suspend it?", "Suspend CronJob shop/web on kind-oxikube")
        .with_preview(Some(PreviewNote::Unavailable(
            "dryRun is not supported by the aggregated API metrics.example.com".into(),
        )));
    let (dialog, mut vcx) = open_dialog(cx, prompt);
    assert!(drawn(&mut vcx, "confirm-preview"), "the banner");
    assert!(!drawn(&mut vcx, "confirm-changes"), "nothing to compare");
    assert!(
        vcx.read(|cx| dialog.read(cx).can_confirm(cx)),
        "a missing preview never blocks"
    );
}

/// A ConfigMap of at least 1 MB against its dry-run result, three keys changed through it: the
/// first, one in the middle, the last.
fn one_mb_change() -> PreviewChange {
    const MB: usize = 1 << 20;
    let (live, mut result) = (config_map("green", "7", MB), config_map("green", "8", MB));
    let keys: Vec<String> = live["data"]
        .as_object()
        .expect("a data map")
        .keys()
        .cloned()
        .collect();
    for key in [&keys[1], &keys[keys.len() / 2], &keys[keys.len() - 1]] {
        result["data"][key] = json!("changed");
    }
    assert!(
        serde_json::to_vec(&live).expect("serialises").len() >= MB,
        "the live object is at least 1 MB"
    );
    change("ConfigMap", live, result)
}

/// The diff of a 1 MB object is computed by a background task: nothing while the view is built or
/// drawn, nothing while only UI-thread tasks have run, and the result reaches the view on the UI
/// thread after the background work. Frame times are measured by the `dry_run_bench` example.
#[gpui::test]
fn a_1mb_object_is_diffed_on_the_background_executor(cx: &mut TestAppContext) {
    let change = one_mb_change();
    init(cx);
    cx.run_until_parked();
    let window = cx.add_window(|_, cx| DryRunDiff::new(&change, cx));
    let view = window.root(cx).expect("the view");
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    let pending = |vcx: &mut VisualTestContext| vcx.read(|cx| view.read(cx).diff().is_none());
    assert!(
        pending(&mut vcx),
        "nothing is diffed while the view is built"
    );
    assert!(
        drawn(&mut vcx, "dry-run-diff-pending") && pending(&mut vcx),
        "a frame drawn while the diff is in flight shows the placeholder"
    );

    let dispatcher = vcx.executor().dispatcher().clone();
    let test = dispatcher.as_test().expect("the test dispatcher");
    let mut background_runs = 0;
    while pending(&mut vcx) {
        let (_, background) = test.scheduler().pending_task_counts();
        if background > 0 {
            // Only the background work runs: the view must not have its result until the UI
            // thread takes it.
            while test.tick(true) {}
            background_runs += 1;
            assert!(
                pending(&mut vcx),
                "the result reaches the view on the UI thread, after the background work"
            );
        } else {
            assert!(
                test.tick(false),
                "the diff arrives before the executor parks"
            );
        }
    }
    assert!(
        background_runs > 0,
        "the diff was computed without a background task: on the UI thread"
    );
    let hunks = vcx.read(|cx| view.read(cx).diff().map(|diff| diff.hunks()));
    assert_eq!(hunks, Some(3));
}

#[gpui::test]
fn the_dialog_opens_before_the_diff_of_a_1mb_object_arrives(cx: &mut TestAppContext) {
    let prompt = ConfirmPrompt::new("Apply?", "Apply ConfigMap shop/web on kind-oxikube")
        .with_changes(vec![one_mb_change()], 0);
    init(cx);
    cx.run_until_parked();
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| ConfirmDialog::new(prompt, |_, _, _| {}, |_| {}, window, cx));
        Root::new(view, window, cx)
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    assert!(drawn(&mut vcx, "confirm-change-0"), "the dialog is drawn");
    assert!(
        drawn(&mut vcx, "dry-run-diff-pending"),
        "with the diff pending"
    );
    assert!(!drawn(&mut vcx, "dry-run-diff"), "no diff yet");
    vcx.run_until_parked();
    assert!(drawn(&mut vcx, "dry-run-diff"), "the diff arrived");
    assert!(
        !drawn(&mut vcx, "dry-run-diff-pending"),
        "the placeholder is gone"
    );
}
