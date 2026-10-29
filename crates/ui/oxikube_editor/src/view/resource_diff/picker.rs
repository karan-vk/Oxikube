//! [`DiffTargetDialog`]: the "Diff with..." picker. It asks for the second object of a
//! comparison: the cluster (every cluster of the catalog, the first one is the object's own),
//! the namespace and the name. The kind is fixed by the object the comparison started from, so
//! the second object is always of that kind (the version is the target's).

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Task, Window,
    div, px,
};
use oxikube_app::ClusterCatalog;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::CommandDispatcher;
use oxikube_workspace::modal::{DIALOG_KEY_CONTEXT, ModalPlacement, ModalView};
use std::rc::Rc;

/// The picker for the second object of a comparison.
pub struct DiffTargetDialog {
    target: ResourceRef,
    clusters: Vec<(ClusterId, String)>,
    cluster: ClusterId,
    namespace: Option<Entity<InputState>>,
    name: Entity<InputState>,
    /// What is wrong with the choice, until the next edit.
    problem: Option<&'static str>,
    dispatcher: Rc<dyn CommandDispatcher>,
    focus: FocusHandle,
    _catalog: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<DismissEvent> for DiffTargetDialog {}

impl Focusable for DiffTargetDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for DiffTargetDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }
}

impl DiffTargetDialog {
    /// A picker over `target` (the left side). `catalog` lists the clusters to choose from; the
    /// target's own cluster is offered without it.
    pub fn new(
        target: ResourceRef,
        own_label: String,
        catalog: Option<ClusterCatalog>,
        dispatcher: Rc<dyn CommandDispatcher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = value.to_owned();
            cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(value, window, cx);
                state
            })
        };
        let namespace = target
            .namespace()
            .map(|namespace| input(namespace, window, cx));
        let name = input(&target.name, window, cx);
        let mut subscriptions = Vec::new();
        for state in namespace.iter().chain([&name]) {
            subscriptions.push(cx.subscribe_in(state, window, Self::on_input));
        }
        let catalog_task = catalog.map(|catalog| {
            cx.spawn(async move |this, cx| {
                let loaded =
                    oxikube_runtime::spawn_kube(cx, async move { catalog.load().await }).await;
                if let Ok(Ok(entries)) = loaded {
                    this.update(cx, |dialog, cx| dialog.set_clusters(entries, cx))
                        .ok();
                }
            })
        });
        let cluster = target.cluster.clone();
        Self {
            clusters: vec![(cluster.clone(), own_label)],
            cluster,
            target,
            namespace,
            name,
            problem: None,
            dispatcher,
            focus: cx.focus_handle(),
            _catalog: catalog_task,
            _subscriptions: subscriptions,
        }
    }

    fn set_clusters(
        &mut self,
        entries: Vec<oxikube_app::catalog::CatalogEntry>,
        cx: &mut Context<Self>,
    ) {
        let own = self.clusters.first().cloned();
        let mut clusters: Vec<(ClusterId, String)> = entries
            .iter()
            .map(|entry| (entry.id().clone(), entry.name().to_owned()))
            .collect();
        if let Some(own) = own
            && !clusters.iter().any(|(id, _)| *id == own.0)
        {
            clusters.insert(0, own);
        }
        self.clusters = clusters;
        cx.notify();
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.problem.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => self.submit(window, cx),
            _ => {}
        }
    }

    /// The clusters on offer.
    pub fn clusters(&self) -> Vec<&ClusterId> {
        self.clusters.iter().map(|(id, _)| id).collect()
    }

    /// The cluster chosen.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// Chooses `cluster` for the second object.
    pub fn select_cluster(&mut self, cluster: ClusterId, cx: &mut Context<Self>) {
        self.cluster = cluster;
        cx.notify();
    }

    /// The second object as typed, `None` (with the problem set) when the name is empty.
    fn other(&mut self, cx: &App) -> Option<ResourceRef> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.problem = Some("Enter the name of the object to compare with");
            return None;
        }
        let namespace = match &self.namespace {
            Some(input) => {
                let namespace = input.read(cx).value().trim().to_owned();
                if namespace.is_empty() {
                    self.problem = Some("Enter the namespace of the object to compare with");
                    return None;
                }
                Some(namespace.into())
            }
            None => None,
        };
        Some(ResourceRef::new(
            self.cluster.clone(),
            self.target.gvk.clone(),
            namespace,
            name,
        ))
    }

    /// Sends the comparison (`resource::DiffWith` with both objects) and closes.
    pub fn submit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(other) = self.other(cx) else {
            cx.notify();
            return;
        };
        self.dispatcher.dispatch(
            Command::ResourceDiffWith {
                target: self.target.clone(),
                other: Some(other),
                include_status: false,
                compare_identity: false,
            },
            cx,
        );
        cx.emit(DismissEvent);
    }

    /// What is wrong with the choice, until the next edit.
    pub fn problem(&self) -> Option<&'static str> {
        self.problem
    }

    /// Fills the namespace field (the picker for a cluster-scoped kind has none).
    pub fn set_namespace(&mut self, namespace: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.namespace {
            let namespace = namespace.to_owned();
            input.update(cx, |input, cx| input.set_value(namespace, window, cx));
        }
    }

    /// Fills the name field.
    pub fn set_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.to_owned();
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
    }

    /// Closes without comparing.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}

impl Render for DiffTargetDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let clusters = self.clusters.clone();
        let chosen = self.cluster.clone();
        let label = |text: &'static str| {
            div()
                .text_size(u(px(12.)))
                .text_color(colors.text_muted)
                .child(text)
        };
        v_flex()
            .id("diff-target-dialog")
            .debug_selector(|| "diff-target-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(520.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.submit(window, cx)))
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child(format!("Diff {} with…", self.target.name))),
            )
            .child(
                v_flex().gap(u(px(4.))).child(label("Cluster")).child(
                    h_flex()
                        .id("diff-target-clusters")
                        .debug_selector(|| "diff-target-clusters".into())
                        .flex_wrap()
                        .gap(u(px(4.)))
                        .children(clusters.into_iter().enumerate().map(
                            |(ix, (cluster, title))| {
                                let selected = cluster == chosen;

                                Button::new(("diff-target-cluster", ix))
                                    .label(title)
                                    .small()
                                    .selected(selected)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_cluster(cluster.clone(), cx)
                                    }))
                            },
                        )),
                ),
            )
            .children(self.namespace.as_ref().map(|input| {
                v_flex().gap(u(px(4.))).child(label("Namespace")).child(
                    div()
                        .debug_selector(|| "diff-target-namespace".into())
                        .child(Input::new(input)),
                )
            }))
            .child(
                v_flex().gap(u(px(4.))).child(label("Name")).child(
                    div()
                        .debug_selector(|| "diff-target-name".into())
                        .child(Input::new(&self.name)),
                ),
            )
            .children(self.problem.map(|problem| {
                div()
                    .debug_selector(|| "diff-target-problem".into())
                    .text_size(u(px(12.)))
                    .text_color(colors.error)
                    .child(problem)
            }))
            .child(
                DialogFooter::new()
                    .child(
                        Button::new("diff-target-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        div().debug_selector(|| "diff-target-compare".into()).child(
                            Button::new("diff-target-compare")
                                .label("Compare")
                                .primary()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                    ),
            )
    }
}
