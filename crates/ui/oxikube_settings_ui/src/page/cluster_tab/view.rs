//! The per-cluster scope on the page: switching scope, the cluster list, the catalog, and the
//! precedence strip under each field.

use futures::StreamExt as _;
use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use oxikube_app::{CatalogEntry, ClusterCatalog};
use oxikube_domain::ids::ClusterId;
use oxikube_runtime::spawn_kube;
use oxikube_settings::{SettingSource, SettingsStore};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::choice::{ClusterChoice, choices};
use super::model::cluster_page_model;
use super::precedence::{
    EDITS_ONLY_THE_CLUSTER, EXPLANATION, Precedence, PrecedenceCell, protection_warning,
};
use super::state::{Parked, Scope};
use crate::page::{PageField, SearchIndex, SettingsPage, SettingsPageModel};

/// The tallest the cluster list grows before it scrolls, unscaled.
const LIST_HEIGHT: f32 = 200.0;

impl SettingsPage {
    /// Which settings the page edits: all of them, or one cluster's.
    pub fn scope(&self) -> Scope {
        self.tab.scope
    }

    /// The rows of the cluster list.
    pub fn cluster_choices(&self) -> &[ClusterChoice] {
        &self.tab.choices
    }

    /// The cluster the per-cluster tab edits (`None` on the global scope, or before one is
    /// picked).
    pub fn selected_cluster(&self) -> Option<&ClusterId> {
        self.cluster.as_ref()
    }

