//! Drawing the page: the toolbar, the info lines, the table and, under a matrix, the detail
//! that names the bindings and roles behind the selected cell.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_app::rbac_subjects::{GROUP_MEMBERS_NOTE, Inherit, MatrixColumn, Provenance};
use oxikube_domain::access::Access;
use oxikube_domain::rbac::{GrantScope, RoleKind, Subject};
use oxikube_keymap::{KeyContextBuilder, contexts};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::table::Table;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::actions::{
    Back, CopyMatrix, FocusFilter, FocusTable, NextColumn, NextScope, OpenSelected, PreviousColumn,
    SelectFirst, SelectLast, SelectNext, SelectPrevious, ToggleImplicitGroups,
};
use super::model::{Page, Phase, scope_label};
use super::view::SubjectsView;

impl SubjectsView {
    fn key_context(&self) -> gpui::KeyContext {
        let mut builder = KeyContextBuilder::new(contexts::RBAC_SUBJECTS);
        builder.flag_if(self.editing, contexts::EDITING);
        builder.build()
    }

    fn counts(&self, cx: &Context<Self>) -> SharedString {
        let (shown, total, noun) = match self.page {
            Page::Subject => self.grid.read(cx, |d| (d.len(), d.total(), "rows")),
            Page::Users => self.list.read(cx, |d| (d.len(), d.total(), "users")),
            Page::Groups => self.list.read(cx, |d| (d.len(), d.total(), "groups")),
        };
        match &self.phase {
            Phase::Loading { pending } if total == 0 => {
                format!("Reading roles and bindings ({pending} of 4 lists pending)...").into()
            }
            _ if shown == total => format!("{total} {noun}").into(),
            _ => format!("{shown} of {total} {noun}").into(),
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let counts = self.counts(cx);
        let matrix_page = self.page == Page::Subject;
        let mut bar = h_flex()
            .id("rbac-toolbar")
            .debug_selector(|| "rbac-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(colors.border_variant);
        if matrix_page {
            bar = bar.child(
                div().debug_selector(|| "rbac-back".into()).child(
                    Button::new("rbac-back-button")
                        .label("Back")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                ),
            );
        }
        bar = bar
            .child(
                div()
                    .debug_selector(|| "rbac-title".into())
                    .text_size(u(tokens.font.body))
                    .child(self.page_title()),
            )
            .child(
                div()
                    .debug_selector(|| "rbac-filter".into())
                    .flex_none()
                    .w(u(px(220.)))
                    .child(Input::new(&self.filter_input).xsmall()),
            )
            .child(
                div()
                    .debug_selector(|| "rbac-count".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(counts),
            );
        if let Some(problem) = self.filter.problem() {
            bar = bar.child(
                div()
                    .debug_selector(|| "rbac-filter-problem".into())
                    .text_size(u(tokens.font.small))
                    .text_color(colors.warning)
                    .child(SharedString::from(problem.to_owned())),
            );
        }
        bar = bar.child(div().flex_1());
        if matrix_page {
            bar = bar.children(self.scopes.iter().enumerate().map(|(ix, scope)| {
                let target = scope.clone();
                div()
                    .debug_selector(move || format!("rbac-scope-{ix}"))
                    .child(
                        Button::new(("rbac-scope", ix))
                            .label(scope_label(scope))
                            .ghost()
                            .xsmall()
                            .selected(*scope == self.scope)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_scope(target.clone(), cx);
                            })),
                    )
            }));
            bar = bar
                .child(
                    div().debug_selector(|| "rbac-implicit".into()).child(
                        Button::new("rbac-implicit-button")
                            .label("Implicit groups")
                            .ghost()
                            .xsmall()
                            .selected(self.inherit == Inherit::ImplicitGroups)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.toggle_implicit_groups(cx)),
                            ),
                    ),
                )
                .child(
                    div().debug_selector(|| "rbac-copy".into()).child(
                        Button::new("rbac-copy-button")
                            .label("Copy as text")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| this.request_copy(cx))),
                    ),
                );
        } else {
            bar = bar.child(
                div().debug_selector(|| "rbac-open".into()).child(
                    Button::new("rbac-open-button")
                        .label("Show matrix")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.open_selected(cx))),
                ),
            );
        }
        bar.into_any_element()
    }

    fn page_title(&self) -> SharedString {
        match self.page {
            Page::Users => "Users and ServiceAccounts".into(),
            Page::Groups => "Groups".into(),
            Page::Subject => match &self.subject {
                Some(subject) => subject_title(subject),
                None => self.asked.clone().into(),
            },
        }
    }

    /// The lines under the toolbar: what the page cannot know, what failed to load.
    fn notes(&self, cx: &Context<Self>) -> Vec<(&'static str, SharedString, bool)> {
        let mut notes = Vec::new();
        let is_group = match self.page {
            Page::Groups => true,
            Page::Subject => matches!(self.subject, Some(Subject::Group(_))),
            Page::Users => false,
        };
        if is_group {
            notes.push(("rbac-group-note", GROUP_MEMBERS_NOTE.into(), false));
        }
        if self.page == Page::Subject
            && let Some(matrix) = &self.matrix
            && !matrix.is_empty()
        {
            let scope = scope_label(&self.scope);
            notes.push((
                "rbac-matrix-note",
                format!(
                    "{scope}: {} binding{} grant{} this. \u{2713} every object, ~ only named objects.",
                    matrix.grants,
                    if matrix.grants == 1 { "" } else { "s" },
                    if matrix.grants == 1 { "s" } else { "" },
                )
                .into(),
                false,
            ));
            for dangling in &matrix.dangling {
                notes.push((
                    "rbac-dangling",
                    format!("{dangling} does not exist: the binding grants nothing.").into(),
                    true,
                ));
            }
        }
        if let Phase::Unavailable(why) = &self.phase {
            notes.push(("rbac-unavailable", why.clone().into(), true));
        }
        for problem in &self.status.problems {
            notes.push(("rbac-problem", problem.clone().into(), true));
        }
        let _ = cx;
        notes
    }

    /// The detail under a matrix: who grants the selected cell.
    fn why_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.page != Page::Subject {
            return None;
        }
        let (row, line) = self.grid.read(cx, |d| {
            let row = d.selected_row()?;
            Some((row, d.line(row)?.clone()))
        })?;
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let chips = MatrixColumn::all().enumerate().map(|(ix, column)| {
            div()
                .debug_selector(move || format!("rbac-column-{ix}"))
                .child(
                    Button::new(("rbac-column", ix))
                        .label(column.header())
                        .ghost()
                        .xsmall()
                        .selected(column == self.column)
                        .on_click(cx.listener(move |this, _, _, cx| this.set_column(column, cx))),
                )
        });
        let heading = format!(
            "{} {} \u{2014} {}",
            line.group_label(),
            line.resource,
            self.column.header()
        );
        let body: Vec<AnyElement> = if self.why.is_empty() {
            vec![
                div()
                    .debug_selector(|| "rbac-why-none".into())
                    .text_color(colors.text_muted)
                    .child("No binding grants this.")
                    .into_any_element(),
            ]
        } else {
            self.why
                .iter()
                .enumerate()
                .map(|(ix, why)| {
                    div()
                        .debug_selector(move || format!("rbac-why-{ix}"))
                        .child(SharedString::from(why_text(why)))
                        .into_any_element()
                })
                .collect()
        };
        let _ = row;
        Some(
            v_flex()
                .id("rbac-why")
                .debug_selector(|| "rbac-why".into())
                .size_full()
                .gap(u(tokens.spacing.xs))
                .p(u(tokens.spacing.md))
                .text_size(u(tokens.font.small))
                .child(
                    h_flex()
                        .gap(u(tokens.spacing.sm))
                        .items_center()
                        .child(div().text_color(colors.text_muted).child("Granted by"))
                        .child(SharedString::from(heading))
                        .child(div().flex_1())
                        .children(chips),
                )
                .children(body)
                .into_any_element(),
        )
    }
}

