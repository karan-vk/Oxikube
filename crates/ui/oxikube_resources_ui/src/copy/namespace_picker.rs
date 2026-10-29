//! [`NamespacePicker`]: the second step, which namespace of the chosen cluster to copy into. The
//! cluster is connected on demand (the picker says so while it waits). A name that is not in the
//! list can be typed: the review then offers to create the namespace as its own guarded step.

use std::sync::Arc;

use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Task, Window, div, px,
};
use oxikube_palette::picker::fuzzy::{self, StringMatch, StringMatchCandidate};
use oxikube_palette::{Picker, PickerDelegate, match_label};
use oxikube_runtime::spawn_kube;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::flow::{CopyDraft, CopyFlow};

/// The namespace chooser: a [`Picker`] whose matches are the namespaces of the target cluster.
pub type NamespacePicker = Picker<NamespacePickerDelegate>;

/// The picker's width (before UI zoom).
const WIDTH: f32 = 480.;

/// Where the namespace list is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceList {
    /// Connecting and reading.
    Loading,
    /// Read.
    Loaded,
    /// Could not be read (not connected, or no right to list namespaces): the message. A name
    /// can still be typed.
    Failed(String),
}

/// Asks which namespace to copy into. Typing filters; a name not in the list is offered as the
/// last row.
pub struct NamespacePickerDelegate {
    flow: CopyFlow,
    draft: CopyDraft,
    names: Vec<String>,
    candidates: Arc<[StringMatchCandidate]>,
    matches: Vec<StringMatch>,
    /// The typed query, when it names no namespace of the list: offered as the last row.
    typed: Option<String>,
    selected: usize,
    state: NamespaceList,
    started: bool,
    /// The connect and read; owned here, so it ends with the picker.
    load: Option<Task<()>>,
}

impl NamespacePickerDelegate {
    pub(super) fn picker(
        flow: CopyFlow,
        draft: CopyDraft,
        window: &mut Window,
        cx: &mut Context<NamespacePicker>,
    ) -> NamespacePicker {
        let delegate = Self {
            flow,
            draft,
            names: Vec::new(),
            candidates: Arc::from(Vec::new()),
            matches: Vec::new(),
            typed: None,
            selected: 0,
            state: NamespaceList::Loading,
            started: false,
            load: None,
        };
        Picker::uniform_list(delegate, window, cx).width(px(WIDTH))
    }

    /// Where the list is.
    pub fn state(&self) -> &NamespaceList {
        &self.state
    }

    /// The namespaces of the cluster, once read.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The namespace the selected row stands for.
    pub fn selected(&self) -> Option<&str> {
        match self.matches.get(self.selected) {
            Some(found) => self.names.get(found.candidate_id).map(String::as_str),
            None if self.selected == self.matches.len() => self.typed.as_deref(),
            None => None,
        }
    }

    fn arrived(&mut self, names: Result<Vec<String>, String>) {
        match names {
            Ok(names) => {
                self.candidates = names
                    .iter()
                    .enumerate()
                    .map(|(ix, name)| StringMatchCandidate::new(ix, name.clone()))
                    .collect();
                self.names = names;
                self.state = NamespaceList::Loaded;
            }
            Err(message) => self.state = NamespaceList::Failed(message),
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<NamespacePicker>) {
        self.started = true;
        let Some(cluster) = self.draft.cluster.as_ref().map(|choice| choice.id.clone()) else {
            return;
        };
        let service = self.flow.service().clone();
        let read = spawn_kube(cx, async move { service.namespaces(&cluster).await });
        self.load = Some(cx.spawn_in(window, async move |picker, cx| {
            let names = match read.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            picker
                .update_in(cx, |picker, window, cx| {
                    picker.delegate.arrived(names);
                    picker.refresh(window, cx);
                })
                .ok();
        }));
    }

    fn cluster_name(&self) -> &str {
        self.draft
            .cluster
            .as_ref()
            .map_or("the cluster", |choice| choice.name.as_str())
    }
}

impl PickerDelegate for NamespacePickerDelegate {
    type ListItem = gpui::Div;

    fn match_count(&self) -> usize {
        self.matches.len() + usize::from(self.typed.is_some())
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<NamespacePicker>) {
        self.selected = ix;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Copy into which namespace?".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some(match &self.state {
            NamespaceList::Loading => "Connecting and reading the namespaces…".into(),
            NamespaceList::Loaded => "No such namespace".into(),
            NamespaceList::Failed(message) => {
                format!("Could not list the namespaces ({message}). Type the name.").into()
            }
        })
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<NamespacePicker>,
    ) -> Task<()> {
        if !self.started {
            self.start(window, cx);
        }
        self.matches = fuzzy::match_strings(&self.candidates, &query, usize::MAX);
        let query = query.trim();
        self.typed = (!query.is_empty()
            && self.state != NamespaceList::Loading
            && !self.names.iter().any(|name| name == query))
        .then(|| query.to_owned());
        // The blank query starts on the source's own namespace when the cluster has it.
        self.selected = if query.is_empty() {
            let own = self.draft.source.namespace();
            self.matches
                .iter()
                .position(|found| own == self.names.get(found.candidate_id).map(String::as_str))
                .unwrap_or(0)
        } else {
            0
        };
        Task::ready(())
    }

    /// Chooses the selected namespace (both confirms do the same): the review follows.
    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<NamespacePicker>,
    ) {
        let Some(namespace) = self.selected().map(str::to_owned) else {
            return;
        };
        let (flow, draft) = (self.flow.clone(), self.draft.clone());
        // The review replaces this modal: after this picker's own update is over.
        window.defer(cx, move |window, cx| {
            flow.review(draft, Some(namespace), window, cx);
        });
    }

    /// Closing without a choice sends nothing.
    fn dismissed(&mut self, _: &mut Window, _: &mut Context<NamespacePicker>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<NamespacePicker>,
    ) -> Option<Self::ListItem> {
        let tokens = cx.tokens();
        if let Some(found) = self.matches.get(ix) {
            let name = self.names.get(found.candidate_id)?;
            return Some(
                match_label(name.clone().into(), &found.positions, selected, cx)
                    .debug_selector(move || format!("copy-namespace-row-{ix}")),
            );
        }
        let typed = self.typed.as_ref().filter(|_| ix == self.matches.len())?;
        Some(
            h_flex()
                .debug_selector(|| "copy-namespace-typed".into())
                .gap(u(tokens.spacing.md))
                .child(div().child(format!("Use {typed}")))
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text_muted)
                        .child("not in the list"),
                ),
        )
    }

    fn render_header(
        &self,
        _: &mut Window,
        cx: &mut Context<NamespacePicker>,
    ) -> Option<gpui::AnyElement> {
        let tokens = cx.tokens();
        Some(
            h_flex()
                .debug_selector(|| "copy-namespace-picker".into())
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Copy {}", self.draft.source.name)),
                )
                .child(
                    div()
                        .text_color(tokens.colors.text_muted)
                        .child(format!("to {}", self.cluster_name())),
                )
                .into_any_element(),
        )
    }
}