    /// Lists the clusters of `catalog` on the per-cluster tab and follows its changes. The load
    /// runs on the Kubernetes bridge; the page redraws once when it lands.
    pub fn set_catalog(&mut self, catalog: ClusterCatalog, cx: &mut Context<Self>) {
        let mut changes = catalog.changes();
        self.tab.catalog = Some(catalog);
        self.reload_clusters(cx);
        self.tab.watch = Some(cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this
                    .update(cx, |this, cx| this.reload_clusters(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn reload_clusters(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.tab.catalog.clone() else {
            return;
        };
        self.tab.load = Some(cx.spawn(async move |this, cx| {
            let loaded = spawn_kube(&*cx, async move { catalog.load().await }).await;
            match loaded {
                Ok(Ok(mut entries)) => {
                    entries.sort_by(CatalogEntry::cmp_default);
                    this.update(cx, |this, cx| this.set_catalog_entries(entries, cx))
                        .ok();
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "the cluster catalog could not be read for the settings page");
                }
                Err(error) => {
                    tracing::warn!(%error, "the cluster catalog load did not finish");
                }
            }
        }));
    }

    /// Sets the clusters the list offers to the catalog's `entries` (plus the file's blocks they
    /// do not cover). Keeps the picked cluster when it is still listed, else picks the first.
    pub fn set_catalog_entries(&mut self, entries: Vec<CatalogEntry>, cx: &mut Context<Self>) {
        self.tab.entries = entries;
        self.refresh_choices(cx);
    }

    /// Rebuilds the list from the entries and the file.
    fn refresh_choices(&mut self, cx: &mut Context<Self>) {
        let store = cx.try_global::<SettingsStore>();
        self.tab.choices = choices(&self.tab.entries, store);
        if self.tab.scope == Scope::Cluster {
            let listed = |id: &ClusterId| self.tab.choices.iter().any(|row| row.id == *id);
            if !self.cluster.as_ref().is_some_and(listed) {
                let first = self.tab.choices.first().map(|row| row.id.clone());
                self.pick(first);
            }
        }
        cx.notify();
    }

    /// Shows the all-settings page or the per-cluster tab. Clears the search; the per-cluster
    /// tab opens on the cluster picked last, else the first in the list.
    pub fn set_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab.scope == scope {
            return;
        }
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.query_text.clear();
        self.hits.clear();
        self.highlighted = None;
        let other = self
            .tab
            .parked
            .take()
            .unwrap_or_else(|| self.build_scope(scope, cx));
        let current = Parked {
            model: std::mem::replace(&mut self.model, other.model),
            index: std::mem::replace(&mut self.index, other.index),
            selected: std::mem::replace(&mut self.selected, other.selected),
        };
        self.tab.parked = Some(current);
        self.tab.scope = scope;
        match scope {
            Scope::Global => {
                self.tab.last = self.cluster.take().or(self.tab.last.take());
                self.controls.forget_editors();
            }
            Scope::Cluster => {
                self.refresh_choices(cx);
                let wanted = self
                    .tab
                    .last
                    .clone()
                    .filter(|id| self.tab.choices.iter().any(|row| row.id == *id))
                    .or_else(|| self.tab.choices.first().map(|row| row.id.clone()));
                self.pick(wanted);
            }
        }
        self.content_scroll.scroll_to_item(0);
        cx.notify();
    }

    /// The model and index of `scope`, built from the store's schema.
    fn build_scope(&self, scope: Scope, cx: &Context<Self>) -> Parked {
        let schema = cx
            .try_global::<SettingsStore>()
            .map(SettingsStore::json_schema);
        let model = match (&schema, scope) {
            (None, _) => SettingsPageModel::default(),
            (Some(schema), Scope::Cluster) => cluster_page_model(schema),
            (Some(schema), Scope::Global) => SettingsPageModel::from_schema(schema),
        };
        let index = SearchIndex::new(&model);
        Parked {
            model,
            index,
            selected: 0,
        }
    }

    /// Edits `cluster` on the per-cluster tab. Ignored when the list does not offer it.
    pub fn select_cluster(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        if self.tab.scope != Scope::Cluster
            || !self.tab.choices.iter().any(|row| row.id == *cluster)
        {
            return;
        }
        self.pick(Some(cluster.clone()));
        cx.notify();
    }

    /// Points the controls at `cluster`: their inputs are rebuilt from its values.
    fn pick(&mut self, cluster: Option<ClusterId>) {
        if self.cluster == cluster {
            return;
        }
        self.tab.last = cluster.clone().or(self.tab.last.take());
        self.cluster = cluster;
        self.controls.forget_editors();
    }

    /// How many elements precede the section header in the content column.
    pub(in crate::page) fn header_offset(&self) -> usize {
        usize::from(self.tab.scope == Scope::Cluster)
    }

    /// The two buttons under the title.
    pub(in crate::page) fn scope_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let button = |scope: Scope, label: &'static str, selector: &'static str| {
            let selected = self.tab.scope == scope;
            div()
                .id(selector)
                .debug_selector(move || selector.to_owned())
                .flex_1()
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .rounded(u(tokens.radius.md))
                .cursor_pointer()
                .text_center()
                .when(selected, |button| {
                    button
                        .bg(colors.element_selected)
                        .font_weight(FontWeight::MEDIUM)
                })
                .hover(|style| style.bg(colors.element_hover))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| this.set_scope(scope, window, cx)))
        };
        h_flex()
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.sm))
            .pb(u(tokens.spacing.md))
            .child(button(Scope::Global, "All", "settings-scope-global"))
            .child(button(
                Scope::Cluster,
                "Per cluster",
                "settings-scope-cluster",
            ))
            .into_any_element()
    }

    /// The precedence explanation and the cluster list: one element, the first of the content.
    pub(in crate::page) fn cluster_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let store = cx.try_global::<SettingsStore>();
        let rows: Vec<AnyElement> = self
            .tab
            .choices
            .iter()
            .enumerate()
            .map(|(ix, choice)| {
                let selected = self.cluster.as_ref() == Some(&choice.id);
                let id = choice.id.clone();
                let overrides = choice.override_count(store);
                h_flex()
                    .id(("settings-cluster", ix))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("settings-cluster-{id}")
                    })
                    .gap(u(tokens.spacing.md))
                    .px(u(tokens.spacing.lg))
                    .py(u(tokens.spacing.sm))
                    .rounded(u(tokens.radius.md))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(colors.element_selected))
                    .hover(|style| style.bg(colors.element_hover))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(choice.label(store)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(u(tokens.font.small))
                                    .text_color(colors.text_muted)
                                    .child(choice.detail(store)),
                            ),
                    )
                    .when(overrides > 0, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_size(u(tokens.font.small))
                                .text_color(colors.warning)
                                .child(match overrides {
                                    1 => "1 override".to_owned(),
                                    n => format!("{n} overrides"),
                                }),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_cluster(&id, cx)))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div()
                .debug_selector(|| "settings-cluster-none".into())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.md))
                .text_color(colors.text_muted)
                .child("No clusters yet. Add a kubeconfig to configure a cluster.")
        });
        v_flex()
            .debug_selector(|| "settings-cluster-header".into())
            .gap(u(tokens.spacing.md))
            .pt(u(tokens.spacing.xl))
            .child(
                v_flex()
                    .debug_selector(|| "settings-precedence-explanation".into())
                    .gap(u(tokens.spacing.xs))
                    .p(u(tokens.spacing.lg))
                    .rounded(u(tokens.radius.md))
                    .border_1()
                    .border_color(colors.border_variant)
                    .bg(colors.surface)
                    .text_size(u(tokens.font.small))
                    .text_color(colors.text_muted)
                    .child(
                        div()
                            .text_color(colors.text)
                            .font_weight(FontWeight::MEDIUM)
                            .child("Which value wins"),
                    )
                    .child(EXPLANATION)
                    .child(EDITS_ONLY_THE_CLUSTER),
            )
            .child(
                v_flex()
                    .id("settings-cluster-list")
                    .debug_selector(|| "settings-cluster-list".into())
                    .max_h(u(px(LIST_HEIGHT)))
                    .overflow_y_scroll()
                    .gap(u(tokens.spacing.xs))
                    .children(rows)
                    .children(empty),
            )
            .into_any_element()
    }

    /// The strip under a field on the per-cluster tab: what each layer says, which wins, and a
    /// warning when the override weakens a protection. `None` on the global scope.
    pub(in crate::page) fn precedence_row(
        &self,
        field: &PageField,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.tab.scope != Scope::Cluster {
            return None;
        }
        let cluster = self.cluster.as_ref()?;
        let store = cx.try_global::<SettingsStore>()?;
        let values = store.layer_values(&field.path, Some(cluster));
        let precedence = Precedence::new(&values, store.field_source(&field.path, Some(cluster)));
        let warning = protection_warning(
            &field.path,
            store.effective_value(&field.path, None),
            values.cluster.as_ref(),
        );
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let path = field.path.clone();
        let cells = precedence
            .cells
            .iter()
            .map(|cell| precedence_cell(&path, cell, &colors, cx));
        Some(
            v_flex()
                .gap(u(tokens.spacing.xs))
                .child(
                    h_flex()
                        .debug_selector({
                            let path = path.clone();
                            move || format!("settings-precedence-{path}")
                        })
                        .gap(u(tokens.spacing.xs))
                        .children(cells),
                )
                .child(
                    div()
                        .debug_selector({
                            let path = path.clone();
                            move || {
                                format!(
                                    "settings-precedence-summary-{path}-{}",
                                    precedence.winner.label()
                                )
                            }
                        })
                        .text_size(u(tokens.font.small))
                        .text_color(layer_colour(precedence.winner, &colors))
                        .child(precedence.summary()),
                )
                .children(warning.map(|text| {
                    div()
                        .debug_selector(move || format!("settings-warning-{path}"))
                        .text_size(u(tokens.font.small))
                        .text_color(colors.warning)
                        .child(SharedString::from(format!("Warning: {text}")))
                }))
                .into_any_element(),
        )
    }
}

