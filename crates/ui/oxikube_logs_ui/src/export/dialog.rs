//! [`SaveDialog`]: what a save would write, before the user picks the file.

use std::ops::Range;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window, div,
    px,
};
use oxikube_app::logs::export::{ExportFormat, ExportSpec, LineFilter};
use oxikube_domain::log::LogSaveScope;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use oxikube_workspace::ModalView;
use oxikube_workspace::modal::ModalPlacement;

use crate::view::{LogView, group};

/// What one scope would write: its seqs and how many lines pass the filter (`None` while they are
/// being counted, which only a filter makes slow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveOffer {
    /// The scope.
    pub scope: LogSaveScope,
    /// The seqs it covers, fixed when the dialog opened: the file holds these, however much the
    /// stream grows meanwhile.
    pub seqs: Range<u64>,
    /// Lines that will be written.
    pub lines: Option<u64>,
}

/// What the dialog hands the view when the user goes on to choose a file.
#[derive(Clone)]
pub struct SaveRequest {
    /// Which lines.
    pub scope: LogSaveScope,
    /// What to read and how to write it.
    pub spec: ExportSpec,
    /// Lines the file will hold, when known (for the progress).
    pub lines: Option<u64>,
}

/// The save dialog, a modal of the cluster tab. See the [module docs](super).
pub struct SaveDialog {
    view: WeakEntity<LogView>,
    focus: FocusHandle,
    offers: Vec<SaveOffer>,
    scope: LogSaveScope,
    format: ExportFormat,
    filter: Option<LineFilter>,
    note: Option<String>,
}

impl SaveDialog {
    /// A dialog for `view` offering `offers` (one per scope), starting on `scope` with `format`.
    /// `note` is the truncation note, `filter` the view's filter.
    pub fn new(
        view: WeakEntity<LogView>,
        offers: Vec<SaveOffer>,
        scope: LogSaveScope,
        format: ExportFormat,
        filter: Option<LineFilter>,
        note: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            view,
            focus: cx.focus_handle(),
            offers,
            scope,
            format,
            filter,
            note,
        }
    }

    /// The chosen scope.
    pub fn scope(&self) -> LogSaveScope {
        self.scope
    }

    /// The format toggles as they are now.
    pub fn format(&self) -> ExportFormat {
        self.format
    }

    /// What the dialog offers for `scope`.
    pub fn offer(&self, scope: LogSaveScope) -> Option<&SaveOffer> {
        self.offers.iter().find(|offer| offer.scope == scope)
    }

    /// Lines the chosen scope would write (`None` while counting).
    pub fn lines(&self) -> Option<u64> {
        self.offer(self.scope).and_then(|offer| offer.lines)
    }

    /// Sets the counted lines of `scope` (a filtered count arrives off the UI thread).
    pub fn set_lines(&mut self, scope: LogSaveScope, lines: u64, cx: &mut Context<Self>) {
        if let Some(offer) = self.offers.iter_mut().find(|offer| offer.scope == scope) {
            offer.lines = Some(lines);
            cx.notify();
        }
    }

    /// The truncation note, when the buffer dropped lines.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The words under the title: which lines, how many, filtered or not.
    pub fn summary(&self) -> String {
        let lines = match self.lines() {
            Some(1) => "1 line".to_owned(),
            Some(n) => format!("{} lines", group(n)),
            None => "counting the lines".to_owned(),
        };
        let which = match self.scope {
            LogSaveScope::Visible => "The lines on screen",
            LogSaveScope::All => "Everything the buffer holds",
        };
        let filtered = if self.filter.is_some() {
            ", matching the filter"
        } else {
            ""
        };
        format!("{which}: {lines}{filtered}.")
    }

    /// What goes to the view when the user chooses a file.
    pub fn request(&self) -> Option<SaveRequest> {
        let offer = self.offer(self.scope)?;
        Some(SaveRequest {
            scope: self.scope,
            spec: ExportSpec::new(offer.seqs.clone(), self.format).with_filter(self.filter.clone()),
            lines: offer.lines,
        })
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.request() else {
            return;
        };
        cx.emit(DismissEvent);
        if let Some(view) = self.view.upgrade() {
            window.defer(cx, move |_, cx| {
                view.update(cx, |view, cx| view.save_chosen(request, cx));
            });
        }
    }

    fn scope_button(&self, scope: LogSaveScope, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, label) = match scope {
            LogSaveScope::Visible => ("log-save-scope-visible", "On screen"),
            LogSaveScope::All => ("log-save-scope-all", "Whole buffer"),
        };
        let count = self
            .offer(scope)
            .and_then(|offer| offer.lines)
            .map_or_else(String::new, |n| format!(" ({})", group(n)));
        div().debug_selector(move || id.to_owned()).child(
            Button::new(id)
                .label(format!("{label}{count}"))
                .xsmall()
                .selected(self.scope == scope)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.scope = scope;
                    cx.notify();
                })),
        )
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        flip: fn(&mut ExportFormat),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().debug_selector(move || id.to_owned()).child(
            Button::new(id)
                .label(label)
                .xsmall()
                .selected(on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    flip(&mut this.format);
                    cx.notify();
                })),
        )
    }
}

impl EventEmitter<DismissEvent> for SaveDialog {}

impl Focusable for SaveDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ModalView for SaveDialog {
    fn placement(&self, _: &App) -> ModalPlacement {
        ModalPlacement::Center
    }
}

impl Render for SaveDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let note = self.note.clone().map(|note| {
            div()
                .text_size(u(tokens.font.small))
                .text_color(colors.warning)
                .child(SharedString::from(note))
        });
        v_flex()
            .id("log-save-dialog")
            .debug_selector(|| "log-save-dialog".to_owned())
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(460.)))
            .gap(u(tokens.spacing.xl))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child("Save log to a file"))
                    .child(
                        DialogDescription::new().child(
                            div()
                                .debug_selector(|| "log-save-summary".to_owned())
                                .child(self.summary()),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(self.scope_button(LogSaveScope::Visible, cx))
                    .child(self.scope_button(LogSaveScope::All, cx)),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .child(self.toggle(
                        "log-save-timestamps",
                        "Timestamps",
                        self.format.timestamps,
                        |format| format.timestamps = !format.timestamps,
                        cx,
                    ))
                    .child(self.toggle(
                        "log-save-prefix",
                        "Pod prefix",
                        self.format.pod_prefix,
                        |format| format.pod_prefix = !format.pod_prefix,
                        cx,
                    )),
            )
            .children(note)
            .child(
                DialogFooter::new()
                    .child(
                        div().debug_selector(|| "log-save-cancel".to_owned()).child(
                            Button::new("log-save-cancel-button")
                                .label("Cancel")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "log-save-choose".to_owned()).child(
                            Button::new("log-save-choose-button")
                                .label("Choose file…")
                                .primary()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.choose(window, cx)),
                                ),
                        ),
                    ),
            )
    }
}
