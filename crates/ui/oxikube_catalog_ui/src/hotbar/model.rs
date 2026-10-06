//! [`HotbarModel`]: which clusters the hotbar shows, in what order. Plain Rust, no GPUI.
//!
//! The hotbar shows every cluster that is either *connected* (its session is not `Disconnected`:
//! it has a tab) or a *favourite* (marked in the catalog). The order is the user's: what they
//! dragged, then, for clusters they never placed, connected ones in the order they opened and
//! favourites by name.

use std::collections::HashMap;

use indexmap::IndexMap;
use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_workspace::cluster_tab::initials;

/// What the model keeps of one open session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLook {
    /// The name shown: the display name, else the context name.
    pub title: String,
    /// The accent colour.
    pub colour: Option<ClusterColour>,
    /// The connection state.
    pub state: ClusterSessionState,
}

/// One tile of the hotbar.
#[derive(Debug, Clone, PartialEq)]
pub struct HotbarEntry {
    /// The cluster.
    pub cluster: ClusterId,
    /// Its name.
    pub name: String,
    /// One or two letters standing for the name.
    pub initials: String,
    /// The accent colour, when set.
    pub colour: Option<ClusterColour>,
    /// The connection state (`Disconnected` for a favourite with no session).
    pub state: ClusterSessionState,
    /// Whether the cluster is a favourite.
    pub favourite: bool,
    /// Whether the cluster has a live session (and so a tab).
    pub connected: bool,
    /// Whether its tab is the displayed one.
    pub active: bool,
}

/// The hotbar's data. See the [module docs](self).
#[derive(Debug, Default)]
pub struct HotbarModel {
    /// Favourite clusters and the name the catalog knows them by.
    favourites: HashMap<ClusterId, String>,
    /// Sessions that are not disconnected, in the order they were seen.
    sessions: IndexMap<ClusterId, SessionLook>,
    /// The user's order (what they dragged), possibly naming clusters not shown now.
    order: Vec<ClusterId>,
    active: Option<ClusterId>,
}

impl HotbarModel {
    /// An empty model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the favourites with the catalog's. Returns whether anything changed.
    pub fn set_favourites(&mut self, favourites: HashMap<ClusterId, String>) -> bool {
        if self.favourites == favourites {
            return false;
        }
        self.favourites = favourites;
        true
    }

    /// Marks or unmarks one favourite (`name` is what the catalog calls it). Returns whether
    /// anything changed.
    pub fn set_favourite(&mut self, cluster: &ClusterId, name: &str, favourite: bool) -> bool {
        if favourite {
            self.favourites
                .insert(cluster.clone(), name.to_owned())
                .as_deref()
                != Some(name)
        } else {
            self.favourites.remove(cluster).is_some()
        }
    }

    /// The name the catalog knows `cluster` by, when it is a favourite.
    pub fn favourite_name(&self, cluster: &ClusterId) -> Option<&str> {
        self.favourites.get(cluster).map(String::as_str)
    }

    /// Records the look of a session. A disconnected one is forgotten (it only stays visible as
    /// a favourite). Returns whether anything changed.
    pub fn set_session(&mut self, cluster: ClusterId, look: SessionLook) -> bool {
        if look.state.phase() == SessionPhase::Disconnected {
            return self.remove_session(&cluster);
        }
        self.sessions.insert(cluster, look.clone()).as_ref() != Some(&look)
    }

    /// Forgets a session. Returns whether it was known.
    pub fn remove_session(&mut self, cluster: &ClusterId) -> bool {
        self.sessions.shift_remove(cluster).is_some()
    }

