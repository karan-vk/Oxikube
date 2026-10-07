//! [`ColumnLayout`]: a kind's columns ([`ColumnProvider::columns`]) as the user arranged them:
//! which are shown, in what order, how wide, and which one sorts.
//!
//! Pure and synchronous: built from the provider's columns and the saved [`ColumnPrefs`], changed
//! by the table's events, and turned back into [`ColumnPrefs`] to save. Defaults: default
//! columns shown and wide ones hidden (`kubectl -o wide`), in the provider's order. The name
//! column cannot be hidden, and at least one column always shows.
//!
//! [`ColumnProvider::columns`]: oxikube_app::ColumnProvider::columns

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_app::columns::{Align, SortKind};
use oxikube_app::{Column, ColumnId};

use super::prefs::{ColumnPrefs, SavedSort};

/// The column the layout never hides.
const ALWAYS_SHOWN: &str = ColumnId::NAME;

/// The arranged columns of one table. See the [`table`](crate::table) module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnLayout {
    /// Every column the kind has, in the provider's order.
    all: Arc<[Column]>,
    /// Indices into `all`, in the user's order (every column once).
    order: Vec<usize>,
    /// Whether each column of `all` shows.
    shown: Vec<bool>,
    /// Indices into `all` of the shown columns, in display order.
    visible: Vec<usize>,
    /// Widths the user chose, unscaled, by id.
    widths: BTreeMap<String, f32>,
    /// The sort column and whether it is descending.
    sort: Option<(ColumnId, bool)>,
    /// Choices the user made that the current columns lack, kept so a save does not drop them
    /// and a later column set (a Table feed's server columns, metrics coming back) restores
    /// them: visibility and widths of absent columns, the saved order when it names absent
    /// columns (their slots), and a sort on an absent column.
    foreign: ColumnPrefs,
}

impl ColumnLayout {
    /// The layout of `all` under `prefs`.
    pub fn new(all: Arc<[Column]>, prefs: &ColumnPrefs) -> Self {
        let index = |id: &str| all.iter().position(|c| c.id == *id);
        let mut order: Vec<usize> = Vec::with_capacity(all.len());
        for id in &prefs.order {
            if let Some(ix) = index(id)
                && !order.contains(&ix)
            {
                order.push(ix);
            }
        }
        let rest: Vec<usize> = (0..all.len()).filter(|ix| !order.contains(ix)).collect();
        order.extend(rest);
        let shown = all
            .iter()
            .map(|c| {
                c.id == ALWAYS_SHOWN || prefs.visible.get(c.id.as_str()).copied().unwrap_or(!c.wide)
            })
            .collect();
        let known = |(id, _): &(String, f32)| index(id).is_some();
        let (widths, foreign_widths): (BTreeMap<_, _>, BTreeMap<_, _>) = prefs
            .widths
            .iter()
            .map(|(id, w)| (id.clone(), *w))
            .partition(known);
        let mut foreign_order: Vec<String> = Vec::new();
        if prefs.order.iter().any(|id| index(id).is_none()) {
            for id in &prefs.order {
                if !foreign_order.contains(id) {
                    foreign_order.push(id.clone());
                }
            }
        }
        let (sort, foreign_sort) = match &prefs.sort {
            Some(s) if index(&s.column).is_some() => {
                (Some((ColumnId::new(s.column.as_str()), s.descending)), None)
            }
            other => (None, other.clone()),
        };
        let foreign = ColumnPrefs {
            order: foreign_order,
            sort: foreign_sort,
            visible: prefs
                .visible
                .iter()
                .filter(|(id, _)| index(id).is_none())
                .map(|(id, v)| (id.clone(), *v))
                .collect(),
            widths: foreign_widths,
            ..ColumnPrefs::default()
        };
        let mut layout = Self {
            widths,
            all,
            order,
            shown,
            visible: Vec::new(),
            sort,
            foreign,
        };
        layout.ensure_one_shown();
        layout.reindex();
        if !layout.foreign.order.is_empty() {
            // Every column with a slot: the same layout whichever prefs it was read from.
            layout.foreign.order = layout.saved_order();
        }
        layout
    }

    /// Every column the kind has (shown or not), in the user's order.
    pub fn columns(&self) -> impl Iterator<Item = (&Column, bool)> {
        self.order.iter().map(|&ix| (&self.all[ix], self.shown[ix]))
    }

    /// How many columns show.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// The shown column at display position `ix`.
    pub fn visible(&self, ix: usize) -> Option<&Column> {
        self.visible.get(ix).map(|&i| &self.all[i])
    }

    /// The display position of column `id`, if it shows.
    pub fn visible_index(&self, id: &ColumnId) -> Option<usize> {
        self.visible.iter().position(|&i| self.all[i].id == *id)
    }

    /// The ids of the shown columns, in display order.
    pub fn visible_ids(&self) -> Vec<ColumnId> {
        self.visible
            .iter()
            .map(|&i| self.all[i].id.clone())
            .collect()
    }

    /// Whether column `id` shows.
    pub fn is_shown(&self, id: &ColumnId) -> bool {
        self.visible_index(id).is_some()
    }

    /// Shows or hides column `id`. Returns whether anything changed (the name column, and the
    /// last shown column, stay shown).
    pub fn set_shown(&mut self, id: &ColumnId, shown: bool) -> bool {
        let Some(ix) = self.all.iter().position(|c| c.id == *id) else {
            return false;
        };
        if self.shown[ix] == shown || (!shown && (*id == ALWAYS_SHOWN || self.visible.len() <= 1)) {
            return false;
        }
        self.shown[ix] = shown;
        if !shown && self.sort.as_ref().is_some_and(|(s, _)| s == id) {
            self.sort = None;
        }
        self.reindex();
        true
    }

