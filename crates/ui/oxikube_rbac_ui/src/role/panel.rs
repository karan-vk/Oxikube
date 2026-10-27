//! [`RolePanel`]: a Role's or ClusterRole's rules in the detail, with the aggregation it comes
//! from and the buttons that edit or delete it.

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
    uniform_list,
};
use oxikube_app::role_ops::RoleLinks;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::rbac::{Role, RoleId, RoleKind, RoleSelector};
use oxikube_resources_ui::detail::{DetailView, PanelHost};
use oxikube_runtime::spawn_kube;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Sizable as _, Tokens, u};

use super::rows::RuleRow;

/// The height of a row, at 100 % zoom: uniform, as the virtualised list needs.
const ROW_HEIGHT: f32 = 26.;

/// How many rows the table shows before it scrolls.
const VISIBLE_ROWS: usize = 10;

/// The five columns: title, and the share of the width each takes.
const COLUMNS: [(&str, f32); 5] = [
    ("API groups", 1.),
    ("Resources", 1.6),
    ("Verbs", 1.6),
    ("Resource names", 1.),
    ("Non-resource URLs", 1.),
];

/// What the panel knows of the ClusterRoles an aggregated role collects from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sources {
    /// Not an aggregated role.
    None,
    /// Being read.
    Loading,
    /// The ClusterRoles, sorted.
    Ready(Vec<RoleId>),
    /// The read failed (no right to list ClusterRoles, offline).
    Failed(String),
}

/// What the rows were made from: another version of the object makes them again.
#[derive(Clone, PartialEq, Eq)]
struct Source {
    uid: Option<Arc<str>>,
    version: Option<Arc<str>>,
}

/// The Role panel. See the module docs.
pub struct RolePanel {
    detail: WeakEntity<DetailView>,
    role: Option<Role>,
    rows: Arc<[RuleRow]>,
    sources: Sources,
    built_from: Option<Source>,
    /// The aggregation sources being read; a newer read replaces (and so cancels) it.
    reading: Option<Task<()>>,
    /// The delete button is counting bindings.
    counting: bool,
    /// The count being made; replaced, never cleared from inside.
    count_task: Option<Task<()>>,
    _detail: Option<Subscription>,
}

