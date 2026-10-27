//! [`HelmUpgradeView`]: the entity of the upgrade tab, its state and its loading.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, UniformListScrollHandle, Window,
};
use oxikube_app::CurrentRelease;
use oxikube_domain::helm::{Release, ReleaseRef, ValuesYaml};
use oxikube_domain::ids::ClusterId;
use oxikube_editor::diff::ManifestDiff;
use oxikube_ui::IconName;
use oxikube_ui::editor::{
    CodeEditor, CodeEditorEvent, CodeEditorOptions, EditorApi as _, EditorState,
};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::backend::{ChartSource, UpgradeBackend};
use super::form::UpgradeForm;
use super::model::{HelmUpgradeEvent, Pane, Shown, Stage};
use crate::install::{OutputLog, ValuesAssist};

/// The start of the tab's deduplication key: `helm-upgrade:<namespace>/<release>` (the picker's
/// tab is `helm-upgrade:`), so upgrading a release twice shows its open tab.
pub const ITEM_KEY_PREFIX: &str = "helm-upgrade:";

/// How long an edit rests before the open diffs are computed again.
const DIFF_DEBOUNCE: Duration = Duration::from_millis(150);

/// The deduplication key of the upgrade tab of `release` (`None`: the picker).
pub fn item_key(release: Option<&ReleaseRef>) -> String {
    match release {
        Some(release) => format!("{ITEM_KEY_PREFIX}{release}"),
        None => ITEM_KEY_PREFIX.to_owned(),
    }
}

/// What the tab needs from outside. All of it is handed in, so a test builds the view over fakes
/// and the app over its flow.
#[derive(Clone)]
pub struct UpgradeDeps {
    /// Where the data comes from and the upgrades go.
    pub backend: Rc<dyn UpgradeBackend>,
    /// The cluster the tab upgrades in (its cluster tab's).
    pub cluster: ClusterId,
    /// The cluster's context name, for the header.
    pub cluster_label: SharedString,
}

