//! Drawing the view: the toolbar (type, faults, the four fields, the tail and the exports), the
//! table, and the status line (how many events, the tail, the feed's state).

use gpui::{
    Action, AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_app::events::EventsState;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::table::Table;
use oxikube_ui::tooltip::tooltip_for_action;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::EventsView;
use super::actions::{
    CopyRow, CopyRowJson, ExportJson, FocusFilter, FocusTable, OpenSelected, SelectFirst,
    SelectLast, SelectNext, SelectPrevious, ToggleFaults, TogglePause,
};
use super::filters::Field;
use crate::filter::TypeChoice;

/// `4812` as `4,812`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl EventsView {
    fn key_context(&self, cx: &gpui::App) -> gpui::KeyContext {
        let mut builder = KeyContextBuilder::new(contexts::EVENTS);
        builder
            .flag_if(self.editing, contexts::EDITING)
            .flag_if(self.filters.faults_only, "faults")
            .flag_if(self.model(cx).is_paused(), "paused");
        builder.build()
    }

    /// A toggle of the toolbar, its tooltip naming the key that does the same.
    fn toggle(
        &self,
        id: &'static str,
        label: SharedString,
        action: &dyn Action,
        on: bool,
        cx: &mut Context<Self>,
        command: fn(ClusterId) -> Command,
    ) -> AnyElement {
        let title = label.clone();
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || id.to_owned())
            .tooltip(tooltip_for_action(title, action, Some(contexts::EVENTS)))
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .selected(on)
                    .on_click(cx.listener(move |view, _, _, cx| view.dispatch_events(command, cx))),
            )
            .into_any_element()
    }

    fn field(&self, field: Field, id: &'static str, width: f32) -> AnyElement {
        div()
            .debug_selector(move || id.to_owned())
            .flex_none()
            .w(u(px(width)))
            .child(Input::new(self.fields.get(field)).xsmall())
            .into_any_element()
    }

    fn type_choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.filters.types;
        let faults = self.filters.faults_only;
        h_flex()
            .flex_none()
            .gap(u(px(2.)))
            .children(TypeChoice::ALL.into_iter().map(|choice| {
                let id: &'static str = match choice {
                    TypeChoice::All => "events-type-all",
                    TypeChoice::Normal => "events-type-normal",
                    TypeChoice::Warning => "events-type-warning",
                };
                div().debug_selector(move || id.to_owned()).child(
                    Button::new(id)
                        .label(choice.label())
                        .ghost()
                        .xsmall()
                        .selected(!faults && current == choice)
                        .disabled(faults)
                        .on_click(cx.listener(move |view, _, _, cx| view.set_types(choice, cx))),
                )
            }))
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let paused = self.model(cx).is_paused();
        let unseen = self.model(cx).unseen();
        let pause_label: SharedString = if paused { "Resume" } else { "Pause" }.into();
        h_flex()
            .id("events-toolbar")
            .debug_selector(|| "events-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(self.type_choices(cx))
            .child(self.toggle(
                "events-faults",
                "Faults only".into(),
                &ToggleFaults,
                self.filters.faults_only,
                cx,
                |cluster| Command::EventsToggleFaults { cluster },
            ))
            .child(self.field(Field::Reason, "events-filter-reason", 110.))
            .child(self.field(Field::Kind, "events-filter-kind", 100.))
            .child(self.field(Field::Namespace, "events-filter-namespace", 120.))
            .child(self.field(Field::Text, "events-filter-text", 220.))
            .child(div().flex_1())
            .children((paused && unseen > 0).then(|| {
                div()
                    .id("events-unseen")
                    .debug_selector(|| "events-unseen".into())
                    .flex_none()
                    .px(u(px(6.)))
                    .rounded(u(px(8.)))
                    .bg(tokens.colors.element)
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text)
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_pause(cx)))
                    .child(SharedString::from(format!(
                        "{} new event{}",
                        thousands(unseen),
                        if unseen == 1 { "" } else { "s" }
                    )))
            }))
            .child(self.toggle(
                "events-pause",
                pause_label,
                &TogglePause,
                paused,
                cx,
                |cluster| Command::EventsTogglePause { cluster },
            ))
            .child(self.toggle(
                "events-copy",
                "Copy".into(),
                &CopyRow,
                false,
                cx,
                |cluster| Command::EventsCopyRow { cluster },
            ))
            .child(self.toggle(
                "events-export",
                "Export JSON".into(),
                &ExportJson,
                false,
                cx,
                |cluster| Command::EventsExportJson { cluster },
            ))
            .into_any_element()
    }

    /// The line under the toolbar while the text is not a filter.
    fn error_line(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let message = self.filter_error.clone()?;
        let tokens = cx.tokens();
        Some(
            div()
                .debug_selector(|| "events-filter-error".into())
                .flex_none()
                .px(u(tokens.spacing.md))
                .py(u(px(2.)))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.error)
                .child(SharedString::from(format!("Invalid filter: {message}")))
                .into_any_element(),
        )
    }

    fn status_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let model = self.model(cx);
        let count = model.len();
        let tail = if model.is_paused() { "Paused" } else { "Live" };
        let feed = match &self.feed {
            EventsState::Idle => Some("Not watching (the cluster is not connected)".to_owned()),
            EventsState::Warming => Some("Loading…".to_owned()),
            EventsState::Ready => None,
            EventsState::Retrying { message } => Some(format!("Retrying: {message}")),
            EventsState::Failed { message, .. } => Some(format!("Failed: {message}")),
        };
        let mut text = format!(
            "{} event{} · {tail}",
            thousands(count),
            if count == 1 { "" } else { "s" }
        );
        if let Some(feed) = feed {
            text.push_str(" · ");
            text.push_str(&feed);
        }
        div()
            .debug_selector(|| "events-status".into())
            .flex_none()
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .border_t_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(SharedString::from(text))
            .into_any_element()
    }
}

