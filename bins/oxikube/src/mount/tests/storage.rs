//! The Storage section on the real init path (E12-S15): the kinds are in the cluster tab's
//! sidebar, the claims table opens from it with the Pods (used by) column, `shift-r` opens the
//! resize dialog, and the resize reaches the cluster through the app's bus and guard (a dry run
//! first, then the patch, one audit record with both sizes); a read-only cluster refuses.

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_resources_ui::kinds::storage::dialogs::{ResizeDialog, ResizeStage};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::storage::{pod_mounting, pvc, storage_class};
use oxikube_testkit::{ResourceCall, TestPorts, discovery_sets};
use oxikube_workspace::sidebar::SidebarPanel;

use super::App;
use crate::app_state::AppState;

impl App {
    /// A connected cluster that serves the snapshot CRDs and holds a claim `shop/data` (10Gi, a
    /// class that allows expansion) with two Pods mounting it, its table open with the cursor on
    /// the first row.
    fn open_claims_table(&mut self) -> gpui::Entity<ResourceTable> {
        self.serve(discovery_sets::core_with_snapshots());
        let ports = self.ports.connector.ports_for(&TestPorts::cluster_id());
        for object in [
            pvc("data")
                .namespace("shop")
                .class("fast")
                .requests("10Gi")
                .build(),
            storage_class("fast")
                .expandable(true)
                .default_class()
                .build(),
            pod_mounting("web-0", "shop", &["data"]),
            pod_mounting("web-1", "shop", &["data"]),
        ] {
            ports.resources.insert(object);
        }
        self.press("enter");
        self.tick();
        // The sidebar's Storage entry for claims (the rows below the fold are not drawn in a
        // test window, so the entry is activated as a click on it does).
        let ws = self.tab_workspace();
        let panel = self
            .vcx
            .update(|_, cx| ws.read(cx).panel::<SidebarPanel>().expect("the sidebar"));
        self.vcx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.activate("storage/persistentvolumeclaims", cx)
            })
        });
        self.tick();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press("down");
        table
    }

    fn resize_dialog(&mut self) -> Option<gpui::Entity<ResizeDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<ResizeDialog>()
        })
    }
}

#[gpui::test]
fn the_storage_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::PVC_RESIZE, "k8s.pvc_resize"),
        (
            CommandId::STORAGECLASS_SET_DEFAULT,
            "k8s.storageclass_set_default",
        ),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::storage"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    assert!(offered.contains(&CommandId::PVC_RESIZE));
    assert!(offered.contains(&CommandId::STORAGECLASS_SET_DEFAULT));
}

#[gpui::test]
fn the_storage_section_opens_the_claims_table_and_shift_r_resizes_through_the_guard(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let table = app.open_claims_table();
    // The Storage section's kinds are in the sidebar; the claims table lists the Pods per claim.
    let ws = app.tab_workspace();
    let rows: Vec<String> = app.vcx.update(|_, cx| {
        let panel = ws.read(cx).panel::<SidebarPanel>().expect("the sidebar");
        panel
            .read(cx)
            .rows()
            .iter()
            .map(|r| r.id().to_owned())
            .collect()
    });
    for entry in [
        "storage/persistentvolumeclaims",
        "storage/persistentvolumes",
        "storage/storageclasses",
        "storage/volumesnapshots",
        "storage/volumesnapshotclasses",
    ] {
        assert!(rows.contains(&entry.to_owned()), "{entry} in {rows:?}");
    }
    let pods = app.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let row = d.rows().first().expect("the claim").clone();
            d.provider()
                .cell(
                    &row,
                    &oxikube_app::ColumnId::new("pods"),
                    jiff::Timestamp::now(),
                )
                .display()
                .to_owned()
        })
    });
    assert_eq!(pods, "2", "two Pods mount the claim");

    let ports = app.ports.connector.ports_for(&TestPorts::cluster_id());
    let grown = pvc("data")
        .namespace("shop")
        .class("fast")
        .requests("20Gi")
        .build();
    ports.resources.script().patch.push_ok(grown.clone());
    ports.resources.script().patch.push_ok(grown);
    app.press("shift-r");
    app.tick();
    let dialog = app
        .resize_dialog()
        .expect("shift-r opens the resize dialog");
    app.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| d.type_size("5Gi", window, cx));
    });
    assert!(
        !app.vcx.update(|_, cx| dialog.read(cx).can_confirm(cx)),
        "a shrink is refused"
    );
    app.vcx.update(|window, cx| {
        dialog.update(cx, |d, cx| {
            d.type_size("20Gi", window, cx);
            d.confirm(cx);
        });
    });
    app.tick();
    // The resize has no conditions to show, so the dialog closed with a toast.
    assert_ne!(
        app.vcx.update(|_, cx| dialog.read(cx).stage()),
        ResizeStage::Asking,
        "the resize ran"
    );
    let patches: Vec<bool> = ports
        .resources
        .mutating_calls()
        .into_iter()
        .filter_map(|c| match c {
            ResourceCall::Patch { options, .. } => Some(options.dry_run),
            _ => None,
        })
        .collect();
    // The guard's dry run before it asks (E19-S07), the handler's rehearsal, the patch.
    assert_eq!(patches, [true, true, false], "a rehearsal, then the patch");
    let audit: Vec<_> = app
        .ports
        .state
        .audit_log()
        .into_iter()
        .filter(|r| !r.dry_run)
        .collect();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "pvc::Resize");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].detail.as_deref(), Some("from=10Gi to=20Gi"));
}

#[gpui::test]
fn a_read_only_cluster_offers_no_resize_dialog(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let _table = app.open_claims_table();
    let cluster = TestPorts::cluster_id();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");
    app.tick();
    app.press("shift-r");
    app.tick();
    assert!(
        app.resize_dialog().is_none(),
        "no dialog on a read-only cluster"
    );
    app.expect_toast("This cluster is read-only");
    let ports = app.ports.connector.ports_for(&cluster);
    assert!(ports.resources.mutating_calls().is_empty());
}
