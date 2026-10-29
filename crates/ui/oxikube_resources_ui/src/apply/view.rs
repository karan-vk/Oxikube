//! [`ApplyView`]: the entity of the apply panel, its state and its actions.

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, UniformListScrollHandle, Window,
};
use oxikube_app::apply::{ApplyOptions, ClusterPlan, PlanTarget};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kustomize::KustomizeTool;
use oxikube_domain::manifests::ManifestDoc;
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ports::CancellationToken;
use oxikube_ui::IconName;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::backend::ApplyBackend;
use super::document::EditorOpener;
use super::model::{Model, Stage};
use super::source::{ApplySource, SourceKind};

/// The deduplication key of the tab: opening the panel twice shows the open one.
pub const ITEM_KEY: &str = "apply";

/// What the panel needs from outside. All of it is handed in, so a test builds the view over a
/// script and the app over its services.
#[derive(Clone)]
pub struct ApplyDeps {
    /// Where the panel loads, plans, dry-runs and applies.
    pub backend: Rc<dyn ApplyBackend>,
    /// The cluster the panel applies to.
    pub cluster: ClusterId,
    /// Opens a document in the manifest editor.
    pub editor: EditorOpener,
}

/// The apply panel. See the [module docs](super).
pub struct ApplyView {
    pub(super) deps: ApplyDeps,
    pub(super) cluster_label: String,
    pub(super) model: Model,
    pub(super) source: Option<ApplySource>,
    /// The documents of the last load, kept to plan again under another namespace.
    pub(super) docs: Vec<ManifestDoc>,
    pub(super) plan: Option<ClusterPlan>,
    pub(super) namespace: Entity<InputState>,
    pub(super) namespace_focused: bool,
    /// The kustomize tool, `None` when there is none (the kustomize action is then hidden).
    pub(super) kustomize: Option<KustomizeTool>,
    pub(super) read_only: bool,
    /// The Apply button was pressed: the bar asks the user to confirm.
    pub(super) confirming: bool,
    /// Why the last load or run stopped, when it did not finish.
    pub(super) problem: Option<String>,
    /// Stops the run in flight between objects.
    pub(super) cancel: CancellationToken,
    pub(super) started: Option<Instant>,
    pub(super) table_scroll: UniformListScrollHandle,
    pub(super) errors_scroll: UniformListScrollHandle,
    pub(super) focus: FocusHandle,
    /// The load, the dry run or the apply in flight: replaced by the next, dropped (and so
    /// aborted) with the panel. Never cleared by itself.
    pub(super) work: Option<Task<()>>,
    pub(super) picker: Option<Task<()>>,
    pub(super) reader: Option<Task<()>>,
    /// The directory last loaded for this cluster (from the state port), offered to load again.
    pub(super) last_dir: Option<std::path::PathBuf>,
    pub(super) _tool: Task<()>,
    pub(super) _last: Task<()>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for ApplyView {}

impl Focusable for ApplyView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for ApplyView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Apply").icon(IconName::Upload)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.cancel.cancel();
        self.work = None;
    }
}

impl KeyContextual for ApplyView {
    const KEY_CONTEXT: &'static str = contexts::APPLY_RESULTS;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        context.flag_if(self.namespace_focused, contexts::EDITING);
    }
}

