//! [`HelmReposView`]: the entity of the repositories page, its actions and its workspace item.

use std::rc::Rc;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use oxikube_app::RepoSnapshot;
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;
use oxikube_settings::{HelmSettings, Settings as _};
use oxikube_ui::IconName;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent};

use super::backend::ReposBackend;
use super::form::AddForm;
use super::model::{ReposModel, default_repos};

/// The deduplication key of the tab: opening the page twice shows the open one.
pub const ITEM_KEY: &str = "helm-repositories";

/// What the page needs from outside. All of it is handed in, so a test builds the view over fakes
/// and the app builds it over its service and bus.
#[derive(Clone)]
pub struct ReposDeps {
    /// Where the state comes from.
    pub backend: Rc<dyn ReposBackend>,
    /// Where the buttons' commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// The Helm repositories page. See the [module docs](super).
pub struct HelmReposView {
    pub(super) deps: ReposDeps,
    pub(super) snapshot: RepoSnapshot,
    pub(super) model: ReposModel,
    pub(super) skip_tls: bool,
    pub(super) form_open: bool,
    /// The repository whose add was sent; the form clears when it shows up in the list.
    pub(super) pending_add: Option<String>,
    /// The repository whose remove button was clicked once.
    pub(super) confirm_remove: Option<String>,
    pub(super) name: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) ca_file: Entity<InputState>,
    pub(super) focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    /// The initial read of Helm's list; dropped (and so aborted) with the page.
    _load: Option<Task<()>>,
    /// Redraws when the service says its state changed.
    _watch: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for HelmReposView {}

impl Focusable for HelmReposView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for HelmReposView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Helm Repositories").icon(IconName::Package)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}

impl HelmReposView {
    /// The page, showing the state the backend has now and following its changes. It reads Helm's
    /// list, off the UI thread.
    pub fn new(deps: ReposDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snapshot = deps.backend.snapshot();
        let model = ReposModel::build(&snapshot, deps.backend.now(), &default_repos(cx));
        let input = |cx: &mut Context<Self>, window: &mut Window, masked: bool| {
            cx.new(|cx| InputState::new(window, cx).masked(masked))
        };
        let (name, url, username, ca_file) = (
            input(cx, window, false),
            input(cx, window, false),
            input(cx, window, false),
            input(cx, window, false),
        );
        let password = input(cx, window, true);
        let mut subscriptions: Vec<Subscription> = [&name, &url, &username, &password, &ca_file]
            .map(|input| cx.subscribe_in(input, window, Self::on_input))
            .into();
        // `helm.default_repos` hot-reloads into the quick-add row.
        subscriptions.push(HelmSettings::observe_in(cx, |this: &mut Self, cx| {
            this.model =
                ReposModel::build(&this.snapshot, this.deps.backend.now(), &default_repos(cx));
            cx.notify();
        }));
        let mut changes = deps.backend.changes();
        let watch = cx.spawn_in(window, async move |this, cx| {
            while changes.next().await.is_some() {
                if this
                    .update_in(cx, |this, window, cx| this.sync(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        // Always read: it is also how a missing `helm` is found out (the adapter detects on first
        // use), without a process on the UI thread or at start-up.
        let refresh = deps.backend.refresh();
        let load = Some(cx.spawn(async move |_, cx| {
            // The outcome lands in the snapshot; the watcher redraws.
            spawn_kube(cx, refresh).await.ok();
        }));
        Self {
            deps,
            snapshot,
            model,
            skip_tls: false,
            form_open: false,
            pending_add: None,
            confirm_remove: None,
            name,
            url,
            username,
            password,
            ca_file,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _load: load,
            _watch: watch,
            _subscriptions: subscriptions,
        }
    }

    /// What the page shows now.
    pub fn model(&self) -> &ReposModel {
        &self.model
    }

    /// The add form's text, read from the inputs.
    pub fn form(&self, cx: &App) -> AddForm {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        AddForm {
            name: text(&self.name),
            url: text(&self.url),
            username: text(&self.username),
            password: text(&self.password),
            ca_file: text(&self.ca_file),
            skip_tls: self.skip_tls,
        }
    }

    /// Whether the add form is showing.
    pub fn form_open(&self) -> bool {
        self.form_open
    }

    /// Reads the service's state again; redraws only when what is shown differs. Also finishes an
    /// add: once the new repository is listed the form clears, and when the add failed it stays so
    /// the person can fix it.
    pub(super) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.snapshot = self.deps.backend.snapshot();
        let model = ReposModel::build(&self.snapshot, self.deps.backend.now(), &default_repos(cx));
        let mut changed = model != self.model;
        self.model = model;
        // An add is done when Helm lists the repository and the service says it was added; until
        // then (a failure, a refusal, the command not yet started) the form stays as it is.
        if let Some(name) = self.pending_add.clone()
            && !self.snapshot.is_busy()
            && self.snapshot.row(&name).is_some()
            && self.snapshot.notice.as_deref() == Some(format!("Added repository {name}").as_str())
        {
            self.pending_add = None;
            self.deps.backend.discard_password(&name);
            self.clear_form(window, cx);
            self.form_open = false;
            changed = true;
        }
        if self
            .confirm_remove
            .as_ref()
            .is_some_and(|n| self.snapshot.row(n).is_none())
        {
            self.confirm_remove = None;
            changed = true;
        }
        if changed {
            cx.notify();
        }
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
                // Editing the form again withdraws the add this page was waiting for.
                if let Some(name) = self.pending_add.take() {
                    self.deps.backend.discard_password(&name);
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.submit(window, cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Shows or hides the add form.
    pub fn toggle_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.cli_available {
            return;
        }
        self.form_open = !self.form_open;
        if self.form_open {
            self.name.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Ticks or unticks "skip TLS verification".
    pub fn set_skip_tls(&mut self, skip: bool, cx: &mut Context<Self>) {
        self.skip_tls = skip;
        cx.notify();
    }

    /// Fills the form with the `index`th quick-add suggestion.
    pub fn use_suggestion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(suggestion) = self.model.suggestions.get(index).cloned() else {
            return;
        };
        self.form_open = true;
        self.set_input(&self.name.clone(), &suggestion.name, window, cx);
        self.set_input(&self.url.clone(), &suggestion.url, window, cx);
        cx.notify();
    }

    fn set_input(
        &mut self,
        input: &Entity<InputState>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    fn clear_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.name,
            &self.url,
            &self.username,
            &self.password,
            &self.ca_file,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.skip_tls = false;
    }

    /// Sends `helm::RepoAdd`. The password is handed over in memory first and cleared from the
    /// field; the command itself carries none.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(cx);
        if !self.model.cli_available || self.model.busy || !form.can_submit(&self.snapshot) {
            return;
        }
        let command = form.command();
        let name = form.name.trim().to_owned();
        if let Some(password) = form.password() {
            self.deps.backend.stage_password(&name, password);
        }
        self.set_input(&self.password.clone(), "", window, cx);
        self.pending_add = Some(name);
        self.deps.dispatcher.dispatch(command, cx);
        cx.notify();
    }

    /// Refreshes every repository: `helm::RepoUpdate` with no names.
    pub fn update_all(&mut self, cx: &mut Context<Self>) {
        self.update(Vec::new(), cx);
    }

    /// Refreshes repository `name`.
    pub fn update_one(&mut self, name: &str, cx: &mut Context<Self>) {
        self.update(vec![name.to_owned()], cx);
    }

    fn update(&mut self, names: Vec<String>, cx: &mut Context<Self>) {
        if !self.model.cli_available || self.model.busy || self.model.rows.is_empty() {
            return;
        }
        self.deps
            .dispatcher
            .dispatch(Command::HelmRepoUpdate { names }, cx);
    }

    /// Removes `name`: the first click asks, the second sends `helm::RepoRemove`.
    pub fn remove(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.model.cli_available || self.model.busy {
            return;
        }
        if self.confirm_remove.as_deref() != Some(name) {
            self.confirm_remove = Some(name.to_owned());
            cx.notify();
            return;
        }
        self.confirm_remove = None;
        self.deps.dispatcher.dispatch(
            Command::HelmRepoRemove {
                name: name.to_owned(),
            },
            cx,
        );
        cx.notify();
    }

    /// Takes the first-click confirmation back.
    pub fn cancel_remove(&mut self, cx: &mut Context<Self>) {
        if self.confirm_remove.take().is_some() {
            cx.notify();
        }
    }
}
