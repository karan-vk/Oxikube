//! Applying the buffer (E10-S08) in a workspace, end to end over the real `ApplyService`,
//! `CommandBus` and `MutationGuard`, with the testkit's fake cluster ports on the deterministic
//! runtime: the key opens the server's dry run as a diff preview, Apply sends a server-side
//! apply, a read-only cluster is refused before any request, a rejected field gets a squiggle,
//! a conflict lists its managers and Force applies over them, a buffer with errors is refused,
//! and the palette's `resource::Apply` runs the same flow.

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ApplyMethod, ApplyService, ClusterSessionManager, MutationGuard};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::{ConflictDetails, ConflictReason, FieldCause, OxiError, ValidationDetails};
use oxikube_editor::view::{
    ApplyPreviewDialog, ApplyStage, ConflictDialog, EditorHost, EditorRequest, EditorViewSink,
    EditorViews, EditorViewsDeps, ManifestEditor, SchemaSource,
};
use oxikube_ports::{ClusterContext, PatchKind, SchemaPort, SourceId};
use oxikube_testkit::fakes::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeResourcePort,
    FakeSchemaPort, FakeStatePort, ResourceCall,
};
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::modal::ModalView;
use oxikube_workspace::test_support::{TestModal, open_workspace};
use oxikube_workspace::{CommandDispatcher, Workspace};

const CONFIGMAP: &str = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: web
  namespace: shop
data:
  mode: blue
";

fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    ClusterContext::new(
        ClusterId::new("/kubeconfig", &name),
        name,
        SourceId("kubeconfig".into()),
    )
}

fn cluster() -> ClusterId {
    context().cluster
}

/// The editors' commands go to their queue; nothing else is dispatched in these tests.
struct SinkDispatcher(EditorViewSink);

impl CommandDispatcher for SinkDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        if let Some(request) = EditorRequest::of(command) {
            self.0.send(request).expect("the window is open");
        }
    }
}

struct Schemas(Arc<FakeSchemaPort>);

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.0.clone())
    }
}

/// The window's workspace is the cluster's tab; its editors apply through `service`.
struct Host {
    workspace: WeakEntity<Workspace>,
    schemas: Arc<FakeSchemaPort>,
    service: ApplyService,
}

impl EditorHost for Host {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        Some(cluster())
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(Schemas(self.schemas.clone()))
    }
    fn apply(&self, _: &ClusterId) -> Option<ApplyService> {
        Some(self.service.clone())
    }
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    sessions: ClusterSessionManager,
    resources: Arc<FakeResourcePort>,
    state: Arc<FakeStatePort>,
    sink: EditorViewSink,
    _views: Entity<EditorViews>,
}

fn setup(cx: &mut TestAppContext, read_only: bool) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    sessions.open(
        &context(),
        SessionOptions {
            read_only,
            ..SessionOptions::default()
        },
    );
    sessions
        .connect(&cluster())
        .now_or_never()
        .expect("the fakes answer at once")
        .expect("connected");
    let ports = connector.ports_for(&cluster());
    ports
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::apply", |r| {
            oxikube_app::apply::register_commands(r, sessions.clone())
        })
        .expect("registered");
    let guard = MutationGuard::new(sessions.clone(), state.clone(), clock);
    let bus = CommandBus::new(registry, guard);
    let service = ApplyService::new(bus, sessions.clone(), "alice");

    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher(sink.clone()));
    let host = Rc::new(Host {
        workspace: workspace.downgrade(),
        schemas: Arc::new(FakeSchemaPort::new()),
        service,
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_editor::view::install(dispatcher.clone(), cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        let deps = EditorViewsDeps {
            host,
            window_workspace: workspace.downgrade(),
            dispatcher,
        };
        EditorViews::start(deps, requests, window, cx)
    });
    vcx.run_until_parked();
    Fixture {
        workspace,
        vcx,
        sessions,
        resources: ports.resources,
        state,
        sink,
        _views: views,
    }
}

fn apply_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    }
}

