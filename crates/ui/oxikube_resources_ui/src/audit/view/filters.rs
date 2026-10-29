//! The filter bar: cluster, verb, initiator, outcome and time window. A change goes into the
//! store query ([`AuditFilters::query`](oxikube_app::audit::query::AuditFilters::query)), so the
//! store's indices answer it, and reads the newest page again; nothing is filtered in memory.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div,
};
use oxikube_app::audit::query::{AuditFilters, AuditWindow};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::safety::MutationVerb;
use oxikube_ui::Sizable as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};

use super::AuditView;
use crate::audit::model::initiator_text;
use oxikube_app::audit::export::outcome_text;

/// A menu row findable as `selector` in a test window.
fn menu_row(selector: String, label: SharedString) -> PopupMenuItem {
    PopupMenuItem::element(move |_, _| {
        let selector = selector.clone();
        div().debug_selector(move || selector).child(label.clone())
    })
}

/// The export menu's row for `format`.
pub(super) fn menu_row_for_export(
    format: oxikube_domain::audit::AuditExportFormat,
) -> PopupMenuItem {
    menu_row(
        format!("audit-export-{}", format.extension()),
        SharedString::from(format!("Export as {}…", format.label())),
    )
}

impl AuditView {
    /// Applies `filters`: reads the newest page for them.
    pub fn set_filters(&mut self, filters: AuditFilters, cx: &mut Context<Self>) {
        if filters == self.filters {
            return;
        }
        self.filters = filters;
        self.pending_new = None;
        self.first_visible = 0;
        self.table.update_quiet(cx, |d| d.model.select(None));
        self.batch = None;
        if !self.model(cx).is_empty() {
            self.table.scroll_to_row(0, cx);
        }
        self.reload(cx);
    }

    /// The cluster filter alone (`audit::Open` with a cluster).
    pub fn set_cluster_filter(&mut self, cluster: Option<ClusterId>, cx: &mut Context<Self>) {
        let mut filters = self.filters.clone();
        filters.cluster = cluster;
        self.set_filters(filters, cx);
    }

    /// Clears every filter.
    pub fn clear_filters(&mut self, cx: &mut Context<Self>) {
        self.set_filters(AuditFilters::default(), cx);
    }

    /// A dropdown over `options` (the first is "Any"): the button says `title: current`, a row
    /// applies its value with `apply`.
    fn dropdown<T: Clone + PartialEq + 'static>(
        &self,
        id: &'static str,
        title: &'static str,
        current: Option<T>,
        options: Vec<(Option<T>, SharedString)>,
        apply: fn(&mut AuditFilters, Option<T>),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = options
            .iter()
            .find(|(value, _)| *value == current)
            .map_or_else(|| SharedString::from("Any"), |(_, label)| label.clone());
        let view = cx.entity().downgrade();
        let button = Button::new(id)
            .label(format!("{title}: {shown}"))
            .ghost()
            .xsmall()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (ix, (value, label)) in options.iter().enumerate() {
                    let (view, value) = (view.clone(), value.clone());
                    let checked = value == current;
                    menu = menu.item(
                        menu_row(format!("{id}-{ix}"), label.clone())
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let value = value.clone();
                                view.update(cx, |view, cx| {
                                    let mut filters = view.filters.clone();
                                    apply(&mut filters, value);
                                    view.set_filters(filters, cx);
                                })
                                .ok();
                            }),
                    );
                }
                menu
            });
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || id.to_owned())
            .child(button)
            .into_any_element()
    }

    /// The five filter dropdowns.
    pub(super) fn filter_dropdowns(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let any = || SharedString::from("Any");
        let mut clusters = vec![(None, any())];
        let names = self.cluster_names();
        for (id, name) in names.known() {
            clusters.push((Some(id), name));
        }
        // A cluster the filter names but no session knows (opened from a record) still shows.
        if let Some(id) = self.filters.cluster.as_ref().filter(|id| {
            !clusters
                .iter()
                .any(|(value, _)| value.as_ref() == Some(*id))
        }) {
            clusters.push((Some(id.clone()), names.name(id)));
        }
        let mut verbs = vec![(None, any())];
        verbs.extend(MutationVerb::ALL.map(|v| (Some(v), SharedString::from(v.as_str()))));
        let mut initiators = vec![(None, any())];
        initiators.extend(Initiator::ALL.map(|i| (Some(i), SharedString::from(initiator_text(i)))));
        let mut outcomes = vec![(None, any())];
        outcomes.extend(
            [
                AuditOutcome::Succeeded,
                AuditOutcome::Failed,
                AuditOutcome::Denied,
                AuditOutcome::Cancelled,
            ]
            .map(|o| (Some(o), SharedString::from(outcome_text(o)))),
        );
        let windows = AuditWindow::ALL
            .map(|w| (Some(w), SharedString::from(w.label())))
            .to_vec();
        vec![
            self.dropdown(
                "audit-cluster",
                "Cluster",
                self.filters.cluster.clone(),
                clusters,
                |f, v| f.cluster = v,
                cx,
            ),
            self.dropdown(
                "audit-verb",
                "Verb",
                self.filters.verb,
                verbs,
                |f, v| f.verb = v,
                cx,
            ),
            self.dropdown(
                "audit-initiator",
                "Initiator",
                self.filters.initiator,
                initiators,
                |f, v| f.initiator = v,
                cx,
            ),
            self.dropdown(
                "audit-outcome",
                "Outcome",
                self.filters.outcome,
                outcomes,
                |f, v| f.outcome = v,
                cx,
            ),
            self.dropdown(
                "audit-window",
                "Time",
                Some(self.filters.window),
                windows,
                |f, v| f.window = v.unwrap_or_default(),
                cx,
            ),
        ]
    }
}
