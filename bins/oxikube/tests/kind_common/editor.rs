//! The manifest editor in the connected cluster's tab, shared by the E10 kind suites
//! (`kind_templates`, E10-S09; `kind_editor`, E10-S13): the app launched on the real init order
//! with the kind context connected, the cluster's namespace selection on the test's namespace,
//! and what a user does with an editor there (open a template, open a live object with "Edit",
//! type, apply, answer the preview and the conflict list), plus the cluster's side of each check.
//!
//! Real I/O wakes GPUI tasks from Tokio threads: everything here polls with [`wait`] /
//! [`wait_value`], never a fixed sleep before an assertion.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::ApplyService;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::{AuditRecord, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::apply::split_documents;
use oxikube_editor::open::ResourceLoad;
use oxikube_editor::view::{
    ApplyPreviewDialog, ApplyStage, BatchPreviewDialog, ConflictDialog, CreateSecretDialog,
    ManifestEditor, Notice, VALIDATION_DEBOUNCE,
};
use oxikube_ports::AuditQuery;
use oxikube_ui::editor::{EditorApi as _, EditorDiagnostic};
use oxikube_workspace::modal::ModalView;
use oxikube_workspace::{ClusterTab, Workspace};

use super::{Launched, wait, wait_value};

/// Runs the debounced validation and everything it started.
fn settle(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
}

/// The app with `context` connected, its namespace selection on one namespace, and the cluster
/// tab's workspace, where manifest editors open.
pub struct EditorSession {
    pub vcx: VisualTestContext,
    pub inner: Entity<Workspace>,
    pub cluster: ClusterId,
    _tab: Entity<ClusterTab>,
    /// The app's SQLite state (the audit log among it) for this run; removed on drop.
    _dir: tempfile::TempDir,
}

impl EditorSession {
    /// Launches the app on `context` with the cluster's selection on `namespace`.
    pub fn start(cx: &mut TestAppContext, context: &str, namespace: &str) -> Self {
        cx.executor().allow_parking();
        let Launched {
            mut vcx,
            tab,
            cluster,
            dir,
            ..
        } = super::launch(cx, context);
        let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
        let session = Self {
            vcx,
            inner,
            cluster,
            _tab: tab,
            _dir: dir,
        };
        session
            .state()
            .services()
            .sessions
            .set_namespace_selection(&session.cluster, NamespaceSelection::single(namespace))
            .expect("a session");
        session
    }

    /// The app's state: services, bus, state port.
    pub fn state(&self) -> Arc<AppState> {
        self.vcx.read(AppState::global)
    }

    /// Dispatches `command` from the UI and runs the app until it is quiet.
    pub fn dispatch(&mut self, command: Command) {
        let bus = self.state().command_bus().expect("the bus").clone();
        block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
            .expect("the command runs");
        self.vcx.run_until_parked();
    }

