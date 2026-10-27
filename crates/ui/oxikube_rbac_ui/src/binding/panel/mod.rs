//! [`BindingPanel`]: a RoleBinding's or ClusterRoleBinding's role and subjects in the detail, with
//! the rules the role grants and the buttons that edit or delete the binding.

use std::sync::Arc;

use gpui::{Context, Subscription, Task, WeakEntity};
use oxikube_app::binding_ops::role_of;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::rbac::{Binding, Role, RoleKind, Subject};
use oxikube_resources_ui::detail::{DetailView, PanelHost};
use oxikube_runtime::spawn_kube;

use crate::role::RuleRow;

/// What the panel knows of the role the binding names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolePhase {
    /// Being read.
    Loading,
    /// Read: its rules are the table.
    Ready,
    /// The binding names a role that does not exist (the binding grants nothing).
    Missing,
    /// The read failed (no right to read roles, offline).
    Failed(String),
}

/// One subject as the table shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectRow {
    /// `User`, `Group` or `ServiceAccount`.
    pub kind: &'static str,
    /// The name.
    pub name: Arc<str>,
    /// A ServiceAccount's namespace; blank for the others.
    pub namespace: Arc<str>,
    /// `rbac.authorization.k8s.io` for a User or Group; blank (the core group) for a ServiceAccount.
    pub api_group: &'static str,
}

impl SubjectRow {
    /// The row of `subject`.
    pub fn of(subject: &Subject) -> Self {
        match subject {
            Subject::User(name) | Subject::Group(name) => Self {
                kind: subject.kind(),
                name: name.clone(),
                namespace: Arc::from(""),
                api_group: oxikube_domain::rbac::RBAC_GROUP,
            },
            Subject::ServiceAccount { namespace, name } => Self {
                kind: subject.kind(),
                name: name.clone(),
                namespace: namespace.clone(),
                api_group: "",
            },
        }
    }
}

/// What the rows were made from: another version of the object makes them again.
#[derive(Clone, PartialEq, Eq)]
struct Source {
    uid: Option<Arc<str>>,
    version: Option<Arc<str>>,
}

/// The binding panel. See the module docs.
pub struct BindingPanel {
    detail: WeakEntity<DetailView>,
    binding: Option<Arc<Binding>>,
    subjects: Arc<[SubjectRow]>,
    phase: RolePhase,
    rules: Arc<[RuleRow]>,
    built_from: Option<Source>,
    /// The role being read; a newer read replaces (and so cancels) it.
    reading: Option<Task<()>>,
    _detail: Option<Subscription>,
}

impl BindingPanel {
    /// A panel over `detail`, following it.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let subscription = detail
            .upgrade()
            .map(|entity| cx.observe(&entity, |this, _, cx| this.refresh(cx)));
        // The detail is being drawn (it built this panel), so the object is read after the draw.
        let panel = cx.entity().downgrade();
        cx.defer(move |cx| {
            panel.update(cx, |this, cx| this.refresh(cx)).ok();
        });
        Self {
            detail,
            binding: None,
            subjects: Arc::from(Vec::new()),
            phase: RolePhase::Loading,
            rules: Arc::from(Vec::new()),
            built_from: None,
            reading: None,
            _detail: subscription,
        }
    }

    /// The binding, once read.
    pub fn binding(&self) -> Option<&Binding> {
        self.binding.as_deref()
    }

    /// The subjects table.
    pub fn subjects(&self) -> &[SubjectRow] {
        &self.subjects
    }

    /// The rules the named role grants, once read.
    pub fn rules(&self) -> &[RuleRow] {
        &self.rules
    }

    /// Where the read of the role is.
    pub fn phase(&self) -> &RolePhase {
        &self.phase
    }

    /// Builds the rows when the object has a new version.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (binding, source, host) = {
            let view = detail.read(cx);
            let Some(resource) = view.resource() else {
                return;
            };
            let source = Source {
                uid: resource.meta.uid.clone(),
                version: resource.meta.resource_version.clone(),
            };
            if self.built_from.as_ref() == Some(&source) {
                return;
            }
            let Ok(binding) = Binding::from_resource(resource) else {
                return;
            };
            (binding, source, view.host())
        };
        self.subjects = binding.subjects.iter().map(SubjectRow::of).collect();
        self.built_from = Some(source);
        self.read_role(&binding, &host, cx);
        self.binding = Some(Arc::new(binding));
        cx.notify();
    }

    /// Reads the role the binding names, off the UI thread.
    fn read_role(&mut self, binding: &Binding, host: &PanelHost, cx: &mut Context<Self>) {
        self.phase = RolePhase::Loading;
        self.rules = Arc::from(Vec::new());
        let sessions = host.sessions().clone();
        let cluster = host.target().cluster.clone();
        let binding = binding.clone();
        let read = spawn_kube(
            cx,
            async move { role_of(&sessions, &cluster, &binding).await },
        );
        self.reading = Some(cx.spawn(async move |this, cx| {
            let found = match read.await {
                Ok(result) => result,
                Err(error) => Err(oxikube_domain::OxiError::internal(error.to_string())),
            };
            this.update(cx, |this, cx| {
                match found {
                    Ok(Some(role)) => this.role_read(&role),
                    Ok(None) => this.phase = RolePhase::Missing,
                    Err(error) => this.phase = RolePhase::Failed(error.message().to_owned()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn role_read(&mut self, role: &Role) {
        self.rules = RuleRow::list(&role.rules).into();
        self.phase = RolePhase::Ready;
    }

    /// The Role or ClusterRole the binding names.
    fn role_target(&self, cx: &Context<Self>) -> Option<ResourceRef> {
        let binding = self.binding.as_ref()?;
        let cluster = self.host(cx)?.target().cluster.clone();
        let id = binding.role_id()?;
        let gvk: Gvk = match id.kind {
            RoleKind::Role => oxikube_domain::rbac::RbacKind::Role.gvk(),
            RoleKind::ClusterRole => oxikube_domain::rbac::RbacKind::ClusterRole.gvk(),
        };
        Some(ResourceRef::new(
            cluster,
            gvk,
            id.namespace.as_deref().map(Into::into),
            &*id.name,
        ))
    }

    /// The role's name, as a link: opens the Role or ClusterRole.
    fn open_role(&self, cx: &mut Context<Self>) {
        let (Some(host), Some(target)) = (self.host(cx), self.role_target(cx)) else {
            return;
        };
        host.dispatch(Command::ResourceOpen { target }, cx);
    }

    /// "Edit…": the dialog on this binding.
    fn edit(&self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let target = host.target().clone();
        host.dispatch(
            Command::BindingOpenDialog {
                cluster: Some(target.cluster.clone()),
                cluster_binding: false,
                target: Some(target),
            },
            cx,
        );
    }

    /// "Delete": `binding::Delete` (the guard confirms it).
    fn delete(&self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let target = host.target().clone();
        host.dispatch(Command::BindingDelete { target }, cx);
    }

    fn host(&self, cx: &Context<Self>) -> Option<PanelHost> {
        self.detail.upgrade().map(|detail| detail.read(cx).host())
    }

    /// How the button of `command` stands: hidden, enabled, or disabled with the reason.
    fn block(&self, command: CommandId, cx: &Context<Self>) -> Option<Option<String>> {
        let detail = self.detail.upgrade()?;
        detail.read(cx).command_block(command)
    }
}

mod view;