/// The upgrade tab. See the [module docs](super).
pub struct HelmUpgradeView {
    pub(super) deps: UpgradeDeps,
    /// The release; `None` until one is picked.
    pub(super) target: Option<ReleaseRef>,
    pub(super) stage: Stage,
    pub(super) pane: Pane,
    pub(super) releases: Option<Vec<Release>>,
    pub(super) releases_scroll: UniformListScrollHandle,
    pub(super) current: Option<CurrentRelease>,
    /// The release's current values (what the diff is against).
    pub(super) current_values: Arc<str>,
    pub(super) sources: Vec<ChartSource>,
    pub(super) chart: Entity<InputState>,
    pub(super) timeout: Entity<InputState>,
    pub(super) version: String,
    pub(super) reuse_values: bool,
    pub(super) wait: bool,
    pub(super) editor: Entity<CodeEditor>,
    pub(super) assist: Arc<ValuesAssist>,
    pub(super) current_defaults: Option<Arc<str>>,
    pub(super) target_defaults: Option<Arc<str>>,
    /// Why the chart's default values could not be read.
    pub(super) defaults_problem: Option<String>,
    pub(super) values_diff: Option<Arc<ManifestDiff>>,
    pub(super) defaults_diff: Option<Arc<ManifestDiff>>,
    pub(super) diff_scroll: UniformListScrollHandle,
    pub(super) shown: Option<Shown>,
    pub(super) preview_error: Option<String>,
    pub(super) manifest: Entity<EditorState>,
    pub(super) log: OutputLog,
    pub(super) log_scroll: UniformListScrollHandle,
    pub(super) focus: FocusHandle,
    pub(super) load_tasks: Vec<Task<()>>,
    pub(super) defaults_tasks: Vec<Task<()>>,
    pub(super) diff_task: Option<Task<()>>,
    pub(super) preview_task: Option<Task<()>>,
    /// The guard's request, the upgrade itself and the decline; replacing or dropping the upgrade
    /// task cancels it (Cancel).
    pub(super) upgrade_task: Option<Task<()>>,
    pub(super) lines_task: Option<Task<()>>,
    /// The release the last successful upgrade changed.
    pub(super) upgraded: Option<ReleaseRef>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for HelmUpgradeView {}

impl EventEmitter<HelmUpgradeEvent> for HelmUpgradeView {}

impl Focusable for HelmUpgradeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for HelmUpgradeView {
    fn tab_content(&self, _: &App) -> TabContent {
        let title = match &self.target {
            Some(release) => format!("Upgrade {}", release.name),
            None => "Upgrade release".to_owned(),
        };
        TabContent::new(title).icon(IconName::Download)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(item_key(self.target.as_ref()).into())
    }
}

impl HelmUpgradeView {
    /// The tab for `target` (the picker when `None`): it looks for `helm`, then reads the release
    /// (its chart, its user-supplied values), the charts that can replace it and their default
    /// values, all off the UI thread.
    pub fn new(
        deps: UpgradeDeps,
        target: Option<ReleaseRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let chart = input("repo/chart", window, cx);
        let timeout = input("300", window, cx);
        let mut subscriptions: Vec<Subscription> = [&chart, &timeout]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into_iter()
            .collect();
        let assist = ValuesAssist::new();
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(CodeEditorOptions::YAML, window, cx);
            editor.set_assist(assist.clone(), cx);
            editor
        });
        subscriptions.push(cx.subscribe_in(&editor, window, Self::on_editor));
        let manifest =
            oxikube_ui::editor::read_only_state(oxikube_ui::editor::ReadOnly::YAML, window, cx);
        let mut view = Self {
            deps,
            target,
            stage: Stage::Loading,
            pane: Pane::default(),
            releases: None,
            releases_scroll: UniformListScrollHandle::new(),
            current: None,
            current_values: Arc::from(""),
            sources: Vec::new(),
            chart,
            timeout,
            version: String::new(),
            reuse_values: false,
            wait: false,
            editor,
            assist,
            current_defaults: None,
            target_defaults: None,
            defaults_problem: None,
            values_diff: None,
            defaults_diff: None,
            diff_scroll: UniformListScrollHandle::new(),
            shown: None,
            preview_error: None,
            manifest,
            log: OutputLog::default(),
            log_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            load_tasks: Vec::new(),
            defaults_tasks: Vec::new(),
            diff_task: None,
            preview_task: None,
            upgrade_task: None,
            lines_task: None,
            upgraded: None,
            _subscriptions: subscriptions,
        };
        view.load(window, cx);
        view
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => cx.notify(),
            // The chart was typed: its versions and defaults are those of the new source.
            InputEvent::PressEnter { .. } | InputEvent::Blur => self.source_changed(window, cx),
            InputEvent::Focus => {}
        }
    }

    fn on_editor(
        &mut self,
        _: &Entity<CodeEditor>,
        event: &CodeEditorEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, CodeEditorEvent::Changed { .. }) {
            self.refresh_values_diff(cx);
            cx.notify();
        }
    }

    /// Puts `text` in the values editor.
    pub fn set_values(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.api(window, cx).set_text(text));
        cx.notify();
    }

    /// The form as plain data.
    pub fn form(&self, cx: &App) -> UpgradeForm {
        UpgradeForm {
            chart: self.chart.read(cx).value().to_string(),
            version: self.version.clone(),
            reuse_values: self.reuse_values,
            wait: self.wait,
            timeout: self.timeout.read(cx).value().to_string(),
        }
    }

    /// The values editor's text.
    pub fn values(&self, cx: &App) -> ValuesYaml {
        ValuesYaml::new(&*self.editor.read(cx).snapshot(cx).to_text())
    }

    /// The values editor's version (grows with every edit).
    pub(super) fn values_version(&self, cx: &App) -> u64 {
        self.editor.read(cx).version()
    }

    /// Computes the values diff again once the editor rests, on the background executor.
    pub(super) fn refresh_values_diff(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.editor.read(cx).snapshot(cx);
        let current = self.current_values.clone();
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIFF_DEBOUNCE).await;
            let diff = cx
                .background_executor()
                .spawn(async move { super::diffs::values_diff(&current, &snapshot.to_text()) })
                .await;
            this.update(cx, |this, cx| {
                this.values_diff = Some(Arc::new(diff));
                cx.notify();
            })
            .ok();
        }));
    }

    /// Where the tab is.
    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    /// What the right column shows while editing.
    pub fn pane(&self) -> Pane {
        self.pane
    }

    /// The release being upgraded, once named.
    pub fn target(&self) -> Option<&ReleaseRef> {
        self.target.as_ref()
    }

    /// What the release runs now, once read.
    pub fn current(&self) -> Option<&CurrentRelease> {
        self.current.as_ref()
    }

    /// The preview shown, if it still matches the form and the values.
    pub fn fresh_preview(&self, cx: &App) -> Option<&Shown> {
        let form = self.form(cx);
        let version = self.values_version(cx);
        self.shown.as_ref().filter(|s| s.is_fresh(&form, version))
    }

    /// Why the last preview failed, if it did.
    pub fn preview_error(&self) -> Option<&str> {
        self.preview_error.as_deref()
    }

    /// The release the last successful upgrade changed (also sent as
    /// [`HelmUpgradeEvent::Upgraded`]).
    pub fn upgraded(&self) -> Option<&ReleaseRef> {
        self.upgraded.as_ref()
    }

    /// The output log of the upgrade.
    pub fn log(&self) -> &OutputLog {
        &self.log
    }

    /// The charts the repositories offer under the release's chart name.
    pub fn sources(&self) -> &[ChartSource] {
        &self.sources
    }

    /// The values diff (edited against current), once computed.
    pub fn values_diff(&self) -> Option<&ManifestDiff> {
        self.values_diff.as_deref()
    }

    /// The chart-defaults diff (current version against target version), once computed.
    pub fn defaults_diff(&self) -> Option<&ManifestDiff> {
        self.defaults_diff.as_deref()
    }

    /// The text of the preview's manifest pane.
    pub fn manifest_text(&self, cx: &App) -> String {
        oxikube_ui::editor::text(&self.manifest, cx)
    }
}
