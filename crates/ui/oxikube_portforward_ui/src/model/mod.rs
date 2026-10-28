//! The panel's view-model: [`Row`]s built from the manager's [`ForwardInfo`]s and the
//! [`PanelModel`] that orders, filters and selects them. Pure: no gpui, so it is tested without a
//! window.
//!
//! Rows are ordered by cluster, then target, then id, so a forward keeps its place while its state
//! changes. The selection follows the forward (its id), not the row number; when the selected
//! forward goes away the selection stays on the same line number, clamped. Every cell's text is
//! built once per refresh, so drawing the visible rows formats nothing.

pub mod format;

#[cfg(test)]
mod tests;

use gpui::SharedString;
use oxikube_app::{ForwardId, ForwardInfo, ForwardState};
use oxikube_domain::ids::ClusterId;

/// Which look a status gets: its colour and icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    /// Resolving the target and binding the listener.
    Starting,
    /// Listening and serving.
    Active,
    /// Dropped; trying again.
    Reconnecting,
    /// Not running and not retrying: needs a restart.
    Failed,
    /// Stopped by the user.
    Stopped,
}

impl StatusKind {
    fn of(state: &ForwardState) -> Self {
        match state {
            ForwardState::Starting => Self::Starting,
            ForwardState::Active { .. } => Self::Active,
            ForwardState::Reconnecting { .. } => Self::Reconnecting,
            ForwardState::Failed(_) => Self::Failed,
            ForwardState::Stopped => Self::Stopped,
        }
    }
}

/// One forward as a table row, every cell already text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The forward's id in the manager.
    pub id: ForwardId,
    /// The cluster it belongs to.
    pub cluster: ClusterId,
    /// The cluster as the user knows it (the context name).
    pub cluster_label: SharedString,
    /// `namespace/kind/name`.
    pub target: SharedString,
    /// `local -> remote`.
    pub ports: SharedString,
    /// Which look the status gets.
    pub kind: StatusKind,
    /// The status text, with the attempt and next retry of a reconnect.
    pub status: SharedString,
    /// Bytes in and out and open connections.
    pub traffic: SharedString,
    /// How long it has been running, `-` when it is not.
    pub uptime: SharedString,
    /// The local URL, once the listener is bound.
    pub url: Option<String>,
    /// Whether a supervisor runs for it (so "stop" applies; otherwise "start").
    pub running: bool,
    /// Why it is not fine, with what to do about it.
    pub detail: Option<SharedString>,
    /// Lower-cased text the filter looks in.
    haystack: String,
}

impl Row {
    /// The row of `info`; `cluster_label` names its cluster and `now` measures the uptime.
    pub fn new(info: &ForwardInfo, cluster_label: SharedString, now: jiff::Timestamp) -> Self {
        let target: SharedString = format::target(info).into();
        let ports: SharedString = format::ports(info).into();
        let status: SharedString = format::status_text(info).into();
        let haystack = format!("{cluster_label} {target} {ports} {status}").to_lowercase();
        Self {
            id: info.id,
            cluster: info.spec.target.cluster.clone(),
            cluster_label,
            target,
            ports,
            kind: StatusKind::of(&info.state),
            status,
            traffic: format::traffic(&info.stats).into(),
            uptime: format::uptime(info, now).into(),
            url: info.url(),
            running: info.state.is_running(),
            detail: format::detail(info).map(Into::into),
            haystack,
        }
    }
}

/// What the toolbar asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only this cluster's forwards (`None`: all).
    pub cluster: Option<ClusterId>,
    /// Rows containing this text (case-insensitive) in cluster, target, ports or status.
    pub text: String,
}

/// The forwards of every cluster, ordered and filtered, with the selection.
#[derive(Debug, Default)]
pub struct PanelModel {
    all: Vec<Row>,
    /// Indices into `all` of the rows that pass the filter, in order.
    visible: Vec<usize>,
    filter: Filter,
    selected: Option<ForwardId>,
    /// The selected line, kept when its forward goes away.
    selected_ix: usize,
}

impl PanelModel {
    /// An empty model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the manager's current forwards. Returns whether anything a cell shows changed.
    pub fn set_rows(&mut self, mut rows: Vec<Row>) -> bool {
        rows.sort_by(|a, b| {
            (&a.cluster_label, &a.target, a.id).cmp(&(&b.cluster_label, &b.target, b.id))
        });
        if rows == self.all {
            return false;
        }
        self.all = rows;
        self.refilter();
        true
    }

    /// Sets the text filter; returns whether it changed.
    pub fn set_text(&mut self, text: &str) -> bool {
        if self.filter.text == text {
            return false;
        }
        self.filter.text = text.to_owned();
        self.refilter();
        true
    }