fn subject_title(subject: &Subject) -> SharedString {
    match subject {
        Subject::ServiceAccount { namespace, name } => {
            format!("ServiceAccount {namespace}/{name}").into()
        }
        other => other.to_string().into(),
    }
}

/// `RoleBinding web/readers -> ClusterRole view (via Group devs; only: a, b)`.
fn why_text(why: &Provenance) -> String {
    let role = match (why.role.kind, &why.binding.namespace) {
        (RoleKind::Role, Some(ns)) => format!("Role {ns}/{}", why.role.name),
        (kind, _) => format!("{} {}", kind.as_str(), why.role.name),
    };
    let mut text = format!("{} \u{2192} {role}", why.binding);
    if !why.role_found {
        text.push_str(" (missing)");
    }
    let mut notes = Vec::new();
    if matches!(why.via, Subject::Group(_)) {
        notes.push(format!("via {}", why.via));
    }
    if why.access == Access::Restricted && !why.names.is_empty() {
        notes.push(format!("only: {}", why.names.join(", ")));
    }
    if let GrantScope::Namespace(ns) = &why.scope {
        notes.push(format!("in {ns}"));
    }
    if !notes.is_empty() {
        text.push_str(&format!(" ({})", notes.join("; ")));
    }
    text
}

impl Render for SubjectsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let toolbar = self.toolbar(cx);
        let notes = self.notes(cx);
        let why = self.why_pane(cx);
        let matrix_page = self.page == Page::Subject;
        let table = if matrix_page {
            div()
                .id("rbac-matrix")
                .debug_selector(|| "rbac-matrix".into())
                .flex_1()
                .min_h(u(px(120.)))
                .w_full()
                .child(Table::new(&self.grid).bordered(false))
        } else {
            div()
                .id("rbac-table")
                .debug_selector(|| "rbac-table".into())
                .flex_1()
                .min_h(u(px(120.)))
                .w_full()
                .child(Table::new(&self.list).bordered(false))
        };
        v_flex()
            .id("rbac-subjects")
            .debug_selector(|| "rbac-subjects".into())
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select_end(false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select_end(true, cx)))
            .on_action(
                cx.listener(|this, _: &FocusFilter, window, cx| this.focus_filter(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusTable, window, cx| this.focus_rows(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
            .on_action(cx.listener(|this, _: &NextColumn, _, cx| this.step_column(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousColumn, _, cx| this.step_column(false, cx)))
            .on_action(cx.listener(|this, _: &NextScope, _, cx| this.next_scope(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleImplicitGroups, _, cx| {
                    this.toggle_implicit_groups(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CopyMatrix, _, cx| this.request_copy(cx)))
            .child(toolbar)
            .children(notes.into_iter().map(|(selector, text, warn)| {
                div()
                    .id(selector)
                    .debug_selector(move || selector.to_owned())
                    .px(u(tokens.spacing.md))
                    .py(u(tokens.spacing.xs))
                    .text_size(u(tokens.font.small))
                    .text_color(if warn {
                        colors.warning
                    } else {
                        colors.text_muted
                    })
                    .child(text)
            }))
            .child(table)
            .children(why.map(|pane| {
                div()
                    .id("rbac-why-slot")
                    .flex_none()
                    .h(u(px(150.)))
                    .w_full()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(pane)
            }))
    }
}
