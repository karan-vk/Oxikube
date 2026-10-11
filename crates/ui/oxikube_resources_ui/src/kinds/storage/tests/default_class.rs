//! The set-default dialog and the `storageclass::SetDefault` it sends.

use gpui::TestAppContext;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_testkit::ResourceCall;
use oxikube_testkit::storage::storage_class;
use serde_json::json;

use super::{kind, modal, storage_window, with};
use crate::kinds::storage::dialogs::{ClassLoaded, ClassStage, DefaultClassDialog};
use crate::table::tests::fixture::Fixture;

const DEFAULT: &str = "storageclass.kubernetes.io/is-default-class";

fn patches(f: &Fixture) -> Vec<(String, bool, serde_json::Value)> {
    f.ports()
        .resources
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch {
                name,
                patch,
                options,
                ..
            } => Some((
                name,
                options.dry_run,
                patch.body["metadata"]["annotations"].clone(),
            )),
            _ => None,
        })
        .collect()
}

/// Opens the classes table (rows: fast, old, slow) and presses `shift-s` on `row`.
fn press_default(f: &mut Fixture, row: usize) -> gpui::Entity<DefaultClassDialog> {
    let table = f.open(kind("storageclasses"));
    f.settle();
    for _ in 0..=row {
        f.keys(&table, "down");
    }
    f.keys(&table, "shift-s");
    f.settle();
    modal::<DefaultClassDialog>(f).expect("shift-s opens the dialog")
}

fn window(cx: &mut TestAppContext) -> Fixture {
    storage_window(
        cx,
        vec![
            storage_class("fast").build(),
            storage_class("old").default_class().build(),
            storage_class("slow").build(),
        ],
    )
}

#[gpui::test]
fn the_dialog_names_the_current_default_and_offers_to_clear_it(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_default(&mut f, 0); // fast
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("default-class-dialog").is_some());
    assert!(f.vcx.debug_bounds("default-now").is_some());
    assert!(f.vcx.debug_bounds("default-choice").is_some());
    with(&mut f, &dialog, |d, _, _| {
        let ClassLoaded::Ready(context) = d.loaded() else {
            panic!("classes read: {:?}", d.loaded());
        };
        assert_eq!(context.previous.len(), 1);
        assert_eq!(&*context.previous[0], "old");
        assert!(d.clears_previous(), "on by default");
        assert!(d.can_confirm());
    });
    assert!(patches(&f).is_empty());
}

#[gpui::test]
fn confirming_sets_the_annotation_and_clears_the_previous_default(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let resources = f.ports().resources;
    for _ in 0..4 {
        resources
            .script()
            .patch
            .push_ok(storage_class("fast").default_class().build());
    }
    let dialog = press_default(&mut f, 0);
    with(&mut f, &dialog, |d, _, cx| d.confirm(cx));
    assert_eq!(
        patches(&f),
        [
            ("fast".to_owned(), true, json!({DEFAULT: "true"})),
            ("old".to_owned(), true, json!({DEFAULT: "false"})),
            ("fast".to_owned(), false, json!({DEFAULT: "true"})),
            ("old".to_owned(), false, json!({DEFAULT: "false"})),
        ]
    );
    let audit = f.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(&*audit[0].cmd, "storageclass::SetDefault");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert!(
        modal::<DefaultClassDialog>(&mut f).is_none(),
        "closed with a toast"
    );
}

#[gpui::test]
fn keeping_the_previous_default_leaves_its_annotation_alone(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let resources = f.ports().resources;
    for _ in 0..2 {
        resources
            .script()
            .patch
            .push_ok(storage_class("fast").default_class().build());
    }
    let dialog = press_default(&mut f, 0);
    with(&mut f, &dialog, |d, _, cx| {
        d.set_clear_previous(false, cx);
        d.confirm(cx);
    });
    let names: Vec<String> = patches(&f).into_iter().map(|(n, ..)| n).collect();
    assert_eq!(names, ["fast", "fast"]);
}

#[gpui::test]
fn the_only_default_has_nothing_to_set(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_default(&mut f, 1); // old, already the default
    with(&mut f, &dialog, |d, _, cx| {
        assert!(!d.can_confirm());
        d.confirm(cx);
        assert_eq!(d.stage(), ClassStage::Asking);
    });
    assert!(patches(&f).is_empty());
}

#[gpui::test]
fn a_read_only_cluster_disables_the_action_with_the_reason(cx: &mut TestAppContext) {
    let mut f = window(cx);
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .expect("open");
    let table = f.open(kind("storageclasses"));
    f.settle();
    f.keys(&table, "down");
    f.keys(&table, "shift-s");
    assert!(modal::<DefaultClassDialog>(&mut f).is_none());
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let entry = entries
        .iter()
        .find(|e| e.command() == CommandId::STORAGECLASS_SET_DEFAULT)
        .expect("listed");
    assert!(!entry.is_enabled());
    assert!(patches(&f).is_empty());
}
