//! Per-cluster settings in the session manager (E06-S08).
//!
//! The settings store lives in a platform crate the app layer cannot see, so the binary pushes
//! the resolved prefs in: [`ClusterSessionManager::set_prefs_table`] at startup and again
//! whenever the settings change. A session opened afterwards starts from its cluster's prefs
//! (read-only, colour, name, default namespace, exec policy); a session that is already open
//! has the changed fields applied in place, with a [`SessionChange`] for each, and without
//! touching its connection. Only a cluster whose own prefs changed is touched, so a manual
//! change of an unrelated session is never overwritten, and only that cluster's subscribers
//! hear about it.

use std::sync::Arc;

use oxikube_ports::{ClusterContext, ClusterPrefs, ClusterPrefsTable};

use super::entry::Entry;
use super::manager::ClusterSessionManager;
use super::model::ClusterSession;
use super::updates::{SessionChange, UpdateSender};

impl ClusterSessionManager {
    /// Replaces the per-cluster settings and applies them to the open sessions.
    ///
    /// For each open session whose prefs differ from those it last applied: `read_only`,
    /// `colour` and `display_name` change now (each sends its [`SessionChange`]), the exec policy
    /// is used by the next connect, and the rest (default namespace, terminal directory, node
    /// shell, Prometheus, accessible namespaces) is read through
    /// [`ClusterSession::prefs`]. The namespace selection is never moved: `default_namespace`
    /// only decides where a new session starts.
    ///
    /// Returns how many open sessions had changed prefs. Cheap (one short lock per open
    /// session, no I/O), so it may run where the settings changed.
    pub fn set_prefs_table(&self, table: ClusterPrefsTable) -> usize {
        let table = Arc::new(table);
        // Holding the session list (read) keeps `open_with` from adding a session between
        // swapping the table and applying it.
        let sessions = self.shared.sessions.read();
        *self.shared.prefs.write() = table.clone();
        let entries: Vec<_> = sessions.values().cloned().collect();
        drop(sessions);
        let mut changed = 0;
        for entry in entries {
            let mut e = entry.lock();
            let prefs = table.get(&e.id).clone();
            if e.apply_prefs(prefs, &self.shared.updates) {
                changed += 1;
            }
        }
        changed
    }

    /// Opens a `Disconnected` session for `context` configured from its settings (see
    /// [`SessionOptions::from_prefs`](super::config::SessionOptions::from_prefs); the kubeconfig context's namespace is the fallback
    /// default namespace), or returns the existing session unchanged.
    pub fn open_configured(&self, context: &ClusterContext) -> ClusterSession {
        self.shared.open_configured(context).lock().snapshot()
    }
}

impl Entry {
    /// Applies `prefs` as a delta against the prefs applied last. Returns whether they
    /// differed. Updates are sent while the caller holds the entry lock, in order.
    pub(super) fn apply_prefs(&mut self, prefs: Arc<ClusterPrefs>, updates: &UpdateSender) -> bool {
        if *self.prefs == *prefs {
            return false;
        }
        let old = std::mem::replace(&mut self.prefs, prefs.clone());
        if old.read_only != prefs.read_only {
            self.read_only = prefs.read_only;
            updates.send(&self.id, SessionChange::ReadOnlyChanged(prefs.read_only));
        }
        if old.colour != prefs.colour {
            self.colour = prefs.colour;
            updates.send(&self.id, SessionChange::ColourChanged(prefs.colour));
        }
        if old.display_name != prefs.display_name {
            self.display_name = prefs.display_name.clone();
            updates.send(
                &self.id,
                SessionChange::DisplayNameChanged(prefs.display_name.clone()),
            );
        }
        // Read at the next connect; nothing to announce. Like the fields above, only applied
        // when the setting itself changed, so a manual `set_exec_interactivity` survives an
        // unrelated prefs change.
        if old.exec_interactivity != prefs.exec_interactivity {
            self.exec_interactivity = prefs.exec_interactivity;
        }
        true
    }
}