impl Fixture {
    /// A new editor holding `text`, focused.
    fn editor_with(&mut self, text: &str) -> Entity<ManifestEditor> {
        self.sink
            .send(EditorRequest::New { cluster: None })
            .unwrap();
        self.vcx.run_until_parked();
        let editor = self
            .vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>())
            .pop()
            .expect("an editor opened");
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).set_text(text));
        });
        self.vcx.run_until_parked();
        editor
    }

    fn press_apply(&mut self) {
        self.vcx.simulate_keystrokes(apply_key());
        self.vcx.run_until_parked();
    }

    fn modal<V: ModalView>(&mut self) -> Option<Entity<V>> {
        self.vcx.read(|cx| {
            self.workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<V>()
        })
    }

    /// `(method, dry_run)` of each write the cluster got.
    fn writes(&self) -> Vec<(&'static str, bool)> {
        self.resources
            .mutating_calls()
            .iter()
            .map(|call| {
                let method = match call {
                    ResourceCall::Patch { .. } => "patch",
                    ResourceCall::Replace { .. } => "replace",
                    ResourceCall::Create { .. } => "create",
                    _ => "other",
                };
                (method, call.is_dry_run())
            })
            .collect()
    }

    fn notice(&mut self, editor: &Entity<ManifestEditor>) -> String {
        self.vcx.read(|cx| {
            editor
                .read(cx)
                .notice()
                .map(|n| n.text.to_string())
                .unwrap_or_default()
        })
    }

    fn confirm(&mut self, dialog: &Entity<ApplyPreviewDialog>) {
        dialog.update_in(&mut self.vcx, |dialog, window, cx| {
            dialog.confirm(window, cx)
        });
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn the_key_dry_runs_shows_the_diff_and_apply_sends_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();

    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview opened");
    assert_eq!(f.writes(), [("patch", true)], "only the dry run so far");
    let (title, description, added) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog.description(),
            dialog.diff().counts().1,
        )
    });
    assert_eq!(title, "Create ConfigMap shop/web?");
    assert!(
        description.contains("Server-side apply as oxikube"),
        "{description}"
    );
    assert!(added > 0);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Reviewing
    );

    f.confirm(&dialog);
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(
        f.writes(),
        [("patch", true), ("patch", true), ("patch", false)]
    );
    let ResourceCall::Patch { patch, .. } = &f.resources.mutating_calls()[2] else {
        panic!("a patch")
    };
    assert_eq!(
        patch.kind,
        PatchKind::Apply {
            manager: "oxikube".into(),
            force: false
        }
    );
    assert_eq!(f.resources.objects().len(), 1);
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/web");
    assert!(!f.vcx.read(|cx| editor.read(cx).is_dirty(cx)), "clean");
    let toasts = f.vcx.read(|cx| {
        f.workspace
            .read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .into_iter()
            .map(|t| t.message.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(toasts, ["Applied ConfigMap shop/web"]);
    let audit: Vec<(bool, AuditOutcome)> = f
        .state
        .audit_log()
        .iter()
        .map(|r| (r.dry_run, r.outcome))
        .collect();
    assert_eq!(
        audit,
        [
            (true, AuditOutcome::Succeeded),
            (false, AuditOutcome::Succeeded)
        ]
    );
}

#[gpui::test]
fn cancel_sends_nothing_more(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    dialog.update(&mut f.vcx, |dialog, cx| dialog.cancel(cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(f.writes(), [("patch", true)]);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Idle
    );
}

impl Fixture {
    /// Another modal replaces the open one, as the palette's key does over a dialog.
    fn replace_modal(&mut self) {
        let workspace = self.workspace.clone();
        workspace.update_in(&mut self.vcx, |workspace, window, cx| {
            let other = cx.new(|cx| TestModal::new("other", cx));
            workspace.show_modal(other, window, cx);
        });
        self.vcx.run_until_parked();
    }

    fn stage(&mut self, editor: &Entity<ManifestEditor>) -> ApplyStage {
        self.vcx.read(|cx| editor.read(cx).apply_stage())
    }
}

#[gpui::test]
fn a_preview_replaced_by_another_modal_leaves_the_editor_idle(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
    assert_eq!(f.stage(&editor), ApplyStage::Reviewing);

    f.replace_modal();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    assert_eq!(f.writes(), [("patch", true)], "nothing applied");
    // Apply works again.
    f.workspace.update_in(&mut f.vcx, |workspace, window, cx| {
        workspace.hide_modal(window, cx)
    });
    editor.update_in(&mut f.vcx, |view, window, cx| view.begin_apply(window, cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_conflict_list_replaced_by_another_modal_leaves_the_editor_idle(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![FieldCause {
                field: ".data.mode".into(),
                manager: Some("kubectl".into()),
                reason: "FieldManagerConflict".into(),
                message: "conflict with \"kubectl\"".into(),
            }],
        }),
    );
    f.press_apply();
    assert!(f.modal::<ConflictDialog>().is_some());
    assert_eq!(f.stage(&editor), ApplyStage::Reviewing);

    f.replace_modal();
    assert!(f.modal::<ConflictDialog>().is_none());
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
}

#[gpui::test]
fn a_read_only_cluster_is_refused_before_any_request(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert_eq!(
        f.notice(&editor),
        "kind-oxikube is read-only: the manifest was not applied"
    );
    assert_eq!(f.state.audit_log()[0].outcome, AuditOutcome::Denied);
    // Read-only mode lifted: the same key works.
    f.sessions.set_read_only(&cluster(), false).unwrap();
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_rejected_field_gets_a_squiggle_on_its_key(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::validation("ConfigMap \"web\" is invalid").with_source(ValidationDetails {
            causes: vec![FieldCause {
                field: "data.mode".into(),
                reason: "FieldValueInvalid".into(),
                message: "Invalid value: \"blue\"".into(),
                manager: None,
            }],
        }),
    );
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    let (squiggles, problems) = f.vcx.read(|cx| {
        let view = editor.read(cx);
        (
            view.model().server_problems().to_vec(),
            view.model().problems(),
        )
    });
    assert_eq!(squiggles.len(), 1);
    assert_eq!(&CONFIGMAP[squiggles[0].range.clone()], "mode");
    assert_eq!(squiggles[0].message, "data.mode: Invalid value: \"blue\"");
    assert_eq!(problems.errors, 1);
    assert!(
        f.notice(&editor)
            .starts_with("The server rejected the manifest: ConfigMap \"web\" is invalid"),
        "{}",
        f.notice(&editor)
    );
}

#[gpui::test]
fn a_conflict_lists_its_managers_and_force_applies_over_them(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![FieldCause {
                field: ".data.mode".into(),
                manager: Some("kubectl".into()),
                reason: "FieldManagerConflict".into(),
                message: "conflict with \"kubectl\"".into(),
            }],
        }),
    );
    f.press_apply();
    let conflicts = f.modal::<ConflictDialog>().expect("the conflict list");
    let managers = f.vcx.read(|cx| {
        let dialog = conflicts.read(cx);
        assert_eq!(dialog.conflicts()[0].field, ".data.mode");
        dialog.managers().join(",")
    });
    assert_eq!(managers, "kubectl");

    conflicts.update_in(&mut f.vcx, |dialog, window, cx| dialog.force(window, cx));
    f.vcx.run_until_parked();
    let preview = f.modal::<ApplyPreviewDialog>().expect("the forced preview");
    let forced = f.vcx.read(|cx| preview.read(cx).preview().request.force);
    assert!(forced);
    f.confirm(&preview);
    let forces: Vec<bool> = f
        .resources
        .mutating_calls()
        .iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, .. } => match patch.kind {
                PatchKind::Apply { force, .. } => Some(force),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(forces, [false, true, true, true]);
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/web");
}

