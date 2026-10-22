//! [`HelmInstallView`]: the entity of the install tab, its state and its loading.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, UniformListScrollHandle, Window,
};
use oxikube_app::ChartsReadiness;
use oxikube_domain::helm::{ChartRef, ReleaseRef, ValuesDocs, ValuesYaml};
use oxikube_domain::ids::ClusterId;
use oxikube_runtime::spawn_kube;
use oxikube_ui::IconName;
use oxikube_ui::editor::{CodeEditor, CodeEditorOptions, EditorApi as _, EditorState};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::assist::ValuesAssist;
use super::backend::InstallBackend;
use super::form::InstallForm;
use super::model::{HelmInstallEvent, OutputLog, Shown, Stage};

/// The start of the tab's deduplication key: `helm-install:<repo/chart>`, so installing a chart
/// twice shows its open tab.
pub const ITEM_KEY_PREFIX: &str = "helm-install:";

/// The deduplication key of the install tab of `chart`.
pub fn item_key(chart: &ChartRef) -> String {
    format!("{ITEM_KEY_PREFIX}{}", chart.reference())
}

/// What the tab needs from outside. All of it is handed in, so a test builds the view over fakes
/// and the app over its flow.
#[derive(Clone)]
pub struct InstallDeps {
    /// Where the data comes from and the installs go.
    pub backend: Rc<dyn InstallBackend>,
    /// The cluster the tab installs into (its cluster tab's).
    pub cluster: ClusterId,
    /// The cluster's context name, for the header.
    pub cluster_label: SharedString,
}

/// The install tab. See the [module docs](super).
pub struct HelmInstallView {
    pub(super) deps: InstallDeps,
    /// The chart; its `version` is the one asked for (empty: the newest).
    pub(super) chart: ChartRef,
    pub(super) stage: Stage,
    pub(super) release: Entity<InputState>,
    pub(super) namespace: Entity<InputState>,
    pub(super) timeout: Entity<InputState>,
    pub(super) create_namespace: bool,
    pub(super) wait: bool,
    pub(super) version: String,
    pub(super) versions: Vec<String>,
    pub(super) namespaces: Option<Vec<String>>,
    pub(super) editor: Entity<CodeEditor>,
    pub(super) assist: Arc<ValuesAssist>,
    /// Why the chart's default values could not be read.
    pub(super) defaults_problem: Option<String>,
    pub(super) shown: Option<Shown>,
    pub(super) preview_error: Option<String>,
    pub(super) manifest: Entity<EditorState>,
    pub(super) log: OutputLog,
    pub(super) log_scroll: UniformListScrollHandle,
    pub(super) focus: FocusHandle,
    pub(super) load_tasks: Vec<Task<()>>,
    pub(super) preview_task: Option<Task<()>>,
    /// The guard's request, the install itself and the decline; replacing or dropping the install
    /// task cancels it (Cancel).
    pub(super) install_task: Option<Task<()>>,
    pub(super) lines_task: Option<Task<()>>,
    /// The release the last successful install created.
    pub(super) installed: Option<ReleaseRef>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for HelmInstallView {}

impl EventEmitter<HelmInstallEvent> for HelmInstallView {}

impl Focusable for HelmInstallView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for HelmInstallView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(format!("Install {}", self.chart.name)).icon(IconName::Download)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(item_key(&self.chart).into())
    }
}