impl ApplyView {
    /// The panel for `deps.cluster`, empty: the namespace field starts at the cluster tab's
    /// selected namespace, the kustomize tool is looked for off the UI thread.
    pub fn new(deps: ApplyDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let selected = deps.backend.selected_namespace(&deps.cluster);
        let namespace = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("default");
            if let Some(selected) = &selected {
                state.set_value(selected.clone(), window, cx);
            }
            state
        });
        let subscription = cx.subscribe_in(&namespace, window, Self::on_namespace);
        let tool = deps.backend.kustomize_tool();
        let detect = cx.spawn(async move |this, cx| {
            let tool = oxikube_runtime::spawn_kube(cx, tool).await.ok().flatten();
            this.update(cx, |this, cx| {
                this.kustomize = tool;
                cx.notify();
            })
            .ok();
        });
        let last = deps.backend.last_directory(deps.cluster.clone());
        let recall = cx.spawn(async move |this, cx| {
            let last = oxikube_runtime::spawn_kube(cx, last).await.ok().flatten();
            this.update(cx, |this, cx| {
                this.last_dir = last;
                cx.notify();
            })
            .ok();
        });
        Self {
            cluster_label: deps.backend.cluster_label(&deps.cluster),
            read_only: deps.backend.read_only(&deps.cluster),
            deps,
            model: Model::new(),
            source: None,
            docs: Vec::new(),
            plan: None,
            namespace,
            namespace_focused: false,
            kustomize: None,
            confirming: false,
            problem: None,
            cancel: CancellationToken::new(),
            started: None,
            table_scroll: UniformListScrollHandle::new(),
            errors_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            work: None,
            picker: None,
            reader: None,
            last_dir: None,
            _tool: detect,
            _last: recall,
            _subscriptions: vec![subscription],
        }
    }

    fn on_namespace(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.replan(cx),
            InputEvent::Focus => {
                self.namespace_focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                self.namespace_focused = false;
                cx.notify();
            }
            InputEvent::Change => {}
        }
    }

    /// The panel's state.
    pub fn model(&self) -> &Model {
        &self.model
    }

    #[cfg(test)]
    pub(super) fn model_mut(&mut self) -> &mut Model {
        &mut self.model
    }

    /// The path being applied.
    pub fn source(&self) -> Option<&ApplySource> {
        self.source.as_ref()
    }

    /// The plan, once the documents are ordered.
    pub fn plan(&self) -> Option<&ClusterPlan> {
        self.plan.as_ref()
    }

    /// The kustomize tool found, `None` while there is none.
    pub fn kustomize_tool(&self) -> Option<&KustomizeTool> {
        self.kustomize.as_ref()
    }

    /// Why Apply is off, in words; `None` when it may be pressed.
    pub fn apply_blocked(&self) -> Option<String> {
        if self.read_only {
            return Some(format!(
                "{} is read-only: turn read-only off to preview or apply",
                self.cluster_label
            ));
        }
        if self.model.busy() {
            return Some("Wait for the current run to finish".to_owned());
        }
        if self.model.stage() == Stage::Load || self.plan.is_none() {
            return Some("Load a file or directory with manifests first".to_owned());
        }
        if self.model.rows().is_empty() {
            return Some("There is nothing to apply".to_owned());
        }
        None
    }

    /// Whether the panel reads the cluster as read-only, as of now.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The namespace field's text, trimmed; `None` when empty (the plan then uses `default`).
    pub fn namespace_text(&self, cx: &App) -> Option<String> {
        let text = self.namespace.read(cx).value().trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    pub(super) fn target(&self, cx: &App) -> PlanTarget {
        PlanTarget {
            cluster: self.deps.cluster.clone(),
            namespace: self.namespace_text(cx),
        }
    }

    pub(super) fn options(&mut self) -> ApplyOptions {
        self.cancel = CancellationToken::new();
        ApplyOptions {
            cancel: self.cancel.clone(),
            ..ApplyOptions::default()
        }
    }

    /// Copies the results as text (names, statuses and the server's messages).
    pub fn copy_results(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.model.as_text()));
    }

    /// Shows only the failed rows, or all.
    pub fn toggle_errors_only(&mut self, cx: &mut Context<Self>) {
        let on = !self.model.errors_only();
        self.model.set_errors_only(on);
        cx.notify();
    }

    /// Moves the cursor through the rows.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.model.step(delta);
        self.scroll_to_selected();
        cx.notify();
    }

    /// Selects row `index` (a click).
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.model.select(index);
        cx.notify();
    }

    /// Opens or closes the full message of row `index`.
    pub fn toggle_expanded(&mut self, index: usize, cx: &mut Context<Self>) {
        self.model.toggle_expanded(index);
        cx.notify();
    }

    /// Opens or closes the selected row.
    pub fn toggle_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.model.selected() {
            self.toggle_expanded(index, cx);
        }
    }

    fn scroll_to_selected(&mut self) {
        let Some(selected) = self.model.selected() else {
            return;
        };
        let position =
            self.model.lines().iter().position(
                |line| matches!(line, super::model::Line::Row(index) if *index == selected),
            );
        if let Some(position) = position {
            self.table_scroll
                .scroll_to_item(position, gpui::ScrollStrategy::Center);
        }
    }

    /// Whether the source is a kustomize directory.
    pub fn is_kustomize(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.kind == SourceKind::Kustomize)
    }
}