#[gpui::test]
fn a_buffer_with_errors_is_refused_before_the_server_sees_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with("apiVersion: v1\nkind: ConfigMap\nmetadata: [\n");
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert!(
        f.notice(&editor).starts_with("Fix the"),
        "{}",
        f.notice(&editor)
    );
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Idle
    );
}

#[gpui::test]
fn the_palettes_apply_runs_the_editors_flow(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.editor_with(CONFIGMAP);
    let ran = f.vcx.update(|window, cx| {
        let surface = oxikube_workspace::command_surface::focused(window, cx).expect("a surface");
        assert!(surface.own_commands.contains(&CommandId::RESOURCE_APPLY));
        assert_eq!(surface.target.cluster, Some(cluster()));
        oxikube_workspace::command_surface::run_on_focused(
            CommandId::RESOURCE_APPLY,
            Vec::new(),
            window,
            cx,
        )
    });
    assert!(ran);
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_server_without_apply_is_previewed_as_a_replace(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.resources.insert(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "41"},
            "data": {"mode": "green"}
        }))
        .unwrap(),
    );
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(OxiError::unsupported(
        "the body of the request was in an unknown format",
    ));
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    let (method, description) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (dialog.preview().method, dialog.description())
    });
    assert_eq!(method, ApplyMethod::Replace);
    assert!(
        description.contains("replaced at resourceVersion 41"),
        "{description}"
    );
    f.resources.script().patch.push_err(OxiError::unsupported(
        "the body of the request was in an unknown format",
    ));
    f.confirm(&dialog);
    let replaced = f.resources.mutating_calls();
    let ResourceCall::Replace {
        object, options, ..
    } = replaced.last().unwrap()
    else {
        panic!("a replace: {replaced:?}")
    };
    assert!(!options.dry_run);
    assert_eq!(object["metadata"]["resourceVersion"], "41");
    assert!(f.notice(&editor).starts_with("Replaced ConfigMap shop/web"));
}