    /// Marks the cluster read-only (or not), as its tab's toggle does.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.state()
            .services()
            .sessions
            .set_read_only(&self.cluster, read_only)
            .expect("a session");
        self.vcx.run_until_parked();
    }

    /// The app's own apply pipeline (its bus, its guard, its sessions), for a check the editor
    /// itself refuses to send.
    pub fn apply_service(&self) -> ApplyService {
        let state = self.state();
        ApplyService::new(
            state.command_bus().expect("the bus").clone(),
            state.services().sessions.clone(),
            "kind-editor-test",
        )
    }

    /// Runs `fut` on the app's Kubernetes runtime (`spawn_kube`, as the editor runs its applies)
    /// and waits for its answer, running the app meanwhile.
    pub fn on_kube<R: Send + 'static>(
        &mut self,
        what: &str,
        fut: impl Future<Output = R> + Send + 'static,
    ) -> R {
        let (tx, rx) = std::sync::mpsc::channel();
        let task = self.vcx.update(|_, cx| {
            oxikube_runtime::spawn_kube(cx, async move {
                let _ = tx.send(fut.await);
            })
        });
        let answer = wait_value(&mut self.vcx, what, |_| rx.try_recv().ok());
        drop(task);
        answer
    }

    /// The manifest editors open in the cluster tab.
    pub fn editors(&mut self) -> Vec<Entity<ManifestEditor>> {
        self.vcx
            .update(|_, cx| self.inner.read(cx).items_of_type::<ManifestEditor>())
    }

    /// Waits for a new editor beyond the `before` already open and returns the active one.
    fn opened(&mut self, before: usize, what: &str) -> Entity<ManifestEditor> {
        wait(&mut self.vcx, what, |vcx| {
            vcx.update(|_, cx| self.inner.read(cx).items_of_type::<ManifestEditor>().len()) > before
        });
        self.vcx.update(|_, cx| {
            self.inner
                .read(cx)
                .active_item(cx)
                .and_then(|item| item.downcast::<ManifestEditor>())
                .expect("the new editor is the active tab")
        })
    }

    /// Opens the template `name` in the cluster's tab ("New from template") and returns its
    /// editor.
    pub fn create(&mut self, name: &str) -> Entity<ManifestEditor> {
        let before = self.editors().len();
        self.dispatch(Command::ResourceCreate {
            cluster: Some(self.cluster.clone()),
            template: Some(name.to_owned()),
        });
        self.opened(before, &format!("the {name} editor"))
    }

    /// Opens an empty manifest editor on the cluster and types `text` into it.
    pub fn new_manifest(&mut self, text: &str) -> Entity<ManifestEditor> {
        let before = self.editors().len();
        self.dispatch(Command::EditorNewManifest {
            cluster: Some(self.cluster.clone()),
        });
        let editor = self.opened(before, "the new manifest editor");
        self.set_text(&editor, text);
        editor
    }

    /// "Edit" on `target` (the row action and the drawer's button): the editor opened on a fresh
    /// `get` of the live object, once its text is in.
    pub fn edit(&mut self, target: ResourceRef) -> Entity<ManifestEditor> {
        let before = self.editors().len();
        self.dispatch(Command::ResourceEdit { target });
        let editor = self.opened(before, "the Edit editor");
        wait(&mut self.vcx, "the live object to load", |vcx| {
            vcx.update(|_, cx| {
                let load = editor.read(cx).binding().map(|b| b.load().clone());
                match load {
                    Some(ResourceLoad::Ready) => true,
                    Some(ResourceLoad::Failed(error)) => panic!("the get failed: {error}"),
                    _ => false,
                }
            })
        });
        editor
    }

    /// "Edit decoded" on the Secret `target` (`secret::EditDecoded`): the editor opened on a
    /// fresh `get`, once its text, the values as plaintext, is in.
    pub fn edit_decoded(&mut self, target: ResourceRef) -> Entity<ManifestEditor> {
        let before = self.editors().len();
        self.dispatch(Command::SecretEditDecoded { target });
        let editor = self.opened(before, "the Edit decoded editor");
        wait(&mut self.vcx, "the live Secret to load", |vcx| {
            vcx.update(|_, cx| {
                let load = editor.read(cx).binding().map(|b| b.load().clone());
                match load {
                    Some(ResourceLoad::Ready) => true,
                    Some(ResourceLoad::Failed(error)) => panic!("the get failed: {error}"),
                    _ => false,
                }
            })
        });
        editor
    }

    /// `secret::Create`: the dialog in the cluster's tab.
    pub fn create_secret_dialog(&mut self) -> Entity<CreateSecretDialog> {
        self.dispatch(Command::SecretCreate {
            cluster: Some(self.cluster.clone()),
        });
        self.modal("the Create secret dialog")
    }

    /// Waits for the editor the dialog's result opened (beyond the `before` open).
    pub fn secret_editor(&mut self, before: usize) -> Entity<ManifestEditor> {
        self.opened(before, "the new Secret editor")
    }

    /// The editor's text.
    pub fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.update(|_, cx| editor.read(cx).editor().clone());
        self.vcx
            .update(|window, cx| code.update(cx, |c, cx| c.api(window, cx).text().to_string()))
    }

    /// Replaces the editor's text, as typing would (a new buffer version).
    pub fn set_text(&mut self, editor: &Entity<ManifestEditor>, text: &str) {
        let code = self.vcx.update(|_, cx| editor.read(cx).editor().clone());
        self.vcx
            .update(|window, cx| code.update(cx, |c, cx| c.api(window, cx).set_text(text)));
        self.vcx.run_until_parked();
    }

    /// The squiggles the editor draws: its checks' and the server's.
    pub fn diagnostics(&mut self, editor: &Entity<ManifestEditor>) -> Vec<EditorDiagnostic> {
        let code = self.vcx.update(|_, cx| editor.read(cx).editor().clone());
        self.vcx
            .update(|window, cx| code.update(cx, |c, cx| c.api(window, cx).diagnostics()))
    }

    /// Waits for the cluster's schema of every kind in `kinds`, then for the checks of the text
    /// as it is.
    pub fn wait_checked(&mut self, editor: &Entity<ManifestEditor>, kinds: &[Gvk]) {
        for gvk in kinds {
            wait(&mut self.vcx, &format!("the schema of {gvk}"), |vcx| {
                settle(vcx);
                vcx.update(|_, cx| {
                    editor
                        .read(cx)
                        .model()
                        .known()
                        .get(gvk)
                        .is_some_and(Option::is_some)
                })
            });
        }
        let version = self
            .vcx
            .update(|_, cx| editor.read(cx).editor().read(cx).version());
        wait(&mut self.vcx, "the checks of the text", |vcx| {
            settle(vcx);
            vcx.update(|_, cx| editor.read(cx).model().shown_version()) == Some(version)
        });
    }

    /// The editor's apply key (`resource::Apply`): its checks, then the server's dry run.
    pub fn apply(&mut self, editor: &Entity<ManifestEditor>) {
        editor.update_in(&mut self.vcx, |view, window, cx| {
            view.begin_apply(window, cx)
        });
        self.vcx.run_until_parked();
    }

    /// The preview of one object, once the dry run opened it.
    pub fn single_preview(&mut self, what: &str) -> Entity<ApplyPreviewDialog> {
        self.modal(what)
    }

    /// The combined preview of several documents, once the dry runs opened it.
    pub fn batch_preview(&mut self, what: &str) -> Entity<BatchPreviewDialog> {
        self.modal(what)
    }

    /// The list of fields other managers own, once the dry run opened it.
    pub fn conflicts(&mut self, what: &str) -> Entity<ConflictDialog> {
        self.modal(what)
    }

    fn modal<V: ModalView>(&mut self, what: &str) -> Entity<V> {
        wait_value(&mut self.vcx, what, |vcx| {
            vcx.update(|_, cx| {
                self.inner
                    .read(cx)
                    .modal_layer()
                    .read(cx)
                    .active_modal::<V>()
            })
        })
    }

    /// Confirms the open single-object preview (its Apply) and waits until the editor is idle.
    pub fn confirm(&mut self, editor: &Entity<ManifestEditor>, what: &str) {
        let dialog = self.single_preview(what);
        dialog.update_in(&mut self.vcx, |dialog, window, cx| {
            dialog.confirm(window, cx)
        });
        self.wait_idle(editor);
    }

    /// Waits until the editor's apply flow has nothing running and nothing to review.
    pub fn wait_idle(&mut self, editor: &Entity<ManifestEditor>) {
        wait(&mut self.vcx, "the apply to finish", |vcx| {
            vcx.update(|_, cx| editor.read(cx).apply_stage()) == ApplyStage::Idle
        });
    }

    /// The editor's notice line, waiting until it says something other than "in progress".
    pub fn settled_notice(&mut self, editor: &Entity<ManifestEditor>, what: &str) -> Notice {
        wait_value(&mut self.vcx, what, |vcx| {
            vcx.update(|_, cx| {
                let view = editor.read(cx);
                (view.apply_stage() == ApplyStage::Idle)
                    .then(|| view.notice().cloned())
                    .flatten()
            })
        })
    }

    /// The audit log's `resource::Apply` records for `name`, oldest first, once there are at
    /// least `at_least` of them (the guard writes them as each attempt ends).
    pub fn apply_audit(&mut self, name: &str, at_least: usize) -> Vec<AuditRecord> {
        let state = self.state();
        let query = AuditQuery {
            cluster: Some(self.cluster.clone()),
            cmd: Some("resource::Apply".to_owned()),
            ..AuditQuery::default()
        };
        wait_value(
            &mut self.vcx,
            &format!("{at_least} audit record(s) of {name}"),
            |_| {
                let mut records: Vec<AuditRecord> = block_on(state.state().query_audit(&query))
                    .expect("the audit log")
                    .records
                    .into_iter()
                    .filter(|r| &*r.target.name == name)
                    .collect();
                records.sort_by_key(|r| r.ts);
                (records.len() >= at_least).then_some(records)
            },
        )
    }
}

