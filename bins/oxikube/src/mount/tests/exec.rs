//! Debug containers (E09-S10) are at the end: `shift-d` on a pod's row opens the dialog, its button runs
//! `pod::Debug` through the bus and the guard, and a terminal attached to the new container opens.
//!
//! Shells in pods on the real init path (E09-S08): `s` on a pod's row reaches the app's bus and
//! guard, is audited, and opens a terminal tab in the cluster tab's bottom dock; a read-only
//! cluster blocks it unless `exec_in_read_only` is set; the commands are on the bus with unsafe
//! tool stubs.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_settings::SettingsStore;
use oxikube_terminal::view::{
    BackendDescriptor, Launch, TerminalLauncher, TerminalServices, TerminalView,
};
use oxikube_testkit::TestPorts;
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_workspace::DockPosition;

use super::App;
use crate::app_state::AppState;

/// Starts a `FakeTerminalBackend` for each launch and keeps what it was asked.
#[derive(Default)]
struct FakeLauncher {
    launches: RefCell<Vec<BackendDescriptor>>,
}

impl TerminalLauncher for FakeLauncher {
    fn launch(
        &self,
        descriptor: &BackendDescriptor,
        _: oxikube_ports::TerminalSize,
        _: &mut gpui::App,
    ) -> Launch {
        self.launches.borrow_mut().push(descriptor.clone());
        gpui::Task::ready(Ok(Box::new(FakeTerminalBackend::silent())))
    }
}

/// The app with a fake launcher and a pods table open, the first row under the cursor.
fn pods_table(cx: &mut TestAppContext) -> (App, Rc<FakeLauncher>) {
    let launcher = Rc::new(FakeLauncher::default());
    let installed = launcher.clone();
    let mut app = App::start_with(cx, TestPorts::seeded(), move |cx| {
        oxikube_terminal::view::install(TerminalServices::new(installed), cx);
    });
    app.open_pods_table();
    (app, launcher)
}

fn terminals(app: &mut App) -> Vec<(gpui::EntityId, Option<DockPosition>)> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        let ws = ws.read(cx);
        ws.items_of_type::<TerminalView>()
            .iter()
            .map(|view| (view.entity_id(), ws.item_dock(view.entity_id(), cx)))
            .collect()
    })
}

#[gpui::test]
fn the_exec_commands_are_on_the_bus_as_unsafe_stubs_hidden_from_agents(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::POD_SHELL, "k8s.pod_shell"),
        (CommandId::POD_ATTACH, "k8s.pod_attach"),
        (CommandId::POD_EXEC, "k8s.pod_exec"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_terminal"), "{id}");
        let def = bus.tool(id).unwrap_or_else(|| panic!("{id} has a stub"));
        assert_eq!(def.name.as_str(), tool);
        assert!(
            def.annotations.unsafe_ && def.annotations.interactive,
            "{tool}"
        );
        assert!(
            !bus.agent_tools(false).any(|t| t.name.as_str() == tool),
            "{tool} is not offered to agents by default"
        );
        assert!(bus.agent_tools(true).any(|t| t.name.as_str() == tool));
    }
    assert!(
        state.exec_service().is_some(),
        "the mount built the exec service"
    );
}

#[gpui::test]
fn s_on_a_pods_row_opens_an_audited_shell_in_the_cluster_tabs_bottom_dock(cx: &mut TestAppContext) {
    let (mut app, launcher) = pods_table(cx);
    app.press("s");
    app.tick();
    app.tick();

    let launches = launcher.launches.borrow().clone();
    let [
        BackendDescriptor::Exec {
            pod,
            container,
            command,
        },
    ] = launches.as_slice()
    else {
        panic!("one pod shell was started, got {launches:?}");
    };
    assert_eq!(&*pod.name, "web-running");
    assert!(
        container.is_some(),
        "the container was chosen before the command: {container:?}"
    );
    assert!(command.is_empty(), "an empty command is the shell chain");
    assert_eq!(
        terminals(&mut app)
            .iter()
            .map(|(_, dock)| *dock)
            .collect::<Vec<_>>(),
        [Some(DockPosition::Bottom)],
        "a terminal tab in the bottom dock"
    );

    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "pod::Shell");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(&*audit[0].target.name, "web-running");
    assert!(
        audit[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("session=shell container=")),
        "{:?}",
        audit[0].detail
    );
}

#[gpui::test]
fn a_attaches_through_the_same_path(cx: &mut TestAppContext) {
    let (mut app, launcher) = pods_table(cx);
    app.press("a");
    app.tick();
    app.tick();
    let launches = launcher.launches.borrow().clone();
    assert!(
        matches!(launches.as_slice(), [BackendDescriptor::Attach { pod, .. }] if &*pod.name == "web-running"),
        "{launches:?}"
    );
    assert_eq!(&*app.ports.state.audit_log()[0].cmd, "pod::Attach");
}

