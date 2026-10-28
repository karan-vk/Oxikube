//! One rule's card in the rule dialog: the inputs, verb checkboxes and the resource suggestions.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, px,
};
use oxikube_domain::rbac::RoleKind;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::dialog::RoleDialog;
use super::draft::{Field, Level, Problem, RuleDraft, RuleField, VERBS};

/// How many suggestions the resource picker lists under a rule.
const SUGGESTIONS: usize = 8;

impl RoleDialog {
    /// `field` under a small muted `label`, with the problem of `at` below.
    pub(super) fn labelled(
        &self,
        label: &'static str,
        id: &'static str,
        at: Field,
        field: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = cx.colors();
        v_flex()
            .gap(u(px(4.)))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(colors.text_muted)
                    .child(label),
            )
            .child(div().debug_selector(move || id.into()).child(field))
            .children(self.problem_at(at, cx).map(|problem| {
                problem_line(
                    &problem,
                    format!("{id}-problem"),
                    (colors.error, colors.warning),
                )
            }))
    }

    fn verb_boxes(
        &self,
        ix: usize,
        rule: &RuleDraft,
        locked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let supported = self
            .options
            .supported_verbs(&rule.api_groups, &rule.resources);
        h_flex()
            .flex_wrap()
            .gap_x(u(px(12.)))
            .gap_y(u(px(4.)))
            .children(VERBS.iter().enumerate().map(|(vi, verb)| {
                let verb: &'static str = verb;
                let checked = rule.has_verb(verb);
                let unsupported = verb != "*"
                    && supported.is_some_and(|set| {
                        !checked
                            && oxikube_domain::kinds::Verb::ALL
                                .iter()
                                .any(|own| own.as_str() == verb && !set.contains(*own))
                    });
                div()
                    .debug_selector(move || format!("role-{ix}-verb-{verb}"))
                    .child(
                        Checkbox::new(("role-verb", ix * 16 + vi))
                            .label(verb)
                            .checked(checked)
                            .disabled(locked || unsupported)
                            .on_click(cx.listener(move |this, on: &bool, _, cx| {
                                this.set_verb(ix, verb, *on, cx)
                            })),
                    )
            }))
            .into_any_element()
    }

    fn suggestions(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.picking != Some(ix) || self.options.is_empty() {
            return None;
        }
        let editor = self.rules.get(ix)?;
        let query = editor.resource_query(cx);
        let found: Vec<(String, String, String)> = self
            .options
            .suggest(&query, SUGGESTIONS)
            .into_iter()
            .map(|option| {
                (
                    option.group.clone(),
                    option.resource.clone(),
                    option.label(),
                )
            })
            .collect();
        if found.is_empty() {
            return None;
        }
        Some(
            h_flex()
                .debug_selector(move || format!("role-{ix}-suggestions"))
                .flex_wrap()
                .gap(u(px(4.)))
                .children(
                    found
                        .into_iter()
                        .enumerate()
                        .map(|(j, (group, resource, label))| {
                            let picked = resource.clone();
                            div()
                                .debug_selector(move || format!("role-{ix}-pick-{picked}"))
                                .child(
                                    Button::new(("role-pick", ix * 64 + j))
                                        .xsmall()
                                        .label(label)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.pick(ix, &group, &resource, window, cx)
                                        })),
                                )
                        }),
                )
                .into_any_element(),
        )
    }

    /// A Role cannot grant cluster-scoped resources; say so for the ones discovery knows.
    fn scope_hint(&self, rule: &RuleDraft) -> Option<Problem> {
        if self.kind != RoleKind::Role {
            return None;
        }
        let cluster_scoped: Vec<&str> = rule
            .resources
            .iter()
            .filter(|resource| {
                let groups: Vec<&str> = if rule.api_groups.is_empty() {
                    self.options.group_of(resource).into_iter().collect()
                } else {
                    rule.api_groups.iter().map(String::as_str).collect()
                };
                groups
                    .iter()
                    .any(|group| self.options.is_namespaced(group, resource) == Some(false))
            })
            .map(String::as_str)
            .collect();
        if cluster_scoped.is_empty() {
            return None;
        }
        Some(Problem {
            at: Field::Rules,
            level: Level::Warning,
            message: format!(
                "{} is cluster-scoped: a Role cannot grant it, use a ClusterRole",
                cluster_scoped.join(", ")
            ),
        })
    }

    pub(super) fn rule_card(&self, ix: usize, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let editor = &self.rules[ix];
        let rule = editor.draft(cx);
        let all = self.problems(cx);
        let problem = |field| all.iter().find(|p| p.at == Field::Rule(ix, field)).cloned();
        let colors_for_lines = (colors.error, colors.warning);
        let line = |problem: Option<Problem>, name: &str| {
            problem.map(|p| problem_line(&p, format!("role-{ix}-{name}-problem"), colors_for_lines))
        };
        let cluster_role = self.kind == RoleKind::ClusterRole;
        v_flex()
            .debug_selector(move || format!("role-rule-card-{ix}"))
            .gap(u(px(8.)))
            .p(u(tokens.spacing.md))
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.md))
            .child(
                h_flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child(format!("Rule {}", ix + 1)),
                    )
                    .when(!locked, |row| {
                        row.child(
                            Button::new(("role-remove", ix))
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::X))
                                .tooltip("Remove this rule")
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove_rule(ix, cx)),
                                ),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap(u(px(12.)))
                    .items_start()
                    .child(
                        self.labelled_rule(
                            "API groups",
                            format!("role-{ix}-groups"),
                            Input::new(&editor.api_groups)
                                .disabled(locked)
                                .into_any_element(),
                            line(problem(RuleField::ApiGroups), "groups"),
                        )
                        .w(u(px(220.))),
                    )
                    .child(
                        self.labelled_rule(
                            "Resources",
                            format!("role-{ix}-resources"),
                            Input::new(&editor.resources)
                                .disabled(locked)
                                .into_any_element(),
                            line(problem(RuleField::Resources), "resources"),
                        )
                        .flex_1()
                        .min_w_0(),
                    ),
            )
            .children(self.suggestions(ix, cx))
            .children(
                self.scope_hint(&rule).map(|hint| {
                    problem_line(&hint, format!("role-{ix}-scope-hint"), colors_for_lines)
                }),
            )
            .child(self.verb_boxes(ix, &rule, locked, cx))
            .children(line(problem(RuleField::Verbs), "verbs"))
            .child(
                h_flex()
                    .gap(u(px(12.)))
                    .items_start()
                    .child(
                        self.labelled_rule(
                            "Other verbs",
                            format!("role-{ix}-other-verbs"),
                            Input::new(&editor.other_verbs)
                                .disabled(locked)
                                .into_any_element(),
                            None,
                        )
                        .w(u(px(180.))),
                    )
                    .child(
                        self.labelled_rule(
                            "Resource names",
                            format!("role-{ix}-names"),
                            Input::new(&editor.resource_names)
                                .disabled(locked)
                                .into_any_element(),
                            line(problem(RuleField::ResourceNames), "names"),
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .when(cluster_role, |row| {
                        row.child(
                            self.labelled_rule(
                                "Non-resource URLs",
                                format!("role-{ix}-urls"),
                                Input::new(&editor.non_resource_urls)
                                    .disabled(locked)
                                    .into_any_element(),
                                line(problem(RuleField::NonResourceUrls), "urls"),
                            )
                            .flex_1()
                            .min_w_0(),
                        )
                    }),
            )
            .into_any_element()
    }

    fn labelled_rule(
        &self,
        label: &'static str,
        id: String,
        field: AnyElement,
        problem: Option<Div>,
    ) -> Div {
        v_flex()
            .gap(u(px(4.)))
            .child(div().text_size(u(px(12.))).child(label))
            .child(div().debug_selector(move || id).child(field))
            .children(problem)
    }
}

/// One line under a field: red for an error, amber for a warning.
pub(super) fn problem_line(problem: &Problem, id: String, (error, warning): (Hsla, Hsla)) -> Div {
    let color = match problem.level {
        Level::Error => error,
        Level::Warning => warning,
    };
    div()
        .debug_selector(move || id)
        .text_size(u(px(12.)))
        .text_color(color)
        .child(problem.message.clone())
}
