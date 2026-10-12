//! The app side: what a user does to reach a kind, and the bus.

use gpui::Entity;
use oxikube::app_state::AppState;
use oxikube_app::command_bus::{DispatchContext, DispatchError, Outcome};
use oxikube_app::store::FeedState;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_resources_ui::detail::{DetailDrawer, DetailView};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_workspace::Workspace;
use oxikube_workspace::sidebar::SidebarPanel;

use crate::kind_common::{Launched, wait, wait_value};

fn tab_workspace(app: &mut Launched) -> Entity<Workspace> {
    let tab = app.tab.clone();
    app.vcx.update(|_, cx| tab.read(cx).workspace().clone())
}

/// Activates the sidebar row `entry` (a kind's entry, or a custom resource group or kind) once
/// the sidebar lists it.
pub fn activate(app: &mut Launched, entry: &str) {
    let ws = tab_workspace(app);
    let panel = app
        .vcx
        .update(|_, cx| ws.read(cx).panel::<SidebarPanel>())
        .expect("the sidebar");
    wait(
        &mut app.vcx,
        &format!("the sidebar to list {entry}"),
        |vcx| vcx.update(|_, cx| panel.read(cx).row(entry).is_some()),
    );
    app.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.activate(entry, cx)));
}

/// Activates `entry` and returns the table it opened for `gvk`, narrowed with the filter to
/// `name` (other suites share the cluster), once it lists that object; the cursor is on it.
pub fn open_table(app: &mut Launched, entry: &str, gvk: &Gvk, name: &str) -> Entity<ResourceTable> {
    activate(app, entry);
    let ws = tab_workspace(app);
    let table = wait_value(
        &mut app.vcx,
        &format!("{entry} to open a {} table", gvk.kind),
        |vcx| {
            vcx.update(|_, cx| {
                ws.read(cx)
                    .items_of_type::<ResourceTable>()
                    .into_iter()
                    .find(|table| table.read(cx).gvk() == gvk)
            })
        },
    );
    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| table.set_filter_text(name, window, cx));
    });
    wait(&mut app.vcx, &format!("the table to list {name}"), |vcx| {
        vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready && d.rows().iter().any(|row| row.name() == name)
            })
        })
    });
    // The cursor on the object's row: onto the list, back to its first row, then down to it.
    app.vcx.update(|_, cx| {
        table.update(cx, |table, cx| {
            let (row, len) = table.read_rows(cx, |d| {
                let row = d.rows().iter().position(|r| r.name() == name);
                (row.expect("listed"), d.rows().len())
            });
            let step = |n: usize| isize::try_from(n).unwrap_or(isize::MAX);
            table.move_cursor(1, false, cx);
            table.move_cursor(-step(len), false, cx);
            table.move_cursor(step(row), false, cx);
        });
    });
    app.vcx.run_until_parked();
    table
}

/// Sends `command` on the app's Kubernetes runtime (where a handler's requests must run; the
/// test's own thread has no reactor) and waits for the outcome, running the app meanwhile.
pub fn dispatch(
    app: &mut Launched,
    command: Command,
    context: DispatchContext,
) -> Result<Outcome, DispatchError> {
    let bus = app
        .vcx
        .read(AppState::global)
        .command_bus()
        .expect("the bus")
        .clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let task = app.vcx.update(|_, cx| {
        oxikube_runtime::spawn_kube(cx, async move {
            let _ = tx.send(bus.dispatch(command, context).await);
        })
    });
    let outcome = wait_value(&mut app.vcx, "the command to finish", |_| {
        rx.try_recv().ok()
    });
    drop(task);
    outcome
}

/// `resource::Open` on `target` (Enter on its row): the drawer's detail, once it has read the
/// object from the API server and drawn.
pub fn open_detail(app: &mut Launched, target: &ResourceRef) -> Entity<DetailView> {
    let opened = dispatch(
        app,
        Command::ResourceOpen {
            target: target.clone(),
        },
        DispatchContext::new(oxikube_domain::audit::Initiator::Ui, "kind-sections"),
    );
    assert!(opened.is_ok(), "{opened:?}");
    let ws = tab_workspace(app);
    let view = wait_value(&mut app.vcx, "the detail of the object", |vcx| {
        vcx.update(|_, cx| {
            ws.read(cx)
                .panel::<DetailDrawer>()
                .and_then(|drawer| drawer.read(cx).view().cloned())
                .filter(|view| view.read(cx).target() == target)
        })
    });
    wait(&mut app.vcx, "the detail to read the object", |vcx| {
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
            view.read(cx).resource().is_some()
        })
    });
    app.vcx.update(|window, cx| window.draw(cx).clear(cx));
    view
}
