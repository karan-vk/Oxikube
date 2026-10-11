//! The storage kinds in a real window over testkit fakes: the sidebar's Storage section, the
//! tables' columns, the detail panels, and the two actions through the real bus and guard.
//!
//! * `reach`: the kinds are in the sidebar (snapshots only where discovery serves them), open as
//!   tables and as details
//! * `columns`: what each table lists, and the claims' Pods column following the Pods feed
//! * `panel`: the detail panels
//! * `resize`: the resize dialog and the `pvc::Resize` it sends
//! * `default_class`: the set-default dialog and the `storageclass::SetDefault` it sends

mod columns;
mod default_class;
mod panel;
mod reach;
mod resize;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_app::ColumnId;
use oxikube_domain::Resource;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::ResourceKind;
use oxikube_testkit::discovery_sets;
use oxikube_workspace::modal::ModalView;

use crate::detail::DetailView;
use crate::table::ResourceTable;
use crate::table::tests::fixture::{Fixture, cluster};

/// A window with the real bus and the storage actions, the built-in kinds registered, and a
/// connected cluster that serves the snapshot CRDs and holds `objects`.
pub(super) fn storage_window(cx: &mut TestAppContext, objects: Vec<Resource>) -> Fixture {
    let mut f = Fixture::with_storage(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    connect(&mut f, discovery_sets::core_with_snapshots(), objects);
    f
}

/// Serves `kinds`, stores `objects` and connects.
pub(super) fn connect(f: &mut Fixture, kinds: Vec<ResourceKind>, objects: Vec<Resource>) {
    let ports = f.ports();
    ports.discovery.set_kinds(kinds);
    for object in objects {
        ports.resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
}

/// The kind with `plural` as the snapshot-serving cluster reports it.
pub(super) fn kind(plural: &str) -> ResourceKind {
    discovery_sets::core_with_snapshots()
        .into_iter()
        .find(|k| k.plural == plural)
        .unwrap_or_else(|| panic!("no kind {plural}"))
}

/// The text of the cell of the row named `name` in `column` of `table`.
pub(super) fn cell(
    f: &mut Fixture,
    table: &Entity<ResourceTable>,
    name: &str,
    column: &str,
) -> String {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            let row = d
                .rows()
                .iter()
                .find(|row| row.name() == name)
                .unwrap_or_else(|| panic!("no row {name}"));
            d.provider()
                .cell(row, &ColumnId::new(column), jiff::Timestamp::now())
                .display()
                .to_owned()
        })
    })
}

/// The visible column ids of `table`, in order.
pub(super) fn visible(f: &mut Fixture, table: &Entity<ResourceTable>) -> Vec<String> {
    f.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .visible_ids()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    })
}

/// The modal of type `T` open in the cluster tab's workspace, if any.
pub(super) fn modal<T: ModalView>(f: &mut Fixture) -> Option<Entity<T>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<T>()
    })
}

/// Opens the detail of `target`, as `resource::Open` does.
pub(super) fn open_detail(f: &mut Fixture, target: &ResourceRef) -> Entity<DetailView> {
    let views = f.views.clone();
    let target = target.clone();
    let view = f
        .vcx
        .update(|window, cx| views.update(cx, |views, cx| views.open_detail(&target, window, cx)))
        .expect("the cluster has a tab");
    f.settle();
    f.settle();
    view
}

/// Runs `run` on the modal `dialog`, then lets effects land.
pub(super) fn with<T: 'static, R>(
    f: &mut Fixture,
    dialog: &Entity<T>,
    run: impl FnOnce(&mut T, &mut gpui::Window, &mut gpui::Context<T>) -> R,
) -> R {
    let dialog = dialog.clone();
    let out = f
        .vcx
        .update(|window, cx| dialog.update(cx, |dialog, cx| run(dialog, window, cx)));
    f.settle();
    out
}

/// Lets `secs` of virtual time pass (the claim panel looks for the mounting Pods on a timer).
pub(super) fn wait(f: &mut Fixture, secs: u64) {
    f.vcx
        .executor()
        .advance_clock(std::time::Duration::from_secs(secs));
    f.settle();
}