impl HelmInstallView {
    /// The tab for `chart`: it looks for `helm`, then reads the chart's default values, its
    /// versions and the cluster's namespaces, all off the UI thread.
    pub fn new(
        deps: InstallDeps,
        chart: ChartRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |text: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (text, placeholder) = (text.to_owned(), placeholder.to_owned());
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(text)
            })
        };
        let release = input(&default_release(&chart.name), "my-release", window, cx);
        let namespace = input("default", "default", window, cx);
        let timeout = input("", "300", window, cx);
        let subscriptions = [&release, &namespace, &timeout]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into_iter()
            .collect();
        let assist = ValuesAssist::new();
        let editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(CodeEditorOptions::YAML, window, cx);
            editor.set_assist(assist.clone(), cx);
            editor
        });
        let manifest =
            oxikube_ui::editor::read_only_state(oxikube_ui::editor::ReadOnly::YAML, window, cx);
        let mut view = Self {
            deps,
            version: chart.version.clone(),
            chart,
            stage: Stage::Loading,
            release,
            namespace,
            timeout,
            create_namespace: false,
            wait: false,
            versions: Vec::new(),
            namespaces: None,
            editor,
            assist,
            defaults_problem: None,
            shown: None,
            preview_error: None,
            manifest,
            log: OutputLog::default(),
            log_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            load_tasks: Vec::new(),
            preview_task: None,
            install_task: None,
            lines_task: None,
            installed: None,
            _subscriptions: subscriptions,
        };
        view.load(window, cx);
        view
    }

    fn on_input(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    }

    /// Looks for `helm` and, when it is there, reads what the form needs.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = self.deps.backend.ready();
        let task = cx.spawn_in(window, async move |this, cx| {
            let readiness = spawn_kube(cx, ready).await;
            let usable = !matches!(readiness, Ok(Ok(ChartsReadiness::NoHelm)));
            let loaded = this.update_in(cx, |this, window, cx| {
                this.stage = if usable {
                    Stage::Editing
                } else {
                    Stage::NoHelm
                };
                if usable {
                    this.load_form(window, cx);
                }
                cx.notify();
            });
            loaded.ok();
        });
        self.load_tasks.push(task);
    }

    /// Reads the chart's default values (into the editor and the docs), its versions and the
    /// cluster's namespaces.
    fn load_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = self.deps.backend.defaults(self.chart_at_version());
        let assist = self.assist.clone();
        // The editor is usable while the defaults load: what the user types meanwhile wins.
        let baseline = self.values_version(cx);
        self.load_tasks
            .push(cx.spawn_in(
                window,
                async move |this, cx| match spawn_kube(cx, defaults).await {
                    Ok(Ok(text)) => {
                        let parse_text = text.clone();
                        let docs = cx
                            .background_executor()
                            .spawn(async move { ValuesDocs::from_values_yaml(&parse_text) })
                            .await;
                        assist.set_docs(Arc::new(docs));
                        this.update_in(cx, |this, window, cx| {
                            this.apply_defaults(&text, baseline, window, cx);
                        })
                        .ok();
                    }
                    failed => {
                        let why = match failed {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            Ok(Ok(_)) => unreachable!(),
                        };
                        this.update(cx, |this, cx| {
                            this.defaults_problem = Some(why);
                            cx.notify();
                        })
                        .ok();
                    }
                },
            ));
        let versions = self.deps.backend.versions(self.chart.clone());
        self.load_tasks.push(cx.spawn(async move |this, cx| {
            if let Ok(Ok(found)) = spawn_kube(cx, versions).await {
                this.update(cx, |this, cx| {
                    this.versions = found;
                    cx.notify();
                })
                .ok();
            }
        }));
        let namespaces = self.deps.backend.namespaces(self.deps.cluster.clone());
        self.load_tasks.push(cx.spawn(async move |this, cx| {
            if let Ok(Ok(found)) = spawn_kube(cx, namespaces).await {
                this.update(cx, |this, cx| {
                    this.namespaces = Some(found);
                    cx.notify();
                })
                .ok();
            }
        }));
    }

    /// Puts the chart's default values in the editor, unless it was edited since `baseline` (its
    /// version when the read started): a late read never replaces what the user typed.
    pub(super) fn apply_defaults(
        &mut self,
        text: &str,
        baseline: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.values_version(cx) == baseline {
            self.set_values(text, window, cx);
        }
    }

    /// Puts `text` in the values editor (the chart's defaults, or a test's values).
    pub fn set_values(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.api(window, cx).set_text(text));
        cx.notify();
    }

    /// The chart at the version the form names.
    pub(super) fn chart_at_version(&self) -> ChartRef {
        let mut chart = self.chart.clone();
        chart.version.clone_from(&self.version);
        chart
    }

    /// The form as plain data.
    pub fn form(&self, cx: &App) -> InstallForm {
        InstallForm {
            release: self.release.read(cx).value().to_string(),
            namespace: self.namespace.read(cx).value().to_string(),
            create_namespace: self.create_namespace,
            version: self.version.clone(),
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

    /// Where the tab is.
    pub fn stage(&self) -> &Stage {
        &self.stage
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

    /// The cluster's namespaces, once read.
    pub fn namespaces(&self) -> Option<&[String]> {
        self.namespaces.as_deref()
    }

    /// The release the last successful install created (also sent as
    /// [`HelmInstallEvent::Installed`]).
    pub fn installed(&self) -> Option<&ReleaseRef> {
        self.installed.as_ref()
    }

    /// The output log of the install.
    pub fn log(&self) -> &OutputLog {
        &self.log
    }

    /// The text of the preview's manifest pane.
    pub fn manifest_text(&self, cx: &App) -> String {
        oxikube_ui::editor::text(&self.manifest, cx)
    }

    /// The values assist (hover and completion).
    pub fn assist(&self) -> &Arc<ValuesAssist> {
        &self.assist
    }
}

/// A release name made from the chart's name: lowercase, `-` for anything else, cut to Helm's 53.
fn default_release(chart: &str) -> String {
    let name: String = chart
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    name[..name.len().min(oxikube_domain::helm::MAX_RELEASE_NAME)]
        .trim_end_matches('-')
        .to_owned()
}