impl Render for EventsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let toolbar = self.toolbar(cx);
        let error = self.error_line(cx);
        let status = self.status_line(cx);
        v_flex()
            .id("events-view")
            .debug_selector(|| "events-view".into())
            .key_context(self.key_context(cx))
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .on_action(cx.listener(|view, _: &ToggleFaults, _, cx| {
                view.dispatch_events(|cluster| Command::EventsToggleFaults { cluster }, cx);
            }))
            .on_action(cx.listener(|view, _: &TogglePause, _, cx| {
                view.dispatch_events(|cluster| Command::EventsTogglePause { cluster }, cx);
            }))
            .on_action(cx.listener(|view, _: &CopyRow, _, cx| {
                view.dispatch_events(|cluster| Command::EventsCopyRow { cluster }, cx);
            }))
            .on_action(cx.listener(|view, _: &CopyRowJson, _, cx| {
                view.dispatch_events(|cluster| Command::EventsCopyRowJson { cluster }, cx);
            }))
            .on_action(cx.listener(|view, _: &ExportJson, _, cx| {
                view.dispatch_events(|cluster| Command::EventsExportJson { cluster }, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenSelected, _, cx| view.open_selected(cx)))
            .on_action(cx.listener(|view, _: &SelectNext, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &SelectPrevious, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &SelectFirst, _, cx| view.select_end(false, cx)))
            .on_action(cx.listener(|view, _: &SelectLast, _, cx| view.select_end(true, cx)))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                view.focus_filter(window, cx);
            }))
            .on_action(cx.listener(|view, _: &FocusTable, window, cx| {
                view.focus_table(window, cx);
            }))
            .child(toolbar)
            .children(error)
            .child(
                div()
                    .id("events-table")
                    .debug_selector(|| "events-table".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(Table::new(&self.table).bordered(false)),
            )
            .child(status)
    }
}
