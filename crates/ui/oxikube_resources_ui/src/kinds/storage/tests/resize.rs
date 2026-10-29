//! The resize dialog and the `pvc::Resize` it sends, through the real bus and guard.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::view::storage::ResizeBlock;
use oxikube_testkit::ResourceCall;
use oxikube_testkit::storage::{pvc, storage_class};
use serde_json::json;

use super::{kind, modal, storage_window, with};
use crate::kinds::storage::dialogs::{Loaded, ResizeDialog, ResizeStage};
use crate::table::tests::fixture::Fixture;

fn patches(f: &Fixture) -> Vec<(bool, serde_json::Value)> {
    f.ports()
        .resources
        .mutating_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => Some((options.dry_run, patch.body)),
            _ => None,
        })
        .collect()
}

/// A window with an expandable claim `shop/data` (10Gi) and a class that is not.
fn window(cx: &mut TestAppContext) -> Fixture {
    storage_window(
        cx,
        vec![
            pvc("data")
                .namespace("shop")
                .class("fast")
                .requests("10Gi")
                .build(),
            pvc("frozen")
                .namespace("shop")
                .class("slow")
                .requests("5Gi")
                .capacity("5Gi")
                .build(),
            storage_class("fast").expandable(true).build(),
            storage_class("slow").build(),
        ],
    )
}

/// Opens the claims table and presses `shift-r` on `row`.
fn press_resize(f: &mut Fixture, row: usize) -> gpui::Entity<ResizeDialog> {
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    for _ in 0..=row {
        f.keys(&table, "down");
    }
    f.keys(&table, "shift-r");
    f.settle();
    modal::<ResizeDialog>(f).expect("shift-r opens the resize dialog")
}

#[gpui::test]
fn shift_r_on_a_claim_opens_the_dialog_with_the_current_size_and_class(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_resize(&mut f, 0);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resize-dialog").is_some());
    assert!(f.vcx.debug_bounds("resize-input").is_some());
    with(&mut f, &dialog, |d, _, _| {
        assert_eq!(d.stage(), ResizeStage::Asking);
        let Loaded::Ready(context) = d.loaded_state() else {
            panic!("the claim was read: {:?}", d.loaded_state());
        };
        assert_eq!(context.claim.size_text(), "10Gi");
        assert_eq!(context.claim.storage_class.as_deref(), Some("fast"));
    });
    assert!(patches(&f).is_empty(), "opening sends nothing");
}

#[gpui::test]
fn the_dialog_blocks_a_shrink_and_the_same_size_and_says_why(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_resize(&mut f, 0);
    for size in ["5Gi", "10Gi", "10240Mi"] {
        with(&mut f, &dialog, |d, window, cx| {
            d.type_size(size, window, cx)
        });
        f.vcx.update(|window, cx| window.draw(cx).clear(cx));
        let (can, check) = f.vcx.update(|_, cx| {
            let d = dialog.read(cx);
            (d.can_confirm(cx), d.check(cx))
        });
        assert!(!can, "{size} does not grow the claim");
        assert!(
            matches!(check, Some(Err(ResizeBlock::NotLarger { .. }))),
            "{size}: {check:?}"
        );
        assert!(
            f.vcx.debug_bounds("resize-problem").is_some(),
            "{size} says why"
        );
    }
    with(&mut f, &dialog, |d, _, cx| d.confirm(cx));
    assert!(patches(&f).is_empty(), "a refused size sends nothing");
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn the_dialog_blocks_a_class_that_does_not_allow_expansion(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_resize(&mut f, 1); // `frozen`, in class `slow`
    with(&mut f, &dialog, |d, window, cx| {
        d.type_size("50Gi", window, cx)
    });
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (blocked, can) = f.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.blocked(), d.can_confirm(cx))
    });
    assert_eq!(
        blocked,
        Some(ResizeBlock::ExpansionDisallowed {
            class: "slow".into()
        })
    );
    assert!(!can, "even a large size cannot resize it");
    assert!(f.vcx.debug_bounds("resize-problem").is_some());
    with(&mut f, &dialog, |d, _, cx| d.confirm(cx));
    assert!(patches(&f).is_empty());
}