#[gpui::test]
fn a_read_only_cluster_blocks_the_shell_until_the_cluster_allows_it(cx: &mut TestAppContext) {
    let (mut app, launcher) = pods_table(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let cluster = TestPorts::cluster_id();
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");

    // The key says why and opens nothing.
    app.press("s");
    app.tick();
    app.expect_toast(
        "This cluster is read-only: shells are blocked (set exec_in_read_only to allow them)",
    );
    assert!(launcher.launches.borrow().is_empty());
    assert!(terminals(&mut app).is_empty());

    // Whatever a stale menu, the palette or an agent sends, the guard refuses it and audits it.
    let bus = state.command_bus().expect("the bus");
    let shell = Command::PodShell {
        target: ResourceRef::namespaced(
            cluster.clone(),
            Gvk::new("", "v1", "Pod"),
            "default",
            "web-running",
        ),
        container: None,
    };
    for initiator in [Initiator::Agent, Initiator::Command, Initiator::Ui] {
        let outcome = futures::executor::block_on(
            bus.dispatch(shell.clone(), DispatchContext::new(initiator, "someone")),
        );
        assert!(
            matches!(outcome, Err(oxikube_app::DispatchError::ReadOnly { .. })),
            "{initiator}: {outcome:?}"
        );
    }
    app.tick();
    assert!(launcher.launches.borrow().is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 3);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));

    // The setting lets it through, on the next key press; read-only mode itself is unchanged.
    app.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "exec_in_read_only": true }"#)
                .expect("valid settings");
        });
    });
    app.tick();
    app.press("s");
    app.tick();
    app.tick();
    assert_eq!(launcher.launches.borrow().len(), 1, "the shell opened");
    assert!(state.services().sessions.is_read_only(&cluster));
}

fn debug_dialog(app: &mut App) -> Option<gpui::Entity<oxikube_resources_ui::exec::DebugDialog>> {
    let ws = app.tab_workspace();
    app.vcx.update(|_, cx| {
        let layer = ws.read(cx).modal_layer().clone();
        layer
            .read(cx)
            .active_modal::<oxikube_resources_ui::exec::DebugDialog>()
    })
}

#[gpui::test]
fn pod_debug_is_on_the_bus_as_a_guarded_mutation_with_an_unsafe_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(bus.owner(CommandId::POD_DEBUG), Some("oxikube_terminal"));
    let meta = bus
        .commands()
        .find(|m| m.id == CommandId::POD_DEBUG)
        .expect("registered");
    assert!(meta.mutating && !meta.exec, "a mutation, not exec-class");
    let tool = bus.tool(CommandId::POD_DEBUG).expect("a stub");
    assert_eq!(tool.name.as_str(), "k8s.pod_debug");
    assert!(tool.annotations.unsafe_ && tool.annotations.interactive);
    assert!(
        !bus.agent_tools(false)
            .any(|t| t.name.as_str() == "k8s.pod_debug")
    );
    assert!(
        bus.agent_tools(true)
            .any(|t| t.name.as_str() == "k8s.pod_debug")
    );
}

#[gpui::test]
fn shift_d_on_a_pods_row_adds_a_debug_container_and_opens_a_terminal_attached_to_it(
    cx: &mut TestAppContext,
) {
    let (mut app, launcher) = pods_table(cx);
    app.press("shift-d");
    app.tick();
    app.tick();
    let dialog = debug_dialog(&mut app).expect("the debug dialog opens");
    assert!(
        launcher.launches.borrow().is_empty(),
        "nothing before the button"
    );
    assert!(app.ports.state.audit_log().is_empty());

    app.vcx.update(|_, cx| {
        dialog.update(cx, |dialog, cx| {
            let defaults = dialog.defaults();
            assert_eq!(
                (defaults.image.as_str(), defaults.command.as_str()),
                ("busybox", "sh")
            );
            dialog.submit(cx);
        });
    });
    for _ in 0..4 {
        app.tick();
    }
    // The terminal is an attach to the container the command added, in the bottom dock.
    let launches = launcher.launches.borrow().clone();
    let [
        BackendDescriptor::Attach {
            pod,
            container: Some(container),
        },
    ] = launches.as_slice()
    else {
        panic!("one attach to the new container, got {launches:?}");
    };
    assert_eq!(&*pod.name, "web-running");
    assert!(container.starts_with("debugger-"), "{container}");
    assert_eq!(
        terminals(&mut app)
            .iter()
            .map(|(_, dock)| *dock)
            .collect::<Vec<_>>(),
        [Some(DockPosition::Bottom)]
    );
    assert!(debug_dialog(&mut app).is_none(), "the dialog closed");

    // The container was added through the port, and the guard audited one mutation.
    let calls = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .exec
        .recorded_calls();
    assert!(
        matches!(calls.as_slice(), [oxikube_testkit::ExecPortCall::CreateDebugContainer(spec)]
            if spec.image == "busybox" && spec.name.as_deref() == Some(container.as_str())),
        "{calls:?}"
    );
    let audit = app.ports.state.audit_log();
    let [record] = audit.as_slice() else {
        panic!("one audit record, got {audit:?}");
    };
    assert_eq!(&*record.cmd, "pod::Debug");
    assert_eq!(
        (record.outcome, record.initiator),
        (AuditOutcome::Succeeded, Initiator::Ui)
    );
    assert!(
        record
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("session=debug image=busybox target=")),
        "{:?}",
        record.detail
    );
}

#[gpui::test]
fn a_read_only_cluster_blocks_debug_even_when_it_allows_shells(cx: &mut TestAppContext) {
    let (mut app, launcher) = pods_table(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");
    app.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "exec_in_read_only": true }"#)
                .expect("valid settings");
        });
    });
    app.tick();
    app.press("shift-d");
    app.tick();
    assert!(
        debug_dialog(&mut app).is_none(),
        "no dialog on a read-only cluster"
    );
    assert!(launcher.launches.borrow().is_empty());
    assert!(
        app.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .exec
            .recorded_calls()
            .is_empty()
    );
    assert!(app.ports.state.audit_log().is_empty(), "nothing was sent");
}
