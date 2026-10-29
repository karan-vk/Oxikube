//! The pod file browser in the running app (E20-S07): "Browse Files" in a pod row's menu opens
//! the pod's filesystem as a tab of the cluster, `pod::BrowseFiles` is on the bus with a tool
//! stub, and an upload from it goes through the app's guard (blocked on a read-only cluster,
//! audited without content).

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_resources_ui::files::FilesView;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::{ExecScript, TestPorts};

use super::App;
use crate::app_state::AppState;

fn ls() -> String {
    "total 8\n\
     drwxr-xr-x 2 root root 4096 2024-03-05 12:34:56.000000000 +0000 etc\n\
     -rw-r--r-- 1 root root 12 2024-03-05 12:34:56.000000000 +0000 motd\n"
        .to_owned()
}

fn views(app: &mut App) -> Vec<gpui::Entity<FilesView>> {
    let ws = app.tab_workspace();
    app.vcx
        .update(|_, cx| ws.read(cx).items_of_type::<FilesView>())
}

#[gpui::test]
fn the_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool, owner) in [
        (
            CommandId::POD_BROWSE_FILES,
            "k8s.pod_browse_files",
            "oxikube_resources_ui::files",
        ),
        (
            CommandId::POD_DOWNLOAD_FILE,
            "k8s.pod_download_file",
            "oxikube_app::files",
        ),
        (
            CommandId::POD_UPLOAD_FILE,
            "k8s.pod_upload_file",
            "oxikube_app::files",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        let def = bus.tool(id).unwrap_or_else(|| panic!("{id} has a stub"));
        assert_eq!(def.name.as_str(), tool);
        assert_eq!(def.is_mutating(), id == CommandId::POD_UPLOAD_FILE, "{id}");
    }
}

#[gpui::test]
fn browse_files_in_a_pods_row_menu_opens_the_filesystem_in_the_cluster_tab(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    let fakes = ports.connector.ports_for(&TestPorts::cluster_id());
    fakes
        .exec_stream
        .script()
        .exec
        .push_ok(ExecScript::new().stdout(ls()));
    let mut app = App::start(cx, ports);
    app.open_pods_table();

    let ws = app.tab_workspace();
    let table = app
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
    let (entry, targets) = app.vcx.update(|_, cx| {
        let table = table.read(cx);
        let entry = table
            .action_entries(cx)
            .into_iter()
            .find(|entry| entry.command() == CommandId::POD_BROWSE_FILES);
        (entry, table.action_targets(cx))
    });
    let entry = entry.expect("pods offer Browse Files");
    assert_eq!(entry.label, "Browse Files");
    assert!(entry.is_enabled());

    app.vcx.update(|window, cx| {
        table.update(cx, |table, cx| {
            table.run_action(CommandId::POD_BROWSE_FILES, targets, window, cx);
        });
    });
    app.tick();
    app.tick();

    let opened = views(&mut app);
    assert_eq!(opened.len(), 1, "a files tab opened in the cluster tab");
    let (pod, names, path) = app.vcx.update(|_, cx| {
        let view = opened[0].read(cx);
        let names: Vec<String> = view
            .model(cx)
            .rows()
            .map(|r| r.entry.name.clone())
            .collect();
        (view.pod().name.to_string(), names, view.path().to_owned())
    });
    assert_eq!(pod, "web-running");
    assert_eq!(path, "/");
    assert_eq!(
        names,
        ["etc", "motd"],
        "the listing came from the container"
    );
    assert!(app.drawn("files-view"));
}

#[gpui::test]
fn an_upload_from_the_browser_is_audited_without_content_and_blocked_on_a_read_only_cluster(
    cx: &mut TestAppContext,
) {
    let ports = TestPorts::seeded();
    let fakes = ports.connector.ports_for(&TestPorts::cluster_id());
    fakes
        .exec_stream
        .script()
        .exec
        .push_ok(ExecScript::new().stdout(ls()));
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.tick();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let pod = web();
    let upload = Command::PodUploadFile {
        target: pod,
        container: Some("app".into()),
        dest_dir: "/tmp".into(),
        local_path: "/home/me/private/notes.txt".into(),
        size: 5,
        overwrite: false,
    };
    // The guard asks first, whoever sends it.
    let outcome = futures::executor::block_on(
        bus.dispatch(upload.clone(), DispatchContext::new(Initiator::Ui, "me")),
    )
    .expect("dispatched");
    assert!(
        matches!(outcome, oxikube_app::command_bus::Outcome::NeedsConfirmation(ref asked)
            if asked.tier == oxikube_domain::safety::ConfirmTier::Simple),
        "an upload confirms: {outcome:?}"
    );
    // An agent's call needs the same confirmation, and a read-only cluster refuses everyone.
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("read-only");
    let error = futures::executor::block_on(
        bus.dispatch(upload, DispatchContext::new(Initiator::Agent, "claude")),
    )
    .expect_err("blocked");
    assert!(
        matches!(
            error,
            oxikube_app::command_bus::DispatchError::ReadOnly { .. }
        ),
        "{error:?}"
    );
    let audit = app.ports.state.audit_log();
    let denied = audit
        .iter()
        .find(|r| &*r.cmd == "pod::UploadFile" && r.outcome == AuditOutcome::Denied)
        .expect("the refusal is audited");
    let detail = denied.detail.as_deref().unwrap_or("");
    assert!(
        detail.contains("container=app path=/tmp/notes.txt size=5"),
        "{detail}"
    );
    assert!(
        !serde_json::to_string(denied)
            .expect("json")
            .contains("/home/me/private")
    );
}

/// The pod the seeded tables show.
fn web() -> oxikube_domain::ids::ResourceRef {
    oxikube_domain::ids::ResourceRef::namespaced(
        TestPorts::cluster_id(),
        oxikube_domain::ids::Gvk::new("", "v1", "Pod"),
        "default",
        "web-running",
    )
}
