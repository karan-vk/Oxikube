//! [`ClusterPicker`]: the first step, which cluster to copy to: the connected ones first, then the
//! catalog's contexts (connected when chosen). The generic [`Picker`] (E11-S02) over a
//! [`ClusterPickerDelegate`], so typing filters the clusters.

use std::sync::Arc;

use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Task, Window, div, px,
};
use oxikube_app::ClusterChoice;
use oxikube_palette::picker::fuzzy::{self, StringMatch, StringMatchCandidate};
use oxikube_palette::{Picker, PickerDelegate, match_label};
use oxikube_runtime::spawn_kube;
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::flow::{CopyDraft, CopyFlow};

/// The cluster chooser: a [`Picker`] whose matches are the clusters a copy can go to.
pub type ClusterPicker = Picker<ClusterPickerDelegate>;

/// The picker's width (before UI zoom).
const WIDTH: f32 = 480.;

/// Asks which cluster to copy to. Typing filters by name; Enter or a click chooses; Escape sends
/// nothing. A cluster known to be read-only is refused with a toast and the picker stays open.
pub struct ClusterPickerDelegate {
    flow: CopyFlow,
    draft: CopyDraft,
    choices: Vec<ClusterChoice>,
    candidates: Arc<[StringMatchCandidate]>,
    matches: Vec<StringMatch>,
    selected: usize,
    /// Whether the list of clusters was asked for.
    started: bool,
    /// Whether it arrived.
    loaded: bool,
    /// The read of the catalog; owned here, so it ends with the picker.
    load: Option<Task<()>>,
}

impl ClusterPickerDelegate {
    pub(super) fn picker(
        flow: CopyFlow,
        draft: CopyDraft,
        window: &mut Window,
        cx: &mut Context<ClusterPicker>,
    ) -> ClusterPicker {
        let delegate = Self {
            flow,
            draft,
            choices: Vec::new(),
            candidates: Arc::from(Vec::new()),
            matches: Vec::new(),
            selected: 0,
            started: false,
            loaded: false,
            load: None,
        };
        Picker::uniform_list(delegate, window, cx).width(px(WIDTH))
    }

    /// The clusters offered, once read, in order.
    pub fn choices(&self) -> &[ClusterChoice] {
        &self.choices
    }

    /// Whether the clusters have been read.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The selected cluster, if any matches the query.
    pub fn selected(&self) -> Option<&ClusterChoice> {
        self.matches
            .get(self.selected)
            .and_then(|found| self.choices.get(found.candidate_id))
    }

    fn arrived(&mut self, choices: Vec<ClusterChoice>) {
        self.candidates = choices
            .iter()
            .enumerate()
            .map(|(ix, choice)| StringMatchCandidate::new(ix, choice.name.clone()))
            .collect();
        self.choices = choices;
        self.loaded = true;
    }

    fn start(&mut self, cx: &mut Context<ClusterPicker>, window: &mut Window) {
        self.started = true;
        let service = self.flow.service().clone();
        let read = spawn_kube(cx, async move { service.clusters().await });
        self.load = Some(cx.spawn_in(window, async move |picker, cx| {
            let choices = read.await.unwrap_or_default();
            picker
                .update_in(cx, |picker, window, cx| {
                    picker.delegate.arrived(choices);
                    picker.refresh(window, cx);
                })
                .ok();
        }));
    }
}

impl PickerDelegate for ClusterPickerDelegate {
    type ListItem = gpui::Div;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<ClusterPicker>) {
        self.selected = ix;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Copy to which cluster?".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some(if self.loaded {
            "No such cluster".into()
        } else {
            "Reading the clusters…".into()
        })
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<ClusterPicker>,
    ) -> Task<()> {
        if !self.started {
            self.start(cx, window);
        }
        self.matches = fuzzy::match_strings(&self.candidates, &query, usize::MAX);
        self.selected = 0;
        Task::ready(())
    }

    /// Chooses the selected cluster (both confirms do the same): the namespace step follows.
    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<ClusterPicker>) {
        let Some(choice) = self.selected().cloned() else {
            return;
        };
        if self.flow.refuse_read_only(&choice, cx) {
            return;
        }
        let (flow, draft) = (self.flow.clone(), self.draft.clone());
        // The next step replaces this modal: after this picker's own update is over.
        window.defer(cx, move |window, cx| {
            flow.cluster_chosen(draft, choice, window, cx);
        });
    }

    /// Closing without a choice sends nothing.
    fn dismissed(&mut self, _: &mut Window, _: &mut Context<ClusterPicker>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<ClusterPicker>,
    ) -> Option<Self::ListItem> {
        let found = self.matches.get(ix)?;
        let choice = self.choices.get(found.candidate_id)?;
        let tokens = cx.tokens();
        let note: &'static str = if choice.read_only {
            "read-only"
        } else if choice.connected {
            "connected"
        } else {
            ""
        };
        let id = found.candidate_id;
        let colors = tokens.colors;
        Some(
            h_flex()
                .debug_selector(move || format!("copy-cluster-row-{id}"))
                .gap(u(tokens.spacing.md))
                .child(
                    match_label(choice.name.clone().into(), &found.positions, selected, cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(if choice.read_only {
                            colors.warning
                        } else {
                            colors.text_muted
                        })
                        .child(note),
                ),
        )
    }

    fn render_header(
        &self,
        _: &mut Window,
        cx: &mut Context<ClusterPicker>,
    ) -> Option<gpui::AnyElement> {
        let tokens = cx.tokens();
        Some(
            h_flex()
                .debug_selector(|| "copy-cluster-picker".into())
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
                        .child("Pick the target cluster."),
                )
                .into_any_element(),
        )
    }
}