    /// Shows only `cluster`'s forwards (`None`: all of them).
    pub fn set_cluster(&mut self, cluster: Option<ClusterId>) {
        if self.filter.cluster != cluster {
            self.filter.cluster = cluster;
            self.refilter();
        }
    }

    /// Moves the cluster filter to the next cluster that has forwards, then back to all.
    pub fn cycle_cluster(&mut self) {
        let clusters = self.clusters();
        let next = match &self.filter.cluster {
            None => clusters.first().map(|(id, _)| id.clone()),
            Some(current) => clusters
                .iter()
                .position(|(id, _)| id == current)
                .and_then(|ix| clusters.get(ix + 1))
                .map(|(id, _)| id.clone()),
        };
        self.set_cluster(next);
    }

    /// The clusters that have forwards, with their labels, in order.
    pub fn clusters(&self) -> Vec<(ClusterId, SharedString)> {
        let mut out: Vec<(ClusterId, SharedString)> = Vec::new();
        for row in &self.all {
            if !out.iter().any(|(id, _)| *id == row.cluster) {
                out.push((row.cluster.clone(), row.cluster_label.clone()));
            }
        }
        out
    }

    /// The cluster filter.
    pub fn cluster_filter(&self) -> Option<&ClusterId> {
        self.filter.cluster.as_ref()
    }

    /// The text filter.
    pub fn text_filter(&self) -> &str {
        &self.filter.text
    }

    fn refilter(&mut self) {
        let text = self.filter.text.to_lowercase();
        let cluster = &self.filter.cluster;
        self.visible = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, row)| cluster.as_ref().is_none_or(|c| *c == row.cluster))
            .filter(|(_, row)| text.is_empty() || row.haystack.contains(&text))
            .map(|(ix, _)| ix)
            .collect();
        self.reselect();
    }

    /// Keeps the selection on its forward; when it left, on the same line number.
    fn reselect(&mut self) {
        if let Some(id) = self.selected
            && let Some(ix) = self.position(id)
        {
            self.selected_ix = ix;
            return;
        }
        if self.visible.is_empty() {
            self.selected = None;
            self.selected_ix = 0;
        } else if self.selected.is_some() {
            self.selected_ix = self.selected_ix.min(self.visible.len() - 1);
            self.selected = self.row(self.selected_ix).map(|row| row.id);
        }
    }

    fn position(&self, id: ForwardId) -> Option<usize> {
        self.visible.iter().position(|&ix| self.all[ix].id == id)
    }

    /// How many rows pass the filter.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    /// Whether no row passes the filter.
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// How many forwards there are, filtered or not.
    pub fn total(&self) -> usize {
        self.all.len()
    }

    /// How many forwards are `Failed`, filtered or not.
    pub fn failed(&self) -> usize {
        self.all
            .iter()
            .filter(|row| row.kind == StatusKind::Failed)
            .count()
    }

    /// The visible row at line `ix`.
    pub fn row(&self, ix: usize) -> Option<&Row> {
        self.visible.get(ix).map(|&i| &self.all[i])
    }

    /// The visible rows, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.visible.iter().map(|&ix| &self.all[ix])
    }

    /// The selected row.
    pub fn selected(&self) -> Option<&Row> {
        self.selected.and_then(|id| self.row(self.position(id)?))
    }

    /// The selected line.
    pub fn selected_ix(&self) -> Option<usize> {
        self.selected.and_then(|id| self.position(id))
    }

    /// Whether line `ix` is the selected one.
    pub fn is_selected(&self, ix: usize) -> bool {
        self.selected_ix() == Some(ix)
    }

    /// Selects line `ix`; returns whether the selection changed.
    pub fn select(&mut self, ix: usize) -> bool {
        let Some(id) = self.row(ix).map(|row| row.id) else {
            return false;
        };
        let changed = self.selected != Some(id);
        self.selected = Some(id);
        self.selected_ix = ix;
        changed
    }

    /// Moves the selection `delta` lines (selecting the first line when there is none). Returns
    /// the new line when it moved.
    pub fn step(&mut self, delta: isize) -> Option<usize> {
        if self.visible.is_empty() {
            return None;
        }
        let next = match self.selected_ix() {
            Some(ix) => ix.saturating_add_signed(delta).min(self.visible.len() - 1),
            None => 0,
        };
        self.select(next).then_some(next)
    }

    /// Selects the first or the last line; returns it when the selection moved.
    pub fn select_end(&mut self, last: bool) -> Option<usize> {
        let ix = if last {
            self.visible.len().checked_sub(1)?
        } else {
            0
        };
        self.select(ix).then_some(ix)
    }
}