    /// The shown column at display position `from` moved to `to` (a header drag). Hidden
    /// columns keep their place among the others.
    pub fn move_visible(&mut self, from: usize, to: usize) -> bool {
        let (Some(&moved), Some(&target)) = (self.visible.get(from), self.visible.get(to)) else {
            return false;
        };
        if from == to {
            return false;
        }
        self.order.retain(|&ix| ix != moved);
        let at = self
            .order
            .iter()
            .position(|&ix| ix == target)
            .unwrap_or(self.order.len());
        // Moving right lands after the target, moving left before it.
        let at = if to > from { at + 1 } else { at };
        self.order.insert(at.min(self.order.len()), moved);
        self.reindex();
        true
    }

    /// The width the user chose for column `id`, unscaled.
    pub fn width(&self, id: &ColumnId) -> Option<f32> {
        self.widths.get(id.as_str()).copied()
    }

    /// The width a column starts at, unscaled, when the user has not resized it.
    ///
    /// Sized to the longest value the column usually holds, so the default pod columns (Name,
    /// Status, Ready, Restarts, Age, Node, IP) fit a 1280 px window with the sidebar open
    /// (`default_pod_columns_fit_1280px_with_the_sidebar_open`).
    pub fn default_width(column: &Column) -> f32 {
        match (column.id.as_str(), column.sort, column.align) {
            (ColumnId::NAME, _, _) => 260.,
            (ColumnId::NAMESPACE, _, _) => 150.,
            (ColumnId::LABELS, _, _) => 240.,
            // Long phases: `ContainerCreating`, `CrashLoopBackOff`.
            ("status", _, _) => 170.,
            // `12 (279d ago)`: the count and the age of the last restart.
            ("restarts", _, _) => 130.,
            ("ready", _, _) => 70.,
            // `kind-control-plane`, `ip-10-0-123-45.eu-west-1.compute.internal` (truncated).
            ("node", _, _) => 140.,
            // An IPv4 address; an IPv6 one truncates to a tooltip.
            ("ip", _, _) => 110.,
            (_, SortKind::Age, _) => 70.,
            (_, SortKind::Number | SortKind::Quantity, _) | (_, _, Align::Right) => 90.,
            _ => 140.,
        }
    }

    /// The user resized the shown columns: `widths` in display order, unscaled. Returns whether
    /// any width changed.
    pub fn set_widths(&mut self, widths: &[f32]) -> bool {
        let mut changed = false;
        for (&ix, &width) in self.visible.iter().zip(widths) {
            let column = &self.all[ix];
            let current = self
                .widths
                .get(column.id.as_str())
                .copied()
                .unwrap_or_else(|| Self::default_width(column));
            if (current - width).abs() >= 0.5 {
                self.widths.insert(column.id.to_string(), width);
                changed = true;
            }
        }
        changed
    }

    /// The sort: column and descending.
    pub fn sort(&self) -> Option<&(ColumnId, bool)> {
        self.sort.as_ref()
    }

    /// Sets the sort; `None` is the store's default order. The user chose, so a saved sort on
    /// an absent column is forgotten.
    pub fn set_sort(&mut self, sort: Option<(ColumnId, bool)>) -> bool {
        let forgot = self.foreign.sort.take().is_some();
        if self.sort == sort {
            return forgot;
        }
        self.sort = sort;
        true
    }

    /// The layout as preferences to save: the user's order, every visibility that differs from
    /// the default, the widths and the sort, plus what was saved for columns this kind lacks.
    pub fn prefs(&self) -> ColumnPrefs {
        let mut visible = self.foreign.visible.clone();
        for (ix, column) in self.all.iter().enumerate() {
            if self.shown[ix] == column.wide {
                visible.insert(column.id.to_string(), self.shown[ix]);
            }
        }
        let mut widths = self.foreign.widths.clone();
        widths.extend(self.widths.iter().map(|(id, w)| (id.clone(), *w)));
        ColumnPrefs {
            version: super::prefs::PREFS_VERSION,
            order: self.saved_order(),
            visible,
            widths,
            sort: match &self.sort {
                Some((column, descending)) => Some(SavedSort {
                    column: column.to_string(),
                    descending: *descending,
                }),
                None => self.foreign.sort.clone(),
            },
        }
    }

    /// The user's order to save: the current columns in their order, with the absent columns
    /// of the saved order kept in their slots (each slot of a present column takes the next
    /// present column in the current order).
    fn saved_order(&self) -> Vec<String> {
        let mut present = self.order.iter().map(|&ix| self.all[ix].id.to_string());
        let mut out = Vec::with_capacity(self.foreign.order.len().max(self.all.len()));
        for id in &self.foreign.order {
            if self.all.iter().any(|c| c.id == *id.as_str()) {
                out.extend(present.next());
            } else {
                out.push(id.clone());
            }
        }
        out.extend(present);
        out
    }

    fn ensure_one_shown(&mut self) {
        if !self.shown.iter().any(|s| *s)
            && let Some(first) = self.order.first()
        {
            self.shown[*first] = true;
        }
    }

    fn reindex(&mut self) {
        self.visible = self
            .order
            .iter()
            .copied()
            .filter(|&ix| self.shown[ix])
            .collect();
    }
}