#[gpui::test]
fn a_valid_size_resizes_through_the_guard_and_audits_both_sizes(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let grown = pvc("data")
        .namespace("shop")
        .class("fast")
        .requests("20Gi")
        .condition("FileSystemResizePending", "True", "Waiting for the kubelet")
        .build();
    let resources = f.ports().resources;
    // The guard's dry run before it asks (E19-S07), the handler's, the patch.
    for _ in 0..3 {
        resources.script().patch.push_ok(grown.clone());
    }
    let dialog = press_resize(&mut f, 0);
    with(&mut f, &dialog, |d, window, cx| {
        d.type_size("20Gi", window, cx)
    });
    assert!(f.vcx.update(|_, cx| dialog.read(cx).can_confirm(cx)));
    with(&mut f, &dialog, |d, _, cx| d.confirm(cx));

    assert_eq!(
        patches(&f),
        [
            (
                true,
                json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}})
            ),
            (
                true,
                json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}})
            ),
            (
                false,
                json!({"spec": {"resources": {"requests": {"storage": "20Gi"}}}})
            ),
        ],
        "a rehearsal, then the patch"
    );
    let audit: Vec<_> = f
        .state
        .audit_log()
        .into_iter()
        .filter(|r| !r.dry_run)
        .collect();
    assert_eq!(audit.len(), 1);
    assert_eq!(&*audit[0].cmd, "pvc::Resize");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].detail.as_deref(), Some("from=10Gi to=20Gi"));
    // The pending file-system expansion is shown afterwards.
    with(&mut f, &dialog, |d, _, _| {
        assert_eq!(d.stage(), ResizeStage::Done);
        let report = d.report().expect("the report");
        assert_eq!(&*report.conditions[0].kind, "FileSystemResizePending");
    });
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("resize-condition-0").is_some());
    assert!(f.vcx.debug_bounds("resize-summary").is_some());
}

#[gpui::test]
fn cancel_sends_nothing(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let dialog = press_resize(&mut f, 0);
    with(&mut f, &dialog, |d, window, cx| {
        d.type_size("20Gi", window, cx);
        d.cancel(cx);
    });
    assert!(modal::<ResizeDialog>(&mut f).is_none());
    assert!(patches(&f).is_empty());
    assert!(f.state.audit_log().is_empty());
}

#[gpui::test]
fn a_read_only_cluster_offers_no_resize_and_the_key_says_why(cx: &mut TestAppContext) {
    let mut f = window(cx);
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .expect("open");
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    f.keys(&table, "down");
    f.keys(&table, "shift-r");
    assert!(
        modal::<ResizeDialog>(&mut f).is_none(),
        "no dialog on a read-only cluster"
    );
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let resize = entries
        .iter()
        .find(|e| e.command() == oxikube_domain::command::CommandId::PVC_RESIZE)
        .expect("the action is listed");
    assert!(!resize.is_enabled());
    assert!(resize.reason().unwrap().contains("read-only"));
    assert!(patches(&f).is_empty());
}

#[gpui::test]
fn the_key_on_another_kind_says_which_kind_has_it(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let table = f.open(kind("storageclasses"));
    f.settle();
    f.keys(&table, "down");
    f.keys(&table, "shift-r");
    assert!(modal::<ResizeDialog>(&mut f).is_none());
}

#[gpui::test]
fn the_action_is_in_the_menu_and_the_palette_list_of_a_claim(cx: &mut TestAppContext) {
    let mut f = window(cx);
    let table = f.open(kind("persistentvolumeclaims"));
    f.settle();
    f.keys(&table, "down");
    let own = f.vcx.update(|_, cx| {
        oxikube_workspace::command_surface::CommandSurface::own_commands(table.read(cx), cx)
    });
    assert!(
        own.contains(&oxikube_domain::command::CommandId::PVC_RESIZE),
        "{own:?}"
    );
    assert!(!own.contains(&oxikube_domain::command::CommandId::STORAGECLASS_SET_DEFAULT));
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    assert!(labels.contains(&"Resize…"), "{labels:?}");
}
