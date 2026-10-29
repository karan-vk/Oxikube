//! The guard's dry run in the confirmation dialog (E19-S07): `oxikube_editor::init` installs the
//! E10 diff view in the workspace's `ConfirmDialog`, which draws each object the server would
//! write; the diff ignores server-owned fields and never shows a Secret value; the banner and
//! warning variants; and a 1 MB object is diffed off the UI thread, its frames inside the budget.

use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Draws one frame after `scroll` and returns how long it took.
fn frame(vcx: &mut VisualTestContext, scroll: impl FnOnce(&mut VisualTestContext)) -> Duration {
    scroll(vcx);
    let started = Instant::now();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    started.elapsed()
}

#[gpui::test]
fn a_1mb_object_is_diffed_off_the_ui_thread_and_its_frames_fit_the_budget(cx: &mut TestAppContext) {
    const MB: usize = 1 << 20;
    let (live, mut result) = (config_map("green", "7", MB), config_map("green", "8", MB));
    // Changes spread through the object: the first key, one in the middle, the last.
    let keys: Vec<String> = live["data"].as_object().unwrap().keys().cloned().collect();
    for key in [&keys[1], &keys[keys.len() / 2], &keys[keys.len() - 1]] {
        result["data"][key] = json!("changed");
    }
    assert!(serde_json::to_vec(&live).unwrap().len() >= MB);

    // The diff itself, timed as the background executor runs it.
    let started = Instant::now();
    let diff = oxikube_editor::diff::ManifestDiff::of_manifests(Some(&live), &result);
    let background = started.elapsed();
    assert_eq!(diff.hunks(), 3);

    let prompt = ConfirmPrompt::new("Apply?", "Apply ConfigMap shop/web on kind-oxikube")
        .with_changes(vec![change("ConfigMap", live, result)], 0);
    let (_, mut vcx) = open_dialog(cx, prompt);
    assert!(drawn(&mut vcx, "dry-run-diff"), "the diff arrived");

    // 120 frames of the open dialog: each a redraw while the user moves the mouse or types in
    // another field, the rows built for the screen only.
    let mut frames: Vec<Duration> = (0..120).map(|_| frame(&mut vcx, |_| {})).collect();
    frames.sort();
    let p95 = frames[frames.len() * 95 / 100];
    let max = frames[frames.len() - 1];
    eprintln!(
        "dry-run diff of a {} KiB object: background diff {:.1} ms; dialog frames p95 {:.2} ms, \
         max {:.2} ms",
        MB / 1024,
        background.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3,
    );
    // The frame budget (docs/PERFORMANCE.md: p95 <= 8 ms). Debug builds are several times slower
    // than the app, so they only have to stay under one 60 Hz frame.
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(16)
    } else {
        Duration::from_millis(8)
    };
    assert!(p95 <= budget, "p95 frame {p95:?} over {budget:?}");
}
