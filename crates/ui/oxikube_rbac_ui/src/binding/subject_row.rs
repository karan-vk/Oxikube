//! One subject's card in the binding dialog: the kind select, the name with autocomplete, a
//! ServiceAccount's namespace, and the identity the subject authenticates as.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::dialog::BindingDialog;
use super::draft::{Field, SubjectKind};
use crate::role::Level;

/// How many suggestions are listed under a subject.
const SUGGESTIONS: usize = 6;

/// A suggestion: what to show and what a click fills in.
struct Suggestion {
    label: String,
    name: String,
    namespace: Option<String>,
}

impl BindingDialog {
    /// The existing names the typed text could be: users or groups bindings already use, or the
    /// ServiceAccounts (of the typed namespace, when there is one).
    fn suggestions_for(&self, ix: usize, cx: &gpui::App) -> Vec<Suggestion> {
        let Some(subject) = self.subjects.get(ix) else {
            return Vec::new();
        };
        let draft = subject.draft(cx);
        let query = draft.name.trim().to_lowercase();
        let matches = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        match draft.kind {
            SubjectKind::User | SubjectKind::Group => {
                let names = if draft.kind == SubjectKind::User {
                    &self.choices.users
                } else {
                    &self.choices.groups
                };
                names
                    .iter()
                    .filter(|name| matches(name) && draft.name.trim() != &***name)
                    .take(SUGGESTIONS)
                    .map(|name| Suggestion {
                        label: name.to_string(),
                        name: name.to_string(),
                        namespace: None,
                    })
                    .collect()
            }
            SubjectKind::ServiceAccount => {
                let namespace = draft.namespace.trim();
                self.choices
                    .service_accounts
                    .iter()
                    .filter(|(own, name)| {
                        (namespace.is_empty() || **own == *namespace)
                            && matches(name)
                            && !(draft.name.trim() == &**name && namespace == &**own)
                    })
                    .take(SUGGESTIONS)
                    .map(|(own, name)| Suggestion {
                        label: format!("{own}/{name}"),
                        name: name.to_string(),
                        namespace: Some(own.to_string()),
                    })
                    .collect()
            }
        }
    }

    pub(super) fn subject_card(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let Some(editor) = self.subjects.get(ix) else {
            return div().into_any_element();
        };
        let draft = editor.draft(cx);
        let kind = draft.kind;
        let identity = draft.identity();
        let suggestions = if self.picking == Some(ix) {
            self.suggestions_for(ix, cx)
        } else {
            Vec::new()
        };
        let line = |field: Field, id: String, cx: &mut Context<Self>| {
            let colors = cx.colors();
            self.problem_at(field, cx).map(|problem| {
                div()
                    .debug_selector(move || id)
                    .text_size(u(px(12.)))
                    .text_color(match problem.level {
                        Level::Error => colors.error,
                        Level::Warning => colors.warning,
                    })
                    .child(problem.message)
            })
        };
        let name_problem = line(
            Field::SubjectName(ix),
            format!("binding-subject-{ix}-name-problem"),
            cx,
        );
        let namespace_problem = line(
            Field::SubjectNamespace(ix),
            format!("binding-subject-{ix}-namespace-problem"),
            cx,
        );
        v_flex()
            .id(("binding-subject", ix))
            .debug_selector(move || format!("binding-subject-{ix}"))
            .gap(u(px(6.)))
            .p(u(px(10.)))
            .rounded(u(px(6.)))
            .border_1()
            .border_color(colors.border)
            .child(
                h_flex()
                    .gap(u(px(4.)))
                    .items_center()
                    .children(SubjectKind::ALL.map(|own| {
                        div()
                            .debug_selector(move || {
                                format!("binding-subject-{ix}-kind-{}", own.as_str())
                            })
                            .child(
                                Button::new(("binding-subject-kind", ix * 4 + own as usize))
                                    .xsmall()
                                    .label(own.as_str())
                                    .selected(own == kind)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_subject_kind(ix, own, cx)
                                    })),
                            )
                    }))
                    .child(div().flex_1())
                    .child(
                        div()
                            .debug_selector(move || format!("binding-subject-{ix}-remove"))
                            .child(
                                Button::new(("binding-subject-remove", ix))
                                    .xsmall()
                                    .ghost()
                                    .icon(Icon::new(IconName::X))
                                    .tooltip("Remove this subject")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_subject(ix, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap(u(px(8.)))
                    .when(kind == SubjectKind::ServiceAccount, |row| {
                        row.child(
                            div()
                                .debug_selector(move || format!("binding-subject-{ix}-namespace"))
                                .w(u(px(200.)))
                                .child(Input::new(&editor.namespace)),
                        )
                    })
                    .child(
                        div()
                            .debug_selector(move || format!("binding-subject-{ix}-name"))
                            .flex_1()
                            .child(Input::new(&editor.name)),
                    ),
            )
            .children(namespace_problem)
            .children(name_problem)
            .children(identity.map(|identity| {
                div()
                    .debug_selector(move || format!("binding-subject-{ix}-identity"))
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(format!("Authenticates as {identity}"))
            }))
            .when(!suggestions.is_empty(), |card| {
                card.child(
                    h_flex()
                        .debug_selector(move || format!("binding-subject-{ix}-suggestions"))
                        .flex_wrap()
                        .gap(u(px(4.)))
                        .children(suggestions.into_iter().enumerate().map(|(j, s)| {
                            let id = s.label.clone();
                            div()
                                .debug_selector(move || format!("binding-subject-{ix}-pick-{id}"))
                                .child(
                                    Button::new(("binding-subject-pick", ix * 64 + j))
                                        .xsmall()
                                        .label(s.label)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.pick_subject(
                                                ix,
                                                &s.name,
                                                s.namespace.as_deref(),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                        })),
                )
            })
            .into_any_element()
    }
}
