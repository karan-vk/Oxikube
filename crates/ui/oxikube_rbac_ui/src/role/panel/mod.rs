//! [`RolePanel`]: a Role's or ClusterRole's rules in the detail, with the aggregation it comes
//! from and the buttons that edit or delete it.

use std::sync::Arc;

use gpui::{Context, Subscription, Task, WeakEntity};
use oxikube_app::role_ops::RoleLinks;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::rbac::{Role, RoleId};
use oxikube_resources_ui::detail::{DetailView, PanelHost};
use oxikube_runtime::spawn_kube;

use super::rows::RuleRow;

/// What the panel knows of the ClusterRoles an aggregated role collects from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sources {
    /// Not an aggregated role.
    None,
    /// Being read.
    Loading,
    /// The ClusterRoles, sorted.
    Ready(Vec<RoleId>),
    /// The read failed (no right to list ClusterRoles, offline).
    Failed(String),
}

/// What the rows were made from: another version of the object makes them again.
#[derive(Clone, PartialEq, Eq)]
struct Source {
    uid: Option<Arc<str>>,
    version: Option<Arc<str>>,
}

/// The Role panel. See the module docs.
pub struct RolePanel {
    detail: WeakEntity<DetailView>,
    role: Option<Arc<Role>>,
    rows: Arc<[RuleRow]>,
    sources: Sources,
    built_from: Option<Source>,
    /// The aggregation sources being read; a newer read replaces (and so cancels) it.
    reading: Option<Task<()>>,
    /// The delete button is counting bindings.
    counting: bool,
    /// The count being made; replaced, never cleared from inside.
    count_task: Option<Task<()>>,
    _detail: Option<Subscription>,
}

impl RolePanel {
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
            role: None,
            rows: Arc::from(Vec::new()),
            sources: Sources::None,
            built_from: None,
            reading: None,
            counting: false,
            count_task: None,
            _detail: subscription,
        }
    }

    /// The role, once read.
    pub fn role(&self) -> Option<&Role> {
        self.role.as_deref()
    }

    /// The rows of the table.
    pub fn rows(&self) -> &[RuleRow] {
        &self.rows
    }

    /// The aggregation sources.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// Whether the delete button is counting the role's bindings.
    pub fn is_counting(&self) -> bool {
        self.counting
    }

    /// Builds the rows when the object has a new version.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (role, source, host) = {
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
            let Ok(role) = Role::from_resource(resource) else {
                return;
            };
            (role, source, view.host())
        };
        self.rows = RuleRow::list(&role.rules).into();
        self.built_from = Some(source);
        self.read_sources(&role, &host, cx);
        self.role = Some(Arc::new(role));
        cx.notify();
    }

    /// Reads the ClusterRoles an aggregated role collects from, off the UI thread.
    fn read_sources(&mut self, role: &Role, host: &PanelHost, cx: &mut Context<Self>) {
        if role.aggregation.is_none() {
            self.sources = Sources::None;
            self.reading = None;
            return;
        }
        self.sources = Sources::Loading;
        let links = RoleLinks::new(host.sessions().clone());
        let cluster = host.target().cluster.clone();
        let role = role.clone();
        let read = spawn_kube(cx, async move { links.contributors(&cluster, &role).await });
        self.reading = Some(cx.spawn(async move |this, cx| {
            let sources = match read.await {
                Ok(Ok(found)) => Sources::Ready(found),
                Ok(Err(error)) => Sources::Failed(error.message().to_owned()),
                Err(error) => Sources::Failed(error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.sources = sources;
                cx.notify();
            })
            .ok();
        }));
    }

    /// "Edit rules…": the dialog on this role.
    fn edit(&self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let target = host.target().clone();
        host.dispatch(
            Command::RoleOpenDialog {
                cluster: Some(target.cluster.clone()),
                cluster_role: false,
                target: Some(target),
            },
            cx,
        );
    }

    /// "Delete": counts the bindings that name the role, then sends `role::Delete` with the count
    /// (the guard's confirmation warns about them). The count is a read off the UI thread; if it
    /// fails the delete goes ahead without one.
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let Some(role) = self.role.as_ref().map(|role| role.id.clone()) else {
            return;
        };
        if self.counting {
            return;
        }
        self.counting = true;
        let links = RoleLinks::new(host.sessions().clone());
        let target = host.target().clone();
        let cluster = target.cluster.clone();
        let count = spawn_kube(
            cx,
            async move { links.binding_count(&cluster, &role).await },
        );
        self.count_task = Some(cx.spawn(async move |this, cx| {
            let bindings = count.await.ok().and_then(Result::ok);
            this.update(cx, |this, cx| {
                this.counting = false;
                if let Some(host) = this.host(cx) {
                    host.dispatch(Command::RoleDelete { target, bindings }, cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn host(&self, cx: &Context<Self>) -> Option<PanelHost> {
        self.detail.upgrade().map(|detail| detail.read(cx).host())
    }

    /// How the button of `command` stands: hidden (no RBAC grant for what it needs), enabled, or
    /// disabled with the reason.
    fn block(&self, command: CommandId, cx: &Context<Self>) -> Option<Option<String>> {
        let detail = self.detail.upgrade()?;
        detail.read(cx).command_block(command)
    }
}

mod view;
