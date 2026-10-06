//! Count badges (E07-S11): which sidebar entries have a built-in kind to count, and writing the
//! store's answers onto the rows.
//!
//! Pure, so the rules are tested without a window. The panel (`panel/counts`) owns the timer and
//! the store; this file only decides what to ask and where an answer goes.

use std::collections::HashMap;

use gpui::SharedString;
use oxikube_app::{CountState, CountTarget};

use super::rows::{AccessState, Row};
use super::section::{SectionBody, SidebarSection, SidebarTarget};

/// An entry's kind as the sidebar names it: API group and plural resource.
pub type KindKey = (SharedString, SharedString);

/// The kinds the sidebar badges, from the registered sections.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CountPlan {
    /// Every countable kind of an entry the user may list, once each, in section order.
    pub kinds: Vec<(KindKey, CountTarget)>,
    /// Sections that are one kind (Nodes, Namespaces, Events) and show that kind's total.
    pub sections: Vec<(SharedString, KindKey)>,
}

impl CountPlan {
    /// The targets, in order, for [`ResourceStore::counts`](oxikube_app::ResourceStore::counts).
    pub fn targets(&self) -> Vec<CountTarget> {
        self.kinds.iter().map(|(_, t)| t.clone()).collect()
    }
}

/// The kinds worth a badge: built-in kinds ([`CountTarget::core`]) of entries `access` offers.
/// Custom resources and integration items have none (their badge appears when a table opens the
/// feed, which a custom-resource entry cannot know here).
pub fn count_plan(sections: &[SidebarSection], access: &AccessState) -> CountPlan {
    let mut plan = CountPlan::default();
    let mut add = |group: &SharedString, resource: &SharedString| -> Option<KindKey> {
        let target = CountTarget::core(group, resource)?;
        let key = (group.clone(), resource.clone());
        if !plan.kinds.iter().any(|(k, _)| *k == key) {
            plan.kinds.push((key.clone(), target));
        }
        Some(key)
    };
    let mut single: Vec<(SharedString, KindKey)> = Vec::new();
    for section in sections {
        let SectionBody::Entries(entries) = &section.body else {
            continue;
        };
        let mut section_kinds = Vec::new();
        for entry in entries.iter().filter(|e| access.offers(&e.requires)) {
            if let Some(SidebarTarget::Kind { group, resource }) = &entry.target
                && let Some(key) = add(group, resource)
            {
                section_kinds.push(key);
            }
        }
        // A heading that is itself a link to one kind (Events).
        if entries.is_empty()
            && access.offers(&section.requires)
            && let Some(SidebarTarget::Kind { group, resource }) = &section.target
            && let Some(key) = add(group, resource)
        {
            section_kinds.push(key);
        }
        if let [only] = section_kinds.as_slice() {
            single.push((section.id.clone(), only.clone()));
        }
    }
    plan.sections = single;
    plan
}

/// Writes `states` onto `rows`: every entry that lists a counted kind, and every section that is
/// one kind. Rows without an answer keep `None`.
pub fn apply_counts(rows: &mut [Row], plan: &CountPlan, states: &HashMap<KindKey, CountState>) {
    for row in rows {
        match row {
            Row::Entry(entry) => {
                entry.count = match &entry.target {
                    Some(SidebarTarget::Kind { group, resource }) => {
                        states.get(&(group.clone(), resource.clone())).cloned()
                    }
                    _ => None,
                };
            }
            Row::Section(section) => {
                let key = plan
                    .sections
                    .iter()
                    .find(|(id, _)| *id == section.id)
                    .map(|(_, key)| key);
                section.count = key
                    .and_then(|key| states.get(key))
                    .and_then(CountState::count)
                    .map(|c| c.total);
            }
            Row::Group(_) | Row::Notice(_) => {}
        }
    }
}

/// The text of an entry's badge, and its hover text, for a state; `None` draws nothing.
///
/// A dash means "not counted" (over the watch budget, or the feed failed) and says why on hover;
/// "no access" is its own label so it is never mistaken for zero.
pub fn badge_text(state: &CountState) -> Option<(String, Option<String>)> {
    match state {
        CountState::Counted(count) => {
            let hover = count
                .has_health()
                .then(|| format!("{} healthy, {} not", count.healthy, count.unhealthy()));
            Some((count.total.to_string(), hover))
        }
        CountState::Loading => Some(("…".to_owned(), Some("Loading".to_owned()))),
        CountState::NoAccess { message } => Some((
            "no access".to_owned(),
            Some(non_empty(message, "You may not list this kind")),
        )),
        CountState::OverBudget { message } => Some((
            "–".to_owned(),
            Some(non_empty(message, "Not counted: over the watch budget")),
        )),
        CountState::Failed { message } => Some((
            "–".to_owned(),
            Some(non_empty(message, "Could not count this kind")),
        )),
        CountState::NotWatched => None,
    }
}

fn non_empty(message: &str, fallback: &str) -> String {
    if message.is_empty() {
        fallback.to_owned()
    } else {
        message.to_owned()
    }
}