/// One layer of the strip; the winner is highlighted.
fn precedence_cell(
    path: &SharedString,
    cell: &PrecedenceCell,
    colors: &Colors,
    cx: &gpui::App,
) -> AnyElement {
    let tokens = cx.tokens();
    let layer = cell.layer;
    let path = path.clone();
    let winner = cell.winner;
    div()
        .debug_selector(move || {
            format!(
                "settings-layer-{path}-{}{}",
                layer.label(),
                if winner { "-winner" } else { "" }
            )
        })
        .flex_1()
        .min_w_0()
        .px(u(tokens.spacing.sm))
        .py(u(tokens.spacing.xs))
        .rounded(u(tokens.radius.sm))
        .border_1()
        .border_color(if winner {
            layer_colour(layer, colors)
        } else {
            colors.border_variant
        })
        .when(winner, |cell| cell.bg(colors.element_selected))
        .text_size(u(tokens.font.small))
        .child(
            h_flex()
                .gap(u(tokens.spacing.sm))
                .child(
                    div()
                        .flex_none()
                        .text_color(colors.text_muted)
                        .child(layer.label()),
                )
                .child(
                    div()
                        .truncate()
                        .when(winner, |value| value.font_weight(FontWeight::MEDIUM))
                        .child(cell.value.clone().unwrap_or_else(|| "\u{2014}".to_owned())),
                ),
        )
        .into_any_element()
}

fn layer_colour(source: SettingSource, colors: &Colors) -> Hsla {
    match source {
        SettingSource::Default => colors.text_muted,
        SettingSource::User => colors.accent,
        SettingSource::Cluster => colors.warning,
    }
}
