//! The guard's dry run in the runner's dialog (E19-S07): a previewed patch opens the dialog with
//! the object it would change drawn by the installed diff builder, and a conflict on the commit
//! offers "Preview again", which dry-runs the command anew against the live object.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, IntoElement, ParentElement as _, Render, TestAppContext, div};
use oxikube_app::PreviewChange;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ConflictDetails, ConflictReason, OxiError, Resource};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::{Fixture, bounds, fixture};
use crate::cluster::tests::fixture::{PROD, id};
use crate::modal::{ConfirmDialog, set_preview_diff_builder};
use crate::toast::ToastLevel;

fn nightly() -> ResourceRef {
    ResourceRef::namespaced(
        id(PROD),
        Gvk::new("batch", "v1", "CronJob"),
        "default",
        "nightly",
    )
}

fn cronjob(suspend: bool) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1",
        "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": "default", "resourceVersion": "4"},
        "spec": {"schedule": "0 3 * * *", "suspend": suspend}
    }))
    .expect("a valid CronJob")
}

fn suspend() -> Command {
    Command::CronJobSuspend { target: nightly() }
}

fn run(f: &mut Fixture, command: Command) {
    let runner = f.runner.clone();
    f.vcx.update(|window, cx| runner.run(command, window, cx));
    f.vcx.run_until_parked();
}

/// What the builder was handed: `(live spec.suspend, result spec.suspend)`.
type Seen = Arc<Mutex<Vec<(Value, Value)>>>;

/// A stand-in for the editor's diff view.
struct FakeDiff;

impl Render for FakeDiff {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().child("diff")
    }
}

fn install_builder(f: &mut Fixture) -> Seen {
    let seen: Seen = Arc::default();
    let record = seen.clone();
    f.vcx.update(|_, cx| {
        set_preview_diff_builder(
            Rc::new(move |change: &PreviewChange, _, cx| {
                record.lock().push((
                    change
                        .live
                        .as_deref()
                        .map_or(Value::Null, |l| l["spec"]["suspend"].clone()),
                    change.result["spec"]["suspend"].clone(),
                ));
                cx.new(|_| FakeDiff).into()
            }),
            cx,
        );
    });
    seen
}

fn dialog_headings(f: &mut Fixture) -> Option<Vec<String>> {
    let layer = f.vcx.update(|_, cx| f.ws.read(cx).modal_layer().clone());
    f.vcx.update(|_, cx| {
        let dialog = layer.read(cx).active_modal::<ConfirmDialog>()?;
        Some(
            dialog
                .read(cx)
                .preview_headings()
                .iter()
                .map(ToString::to_string)
                .collect(),
        )
    })
}

#[gpui::test]
fn a_previewed_patch_opens_the_dialog_with_its_diff(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.resources.insert(cronjob(false));
    let seen = install_builder(&mut f);
    run(&mut f, suspend());

    assert_eq!(
        dialog_headings(&mut f).expect("the guard asks"),
        ["Update CronJob default/nightly"]
    );
    assert_eq!(*seen.lock(), [(json!(false), json!(true))]);
    assert!(bounds(&mut f.vcx, "confirm-changes").is_some());
    assert!(bounds(&mut f.vcx, "confirm-change-0").is_some());
    assert!(
        f.resources.mutating_calls().iter().all(|c| c.is_dry_run()),
        "only the dry run before the answer"
    );

    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert_eq!(
        f.resources.objects()[0]
            .get("/spec/suspend")
            .map(|v| v.to_value()),
        Some(json!(true))
    );
}

#[gpui::test]
fn a_conflict_on_the_commit_offers_to_preview_again(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.resources.insert(cronjob(false));
    // The dry run passes; the object changes before the commit, which the server refuses.
    f.resources.script().patch.push_ok(cronjob(true));
    f.resources.script().patch.push_err(
        OxiError::conflict("the object has been modified").with_source(ConflictDetails {
            reason: ConflictReason::StaleVersion,
            causes: Vec::new(),
        }),
    );
    run(&mut f, suspend());
    f.vcx.simulate_keystrokes("enter");
    f.vcx.run_until_parked();
    assert!(dialog_headings(&mut f).is_none(), "the dialog closed");

    let layer = f.vcx.update(|_, cx| f.ws.read(cx).toast_layer().clone());
    let toast = f
        .vcx
        .update(|_, cx| layer.read(cx).visible())
        .pop()
        .expect("a toast");
    assert_eq!(toast.level, ToastLevel::Error);
    assert!(toast.message.contains("modified"), "{}", toast.message);
    let selector: &'static str =
        Box::leak(format!("toast-{}-action-0", toast.id.as_u64()).into_boxed_str());
    let button = bounds(&mut f.vcx, selector).expect("the Preview again button is drawn");
    f.vcx
        .simulate_click(button.center(), gpui::Modifiers::none());
    f.vcx.run_until_parked();

    assert_eq!(
        dialog_headings(&mut f).expect("a new question, with a new dry run"),
        ["Update CronJob default/nightly"]
    );
    assert!(
        f.resources.objects()[0]
            .get("/spec/suspend")
            .map(|v| v.to_value())
            == Some(json!(false)),
        "nothing was written"
    );
}
