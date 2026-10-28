//! The detail under a matrix: who grants the selected cell.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div,
};
use oxikube_app::rbac_subjects::{MatrixColumn, Provenance};
use oxikube_domain::access::Access;
use oxikube_domain::rbac::{GrantScope, RoleKind, Subject};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::model::Page;
use super::view::SubjectsView;

impl SubjectsView {
    /// The detail under a matrix: who grants the selected cell.
    pub(super) fn why_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
