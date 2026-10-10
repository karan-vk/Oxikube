//! [`ContainerPicker`]: the modal that asks which container of a pod to open a session in, a
//! generic [`Picker`] (E11-S02) over a [`ContainerPickerDelegate`].

use std::sync::Arc;

use gpui::{
    App, Context, DismissEvent, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Task, Window, div, px,
};
use oxikube_app::ContainerChoices;
use oxikube_domain::ids::ResourceRef;
use oxikube_palette::picker::fuzzy::{self, StringMatch, StringMatchCandidate};
use oxikube_palette::{Picker, PickerDelegate};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::flow::{ExecFlow, ExecKind};

/// The container chooser: a [`Picker`] whose matches are a pod's containers.
pub type ContainerPicker = Picker<ContainerPickerDelegate>;

/// The picker's width (before UI zoom): container names are short.
const WIDTH: f32 = 420.;

/// Asks which container to open a shell (or an attach) in. Typing filters the containers by name;
/// the picker's keys move (up / down, wrapping), Enter or a click opens, Escape cancels (nothing
/// is sent). The default container (or the one opened last in this pod) starts selected, so Enter
/// alone does what the command would have done.
pub struct ContainerPickerDelegate {
    kind: ExecKind,
    target: ResourceRef,
    choices: ContainerChoices,
    flow: ExecFlow,
    /// The container names, matched against the query (a pod has a handful: matched inline).
    candidates: Arc<[StringMatchCandidate]>,
    matches: Vec<StringMatch>,
    selected: usize,
}

impl ContainerPickerDelegate {
    /// A delegate for `choices` of `target`, with `choices.preselected` selected.
    pub fn new(
        kind: ExecKind,
        target: ResourceRef,
        choices: ContainerChoices,
        flow: ExecFlow,
    ) -> Self {
        let candidates: Arc<[StringMatchCandidate]> = choices
            .containers
            .iter()
            .enumerate()
            .map(|(ix, container)| StringMatchCandidate::new(ix, container.name.to_string()))
            .collect();
        Self {
            kind,
            target,
            choices,
            flow,
            candidates,
            matches: Vec::new(),
            selected: 0,
        }
    }

    /// The picker over this delegate, sized for container names.
    pub fn picker(
        kind: ExecKind,
        target: ResourceRef,
        choices: ContainerChoices,
        flow: ExecFlow,
        window: &mut Window,
        cx: &mut Context<ContainerPicker>,
    ) -> ContainerPicker {
        Picker::uniform_list(Self::new(kind, target, choices, flow), window, cx).width(px(WIDTH))
    }

    /// The containers offered, in order.
    pub fn choices(&self) -> &ContainerChoices {
        &self.choices
    }

    /// The index (in [`Self::choices`]) of the selected container, if any matches the query.
    pub fn selected(&self) -> Option<usize> {
        self.matches.get(self.selected).map(|m| m.candidate_id)
    }

    fn title(&self) -> SharedString {
        format!("{} {}", self.kind.verb(), self.target.name).into()
    }
}

impl PickerDelegate for ContainerPickerDelegate {
    type ListItem = gpui::Div;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<ContainerPicker>) {
        self.selected = ix;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Which container?".into()
    }

    fn update_matches(
        &mut self,
        query: String,
        _: &mut Window,
        _: &mut Context<ContainerPicker>,
    ) -> Task<()> {
        self.matches = fuzzy::match_strings(&self.candidates, &query, usize::MAX);
        // The blank query lists every container with the preselected one selected; a filter
        // selects its best match.
        self.selected = if query.trim().is_empty() {
            self.choices
                .preselected
                .min(self.matches.len().saturating_sub(1))
        } else {
            0
        };
        Task::ready(())
    }

    /// Opens a session in the selected container (both confirms do the same) and closes.
    fn confirm(&mut self, _secondary: bool, _: &mut Window, cx: &mut Context<ContainerPicker>) {
        let Some(container) = self
            .selected()
            .and_then(|ix| self.choices.containers.get(ix))
        else {
            return;
        };
        let name = container.name.to_string();
        self.flow
            .dispatch(self.kind, self.target.clone(), Some(name), cx);
        cx.emit(DismissEvent);
    }

    /// Closing without a choice sends nothing, so the guard records no open that never was.
    fn dismissed(&mut self, _: &mut Window, _: &mut Context<ContainerPicker>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<ContainerPicker>,
    ) -> Option<Self::ListItem> {
        let found = self.matches.get(ix)?;
        let container = self.choices.containers.get(found.candidate_id)?;
        let colors = cx.colors();
        let highlight = HighlightStyle {
            color: Some(colors.accent),
            font_weight: Some(FontWeight::BOLD),
            ..HighlightStyle::default()
        };
        let id = found.candidate_id;
        // The label starts with the name, so the name's matched positions hold in it.
        Some(
            div()
                .debug_selector(move || format!("container-row-{id}"))
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_muted
                })
                .child(fuzzy::highlighted_text(
                    container.label().into(),
                    &found.positions,
                    highlight,
                )),
        )
    }

    fn render_header(
        &self,
        _: &mut Window,
        cx: &mut Context<ContainerPicker>,
    ) -> Option<gpui::AnyElement> {
        let tokens = cx.tokens();
        Some(
            h_flex()
                .debug_selector(|| "container-picker".into())
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.lg))
                .pt(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(self.title()))
                .child(
                    div()
                        .text_color(tokens.colors.text_muted)
                        .child("The pod has several containers."),
                )
                .into_any_element(),
        )
    }
}