    /// Keeps only the sessions `keep` accepts. Returns whether any was dropped.
    pub fn retain_sessions(&mut self, mut keep: impl FnMut(&ClusterId) -> bool) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|cluster, _| keep(cluster));
        self.sessions.len() != before
    }

    /// Sets the displayed cluster (`None`: the catalog is shown). Returns whether it changed.
    pub fn set_active(&mut self, active: Option<ClusterId>) -> bool {
        if self.active == active {
            return false;
        }
        self.active = active;
        true
    }

    /// Sets the user's order (as saved). Returns whether it changed.
    pub fn set_order(&mut self, order: Vec<ClusterId>) -> bool {
        if self.order == order {
            return false;
        }
        self.order = order;
        true
    }

    /// The order of the clusters shown now: what [`move_entry`](Self::move_entry) leaves and what
    /// is saved.
    pub fn shown_order(&self) -> Vec<ClusterId> {
        self.shown().into_iter().cloned().collect()
    }

    /// The tiles, top to bottom.
    pub fn entries(&self) -> Vec<HotbarEntry> {
        self.shown()
            .into_iter()
            .map(|cluster| self.entry(cluster))
            .collect()
    }

    fn is_shown(&self, cluster: &ClusterId) -> bool {
        self.sessions.contains_key(cluster) || self.favourites.contains_key(cluster)
    }

    /// The clusters shown, top to bottom.
    fn shown(&self) -> Vec<&ClusterId> {
        let mut shown: Vec<&ClusterId> =
            Vec::with_capacity(self.sessions.len() + self.favourites.len());
        // The user's placement first, then what they never placed.
        for cluster in &self.order {
            if self.is_shown(cluster) && !shown.contains(&cluster) {
                shown.push(cluster);
            }
        }
        for cluster in self.sessions.keys() {
            if !shown.contains(&cluster) {
                shown.push(cluster);
            }
        }
        let mut rest: Vec<(&ClusterId, &String)> = self
            .favourites
            .iter()
            .filter(|(cluster, _)| !shown.contains(cluster))
            .collect();
        rest.sort_by(|(a_id, a), (b_id, b)| {
            a.to_lowercase()
                .cmp(&b.to_lowercase())
                .then_with(|| a_id.cmp(b_id))
        });
        shown.extend(rest.into_iter().map(|(cluster, _)| cluster));
        shown
    }

    fn entry(&self, cluster: &ClusterId) -> HotbarEntry {
        let session = self.sessions.get(cluster);
        let name = session
            .map(|s| s.title.clone())
            .or_else(|| self.favourites.get(cluster).cloned())
            .unwrap_or_else(|| cluster.to_string());
        HotbarEntry {
            cluster: cluster.clone(),
            initials: initials(&name),
            name,
            colour: session.and_then(|s| s.colour),
            state: session.map_or(ClusterSessionState::Disconnected, |s| s.state.clone()),
            favourite: self.favourites.contains_key(cluster),
            connected: session.is_some(),
            active: self.active.as_ref() == Some(cluster),
        }
    }

    /// The tile of `cluster`, if it is shown.
    pub fn find(&self, cluster: &ClusterId) -> Option<HotbarEntry> {
        self.is_shown(cluster).then(|| self.entry(cluster))
    }

    /// Number of tiles.
    pub fn len(&self) -> usize {
        let extra = self
            .favourites
            .keys()
            .filter(|cluster| !self.sessions.contains_key(*cluster))
            .count();
        self.sessions.len() + extra
    }

    /// Whether nothing is shown.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Moves `cluster` to tile index `to` (counted among the tiles shown now; past the end means
    /// last). The new order becomes the user's. Returns whether the order changed.
    pub fn move_entry(&mut self, cluster: &ClusterId, to: usize) -> bool {
        let mut order = self.shown_order();
        let Some(from) = order.iter().position(|c| c == cluster) else {
            return false;
        };
        let moved = order.remove(from);
        order.insert(to.min(order.len()), moved);
        // Clusters the user placed earlier that are not shown now keep their slot at the end, so
        // a disconnected non-favourite comes back where it was dragged to.
        let hidden: Vec<ClusterId> = self
            .order
            .iter()
            .filter(|c| !order.contains(c))
            .cloned()
            .collect();
        let shown_changed = order != self.shown_order();
        order.extend(hidden);
        self.order = order;
        shown_changed
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn id(name: &str) -> ClusterId {
        ClusterId::new("/kube/config", &ContextName::new(name))
    }

    fn look(title: &str, state: ClusterSessionState) -> SessionLook {
        SessionLook {
            title: title.to_owned(),
            colour: None,
            state,
        }
    }

    fn names(model: &HotbarModel) -> Vec<String> {
        model.entries().into_iter().map(|e| e.name).collect()
    }

    fn model() -> HotbarModel {
        let mut model = HotbarModel::new();
        model.set_favourite(&id("zeta"), "zeta", true);
        model.set_favourite(&id("Alpha"), "Alpha", true);
        model.set_session(id("prod"), look("prod", ClusterSessionState::Ready));
        model.set_session(id("dev"), look("dev", ClusterSessionState::Connecting));
        model
    }

    #[test]
    fn connected_clusters_come_first_in_open_order_then_favourites_by_name() {
        assert_eq!(names(&model()), ["prod", "dev", "Alpha", "zeta"]);
    }

    #[test]
    fn a_cluster_that_is_both_shows_once_with_both_marks() {
        let mut model = model();
        model.set_favourite(&id("prod"), "prod", true);
        let prod = model.find(&id("prod")).unwrap();
        assert!(prod.favourite && prod.connected);
        assert_eq!(names(&model).iter().filter(|n| *n == "prod").count(), 1);
        assert_eq!(model.len(), 4);
    }

    #[test]
    fn a_disconnected_non_favourite_leaves_and_a_favourite_stays() {
        let mut model = model();
        assert!(model.set_session(id("prod"), look("prod", ClusterSessionState::Disconnected)));
        assert_eq!(names(&model), ["dev", "Alpha", "zeta"]);
        model.set_favourite(&id("dev"), "dev", true);
        model.set_session(id("dev"), look("dev", ClusterSessionState::Disconnected));
        let dev = model.find(&id("dev")).unwrap();
        assert!(
            !dev.connected && dev.favourite,
            "still there as a favourite"
        );
        assert_eq!(dev.state, ClusterSessionState::Disconnected);
        assert!(model.find(&id("prod")).is_none());
    }

    #[test]
    fn the_session_name_wins_over_the_catalog_name() {
        let mut model = HotbarModel::new();
        model.set_favourite(&id("ctx"), "ctx", true);
        model.set_session(id("ctx"), look("Production EU", ClusterSessionState::Ready));
        let entry = model.find(&id("ctx")).unwrap();
        assert_eq!(entry.name, "Production EU");
        assert_eq!(entry.initials, "PE");
    }

    #[test]
    fn dragging_places_a_tile_and_keeps_the_rest_in_order() {
        let mut model = model();
        assert!(model.move_entry(&id("zeta"), 0));
        assert_eq!(names(&model), ["zeta", "prod", "dev", "Alpha"]);
        assert!(model.move_entry(&id("zeta"), 99), "past the end is last");
        assert_eq!(names(&model), ["prod", "dev", "Alpha", "zeta"]);
        assert!(!model.move_entry(&id("zeta"), 3), "already there");
        assert!(!model.move_entry(&id("unknown"), 0));
    }

    #[test]
    fn a_placed_order_is_followed_and_new_clusters_join_the_end() {
        let mut model = model();
        model.set_order(vec![id("zeta"), id("dev"), id("prod")]);
        assert_eq!(names(&model), ["zeta", "dev", "prod", "Alpha"]);
        model.set_session(id("fresh"), look("fresh", ClusterSessionState::Connecting));
        assert_eq!(names(&model), ["zeta", "dev", "prod", "fresh", "Alpha"]);
    }

    #[test]
    fn a_hidden_cluster_keeps_the_slot_it_was_dragged_to() {
        let mut model = model();
        model.move_entry(&id("prod"), 3);
        assert_eq!(names(&model), ["dev", "Alpha", "zeta", "prod"]);
        model.set_session(id("prod"), look("prod", ClusterSessionState::Disconnected));
        assert_eq!(names(&model), ["dev", "Alpha", "zeta"]);
        // Connecting it again puts it back where the user put it.
        model.set_session(id("prod"), look("prod", ClusterSessionState::Ready));
        assert_eq!(names(&model), ["dev", "Alpha", "zeta", "prod"]);
    }

    #[test]
    fn the_active_cluster_is_marked() {
        let mut model = model();
        assert!(model.set_active(Some(id("dev"))));
        assert!(!model.set_active(Some(id("dev"))));
        let active: Vec<_> = model
            .entries()
            .into_iter()
            .filter(|e| e.active)
            .map(|e| e.name)
            .collect();
        assert_eq!(active, ["dev"]);
        model.set_active(None);
        assert!(model.entries().iter().all(|e| !e.active));
    }

    #[test]
    fn unchanged_updates_report_no_change() {
        let mut model = model();
        assert!(!model.set_session(id("prod"), look("prod", ClusterSessionState::Ready)));
        assert!(!model.set_favourite(&id("zeta"), "zeta", true));
        assert!(!model.set_favourite(&id("nobody"), "nobody", false));
        assert!(!model.remove_session(&id("nobody")));
        assert!(
            model.set_favourite(&id("zeta"), "Zeta", true),
            "a new name is a change"
        );
    }

    #[test]
    fn retain_drops_sessions_that_closed() {
        let mut model = model();
        assert!(model.retain_sessions(|c| *c == id("prod")));
        assert_eq!(names(&model), ["prod", "Alpha", "zeta"]);
        assert!(!model.retain_sessions(|_| true));
    }
}