/// The kind of each document of `text`.
pub fn gvks_in(text: &str) -> Vec<Gvk> {
    split_documents(text)
        .iter()
        .filter_map(|document| {
            let field = |key: &str| {
                document
                    .text
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
            };
            Some(Gvk::from_api_version(
                field("apiVersion")?.trim(),
                field("kind")?.trim(),
            ))
        })
        .collect()
}

/// The dynamic API of `plural` (`group`/`version`, kind `kind`) in `namespace`.
pub fn dynamic_api(
    client: kube::Client,
    namespace: &str,
    (group, version, kind, plural): (&str, &str, &str, &str),
) -> kube::Api<kube::api::DynamicObject> {
    let resource = kube::api::ApiResource::from_gvk_with_plural(
        &kube::api::GroupVersionKind::gvk(group, version, kind),
        plural,
    );
    kube::Api::namespaced_with(client, namespace, &resource)
}

/// The Secrets API of `namespace`.
pub fn secrets(client: kube::Client, namespace: &str) -> kube::Api<kube::api::DynamicObject> {
    dynamic_api(client, namespace, ("", "v1", "Secret", "secrets"))
}

/// The ConfigMaps API of `namespace`.
pub fn configmaps(client: kube::Client, namespace: &str) -> kube::Api<kube::api::DynamicObject> {
    dynamic_api(client, namespace, ("", "v1", "ConfigMap", "configmaps"))
}
