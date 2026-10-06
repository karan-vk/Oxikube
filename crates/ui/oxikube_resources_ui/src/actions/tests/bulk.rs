//! Bulk delete from the table: one confirmation for the selection, a result per object.

use gpui::TestAppContext;
use oxikube_app::ItemStatus;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};

use super::{dialog, with_dialog};
use crate::actions::Stage;
use crate::table::tests::fixture::Fixture;
use crate::table::tests::p;

/// The platform's select-all chord (the keymaps bind `cmd-a` on macOS, `ctrl-a` elsewhere).
const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

#[gpui::test]
fn five_selected_objects_one_confirmation_per_object_results(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with((0..5).map(|i| p("x", &format!("web-{i}"), "1")));
    let table = f.open_pods();
    assert_eq!(f.names(&table).len(), 5);
    // Two requests per object (the rehearsal, then the delete), in selection order: web-1 is
    // forbidden, web-3 is gone already.
    let ports = f.ports();
    let script = ports.resources.script();
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-0 rehearsal
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-0
    script
        .delete
        .push_err(OxiError::forbidden("pods \"web-1\" is forbidden")); // web-1 rehearsal
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-2 rehearsal
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-2
    script
        .delete
        .push_err(OxiError::not_found("pods \"web-3\" not found")); // web-3 rehearsal
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-4 rehearsal
    script.delete.push_ok(oxikube_ports::DeleteOutcome::Deleted); // web-4

    f.keys(&table, SELECT_ALL);
    assert_eq!(f.selected(&table).len(), 5);
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("one dialog for the whole selection");
    with_dialog(&mut f, &d, |d, _, cx| {
        assert_eq!(d.plan().items().len(), 5);
        assert_eq!(d.plan().kinds(), [("Pod".into(), 5)]);
        assert!(d.can_confirm(cx));
        d.confirm(cx);
    });
    f.settle();

    let (stage, report) = d.read_with(&f.vcx, |d, _| (d.stage(), d.report().cloned()));
    assert_eq!(stage, Stage::Done);
    let report = report.expect("results");
    let statuses: Vec<_> = report.items.iter().map(|i| i.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            ItemStatus::Deleted,
            ItemStatus::Forbidden("pods \"web-1\" is forbidden".into()),
            ItemStatus::Deleted,
            ItemStatus::NotFound("pods \"web-3\" not found".into()),
            ItemStatus::Deleted,
        ]
    );
    assert_eq!(report.summary(), "Deleted 3 of 5 objects, 2 failed");

    // Five audit records, all from the UI.
    let audit = f.state.audit_log();
    assert_eq!(audit.len(), 5);
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
    assert_eq!(
        audit
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Succeeded)
            .count(),
        3
    );
    // The dialog stays up to show the list, with a Close button.
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(f.vcx.debug_bounds("delete-summary").is_some());
    assert!(f.vcx.debug_bounds("delete-results").is_some());
    assert!(f.vcx.debug_bounds("delete-close").is_some());
    with_dialog(&mut f, &d, |d, _, cx| d.cancel(cx));
    assert!(dialog(&mut f).is_none());
}

#[gpui::test]
fn a_right_click_inside_the_selection_deletes_the_selection(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with((0..4).map(|i| p("x", &format!("web-{i}"), "1")));
    let table = f.open_pods();
    f.keys(&table, SELECT_ALL);
    super::right_click(&mut f, 2);
    // Open, Copy Name, Select All, Delete: the per-kind action is for one object only.
    super::choose(&mut f, 3);
    let d = dialog(&mut f).expect("dialog");
    let items = d.read_with(&f.vcx, |d, _| d.plan().items().len());
    assert_eq!(items, 4);
}

#[gpui::test]
fn a_read_only_cluster_refuses_each_object_even_when_the_dialog_was_open(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with((0..3).map(|i| p("x", &format!("web-{i}"), "1")));
    let table = f.open_pods();
    f.keys(&table, SELECT_ALL);
    f.keys(&table, "delete");
    let d = dialog(&mut f).expect("dialog");
    // Read-only goes on while the dialog is open: the guard still refuses every object.
    f.sessions
        .set_read_only(&crate::table::tests::fixture::cluster(), true)
        .unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let report = d
        .read_with(&f.vcx, |d, _| d.report().cloned())
        .expect("results");
    assert!(
        report
            .items
            .iter()
            .all(|i| matches!(i.status, ItemStatus::Forbidden(_)))
    );
    assert!(f.ports().resources.mutating_calls().is_empty());
    assert!(
        f.state
            .audit_log()
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied)
    );
}