impl RolePanel {
    /// A panel over `detail`, following it.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let subscription = detail
            .upgrade()
            .map(|entity| cx.observe(&entity, |this, _, cx| this.refresh(cx)));
        // The detail is being drawn (it built this panel), so the object is read after the draw.
        let panel = cx.entity().downgrade();
        cx.defer(move |cx| {
            panel.update(cx, |this, cx| this.refresh(cx)).ok();
        });
        Self {
            detail,
            role: None,
            rows: Arc::from(Vec::new()),
            sources: Sources::None,
            built_from: None,
            reading: None,
            counting: false,
            count_task: None,
            _detail: subscription,
        }
    }

    /// The role, once read.
    pub fn role(&self) -> Option<&Role> {
        self.role.as_ref()
    }

    /// The rows of the table.
    pub fn rows(&self) -> &[RuleRow] {
        &self.rows
    }

    /// The aggregation sources.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// Whether the delete button is counting the role's bindings.
    pub fn is_counting(&self) -> bool {
        self.counting
    }

    /// Builds the rows when the object has a new version.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (role, source, host) = {
            let view = detail.read(cx);
            let Some(resource) = view.resource() else {
                return;
            };
            let source = Source {
                uid: resource.meta.uid.clone(),
                version: resource.meta.resource_version.clone(),
            };
            if self.built_from.as_ref() == Some(&source) {
                return;
            }
            let Ok(role) = Role::from_resource(resource) else {
                return;
            };
            (role, source, view.host())
        };
        self.rows = RuleRow::list(&role.rules).into();
        self.built_from = Some(source);
        self.read_sources(&role, &host, cx);
        self.role = Some(role);
        cx.notify();
    }

    /// Reads the ClusterRoles an aggregated role collects from, off the UI thread.
    fn read_sources(&mut self, role: &Role, host: &PanelHost, cx: &mut Context<Self>) {
        if role.aggregation.is_none() {
            self.sources = Sources::None;
            self.reading = None;
            return;
        }
        self.sources = Sources::Loading;
        let links = RoleLinks::new(host.sessions().clone());
        let cluster = host.target().cluster.clone();
        let role = role.clone();
        let read = spawn_kube(cx, async move { links.contributors(&cluster, &role).await });
        self.reading = Some(cx.spawn(async move |this, cx| {
            let sources = match read.await {
                Ok(Ok(found)) => Sources::Ready(found),
                Ok(Err(error)) => Sources::Failed(error.message().to_owned()),
                Err(error) => Sources::Failed(error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.sources = sources;
                cx.notify();
            })
            .ok();
        }));
    }

    /// "Edit rules…": the dialog on this role.
    fn edit(&self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let target = host.target().clone();
        host.dispatch(
            Command::RoleOpenDialog {
                cluster: Some(target.cluster.clone()),
                cluster_role: false,
                target: Some(target),
            },
            cx,
        );
    }

    /// "Delete": counts the bindings that name the role, then sends `role::Delete` with the count
    /// (the guard's confirmation warns about them). The count is a read off the UI thread; if it
    /// fails the delete goes ahead without one.
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host(cx) else {
            return;
        };
        let Some(role) = self.role.as_ref().map(|role| role.id.clone()) else {
            return;
        };
        if self.counting {
            return;
        }
        self.counting = true;
        let links = RoleLinks::new(host.sessions().clone());
        let target = host.target().clone();
        let cluster = target.cluster.clone();
        let count = spawn_kube(
            cx,
            async move { links.binding_count(&cluster, &role).await },
        );
        self.count_task = Some(cx.spawn(async move |this, cx| {
            let bindings = count.await.ok().and_then(Result::ok);
            this.update(cx, |this, cx| {
                this.counting = false;
                if let Some(host) = this.host(cx) {
                    host.dispatch(Command::RoleDelete { target, bindings }, cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn host(&self, cx: &Context<Self>) -> Option<PanelHost> {
        self.detail.upgrade().map(|detail| detail.read(cx).host())
    }

    /// How the button of `command` stands: hidden (no RBAC grant for what it needs), enabled, or
    /// disabled with the reason.
    fn block(&self, command: CommandId, cx: &Context<Self>) -> Option<Option<String>> {
        let detail = self.detail.upgrade()?;
        detail.read(cx).command_block(command)
    }

    fn header(&self, role: &Role, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let count = role.rules.len();
        let edit = self.block(CommandId::ROLE_CREATE, cx);
        let delete = self.block(CommandId::ROLE_DELETE, cx);
        let counting = self.counting;
        h_flex()
            .px(u(tokens.spacing.lg))
            .pt(u(tokens.spacing.lg))
            .pb(u(tokens.spacing.sm))
            .gap(u(tokens.spacing.md))
            .items_center()
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .child(
                div()
                    .debug_selector(|| "role-kind".to_owned())
                    .px(u(tokens.spacing.md))
                    .rounded(u(tokens.radius.sm))
                    .bg(colors.element)
                    .text_color(colors.text)
                    .child(role.id.kind.as_str()),
            )
            .child(
                div()
                    .debug_selector(|| "role-rule-count".to_owned())
                    .child(match count {
                        1 => "1 rule".to_owned(),
                        n => format!("{n} rules"),
                    }),
            )
            .when(role.aggregation.is_some(), |row| {
                row.child(
                    div()
                        .debug_selector(|| "role-aggregated".to_owned())
                        .px(u(tokens.spacing.md))
                        .rounded(u(tokens.radius.sm))
                        .bg(colors.element)
                        .text_color(colors.text)
                        .child("Aggregated"),
                )
            })
            .child(div().flex_1())
            .when_some(edit, |row, state| {
                let label = if role.aggregation.is_some() {
                    "View rules…"
                } else {
                    "Edit rules…"
                };
                row.child(
                    div().debug_selector(|| "role-edit".to_owned()).child(
                        Button::new("role-edit")
                            .xsmall()
                            .ghost()
                            .label(label)
                            .disabled(state.is_some())
                            .tooltip(state.unwrap_or_else(|| {
                                "Edit the rules with verb checkboxes and a resource picker".into()
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.edit(cx))),
                    ),
                )
            })
            .when_some(delete, |row, state| {
                row.child(
                    div().debug_selector(|| "role-delete".to_owned()).child(
                        Button::new("role-delete")
                            .xsmall()
                            .ghost()
                            .label("Delete")
                            .disabled(state.is_some() || counting)
                            .loading(counting)
                            .tooltip(state.unwrap_or_else(|| {
                                "Delete the role; the confirmation counts the bindings that name it"
                                    .into()
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    /// The aggregation block: the selectors, and the ClusterRoles they pick.
    fn aggregation(&self, role: &Role, tokens: &Tokens) -> Option<AnyElement> {
        let aggregation = role.aggregation.as_ref()?;
        let colors = tokens.colors;
        let selectors: Vec<String> = aggregation.selectors.iter().map(selector_text).collect();
        let from = match &self.sources {
            Sources::None => String::new(),
            Sources::Loading => "Reading the ClusterRoles it collects from…".to_owned(),
            Sources::Failed(why) => format!("Could not list the ClusterRoles: {why}"),
            Sources::Ready(found) if found.is_empty() => {
                "No ClusterRole matches the selectors".to_owned()
            }
            Sources::Ready(found) => {
                let names: Vec<&str> = found.iter().map(|id| &*id.name).collect();
                format!("Aggregated from {}: {}", found.len(), names.join(", "))
            }
        };
        Some(
            v_flex()
                .debug_selector(|| "role-aggregation".to_owned())
                .px(u(tokens.spacing.lg))
                .pb(u(tokens.spacing.sm))
                .gap(u(tokens.spacing.xs))
                .text_size(u(tokens.font.small))
                .text_color(colors.text_muted)
                .child(div().child(format!(
                    "The control plane fills the rules from ClusterRoles matching {}",
                    selectors.join(" or ")
                )))
                .child(
                    div()
                        .debug_selector(|| "role-sources".to_owned())
                        .child(from),
                )
                .into_any_element(),
        )
    }

    fn column_header(&self, tokens: &Tokens) -> AnyElement {
        let colors = tokens.colors;
        h_flex()
            .debug_selector(|| "role-table-header".to_owned())
            .h(u(px(ROW_HEIGHT)))
            .px(u(tokens.spacing.lg))
            .gap(u(tokens.spacing.md))
            .items_center()
            .border_b_1()
            .border_color(colors.border)
            .text_size(u(tokens.font.small))
            .text_color(colors.text_muted)
            .children(
                COLUMNS
                    .iter()
                    .map(|(title, share)| cell(*share, (*title).into())),
            )
            .into_any_element()
    }

    fn row(&self, ix: usize, tokens: &Tokens) -> Option<AnyElement> {
        let row = self.rows.get(ix)?;
        let colors = tokens.colors;
        let raw = row.raw.clone();
        let cells = [
            (COLUMNS[0].1, row.api_groups.clone()),
            (COLUMNS[1].1, row.resources.clone()),
            (COLUMNS[2].1, row.verbs.clone()),
            (COLUMNS[3].1, row.resource_names.clone()),
            (COLUMNS[4].1, row.non_resource_urls.clone()),
        ];
        Some(
            h_flex()
                .id(("role-rule", ix))
                .debug_selector(move || format!("role-rule-{ix}"))
                .h(u(px(ROW_HEIGHT)))
                .px(u(tokens.spacing.lg))
                .gap(u(tokens.spacing.md))
                .items_center()
                .text_size(u(tokens.font.small))
                .text_color(colors.text)
                .hover(|row| row.bg(colors.element))
                .tooltip(move |window, cx| Tooltip::new(raw.to_string()).build(window, cx))
                .children(cells.map(|(share, text)| cell(share, text.to_string())))
                .into_any_element(),
        )
    }
}

/// One cell: `share` of the row's width, one line, cut with an ellipsis.
fn cell(share: f32, text: String) -> AnyElement {
    div()
        .w(gpui::relative(share / 6.2))
        .min_w_0()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .child(text)
        .into_any_element()
}

/// A selector as the aggregation line says it: `key=value` terms and `key In (a,b)` expressions.
fn selector_text(selector: &RoleSelector) -> String {
    let mut terms: Vec<String> = selector
        .match_labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    for requirement in &selector.match_expressions {
        let values: Vec<&str> = requirement.values.iter().map(|value| &**value).collect();
        terms.push(format!(
            "{} {} ({})",
            requirement.key,
            requirement.operator,
            values.join(",")
        ));
    }
    terms.join(", ")
}

impl Render for RolePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let Some(role) = self.role.clone() else {
            return v_flex()
                .id("role-panel")
                .debug_selector(|| "role-panel".to_owned())
                .p(u(tokens.spacing.lg))
                .text_color(tokens.colors.text_muted)
                .text_size(u(tokens.font.small))
                .child("Reading the role…")
                .into_any_element();
        };
        let total = self.rows.len();
        let kind = role.id.kind;
        v_flex()
            .id("role-panel")
            .debug_selector(|| "role-panel".to_owned())
            .child(self.header(&role, cx))
            .children(self.aggregation(&role, &tokens))
            .child(if total == 0 {
                div()
                    .debug_selector(|| "role-empty".to_owned())
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.md))
                    .text_color(tokens.colors.text_muted)
                    .child(match kind {
                        RoleKind::Role => "This Role grants nothing: it has no rules",
                        RoleKind::ClusterRole => "This ClusterRole grants nothing: it has no rules",
                    })
                    .into_any_element()
            } else {
                v_flex()
                    .child(self.column_header(&tokens))
                    .child(
                        div()
                            .debug_selector(|| "role-rules".to_owned())
                            .h(u(px(ROW_HEIGHT * total.min(VISIBLE_ROWS) as f32)))
                            .w_full()
                            .child(
                                uniform_list(
                                    "role-rules-list",
                                    total,
                                    cx.processor(move |this, range: Range<usize>, _, cx| {
                                        let tokens = cx.tokens();
                                        range.filter_map(|ix| this.row(ix, &tokens)).collect()
                                    }),
                                )
                                .size_full(),
                            ),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }
}
