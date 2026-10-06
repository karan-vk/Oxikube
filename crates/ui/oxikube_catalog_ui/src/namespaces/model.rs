//! The selector's rows and trigger label, as plain data (no gpui): what the dropdown lists for
//! a query, a selection and the favourites. The search filters locally, so typing never waits
//! on the cluster.

use gpui::SharedString;
use oxikube_app::session::namespaces::{
    NamespaceCatalog, NamespaceSource, favourite_slot, is_valid_namespace_name,
};
use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};

/// One line of the dropdown. Every row has the same height (the list is virtualised).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A section title (`Favourites`, `Namespaces`). Not selectable.
    Header(SharedString),
    /// "All namespaces", shortcut `0`.
    All {
        /// Whether the selection is `All`.
        checked: bool,
    },
    /// One namespace.
    Namespace(NamespaceRow),
    /// "Add `name`": a typed name the cluster did not list (offered when it cannot list at
    /// all). Adding it also selects it.
    Add {
        /// The valid namespace name that was typed.
        name: String,
    },
}

/// A namespace row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceRow {
    /// The namespace name.
    pub name: String,
    /// Whether it is in the selection (never true while the selection is `All`).
    pub checked: bool,
    /// Whether it is a favourite.
    pub favourite: bool,
    /// The digit key that selects it (`1`-`9`), for the first nine favourites.
    pub slot: Option<u8>,
}

impl Row {
    /// Whether the row can be highlighted and activated.
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Row::Header(_))
    }

    /// The namespace name of a namespace row.
    pub fn namespace(&self) -> Option<&str> {
        match self {
            Row::Namespace(row) => Some(&row.name),
            _ => None,
        }
    }
}

/// The rows for `query`.
///
/// Without a query: `All namespaces`, then the favourites (each with its digit), then every
/// other namespace. With a query: the namespaces whose name contains it (case-insensitive),
/// favourites first. Names that are selected or favourites but not in the catalog (a namespace
/// typed for a restricted cluster, a deleted one) are listed too, so they can be unticked.
/// When the cluster does not list its namespaces and `query` is a valid name nobody listed,
/// the last row offers to add it.
pub fn build_rows(
    query: &str,
    catalog: &NamespaceCatalog,
    selection: &NamespaceSelection,
    favourites: &NamespaceFavourites,
) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut names: Vec<&str> = catalog.names.iter().map(String::as_str).collect();
    names.extend(selection.names());
    names.extend(favourites.iter());
    names.sort_unstable();
    names.dedup();

    let row = |name: &str| {
        Row::Namespace(NamespaceRow {
            name: name.to_owned(),
            checked: selection.names().any(|n| n == name),
            favourite: favourites.contains(name),
            slot: favourite_slot(favourites, name),
        })
    };
    // Namespace names are lowercase by Kubernetes' rules, so only the query needs folding.
    let matching = |name: &&str| query.is_empty() || name.contains(query.as_str());

    let mut rows = Vec::new();
    let favourite_names: Vec<&str> = favourites.iter().filter(matching).collect();
    if query.is_empty() {
        rows.push(Row::All {
            checked: selection.is_all(),
        });
    }
    if !favourite_names.is_empty() {
        rows.push(Row::Header("Favourites".into()));
        rows.extend(favourite_names.iter().map(|n| row(n)));
    }
    let others: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| !favourites.contains(n) && matching(n))
        .collect();
    if !others.is_empty() {
        if !favourite_names.is_empty() || query.is_empty() {
            rows.push(Row::Header("Namespaces".into()));
        }
        rows.extend(others.iter().map(|n| row(n)));
    }
    let offers_add = catalog.source != NamespaceSource::Cluster
        && is_valid_namespace_name(&query)
        && !names.contains(&query.as_str());
    if offers_add {
        rows.push(Row::Add { name: query });
    }
    rows
}

/// The index of the first selectable row at or after `from`, wrapping to the start.
pub fn first_selectable(rows: &[Row], from: usize) -> Option<usize> {
    (from..rows.len())
        .chain(0..from.min(rows.len()))
        .find(|&i| rows[i].is_selectable())
}

/// The next selectable row after `from` in `direction` (`1` or `-1`), staying put at the ends.
pub fn step(rows: &[Row], from: usize, direction: isize) -> usize {
    let mut i = from as isize + direction;
    while i >= 0 && (i as usize) < rows.len() {
        if rows[i as usize].is_selectable() {
            return i as usize;
        }
        i += direction;
    }
    from
}

/// The trigger's text: `All namespaces`, `prod`, or `prod +2` for three.
pub fn selection_label(selection: &NamespaceSelection) -> String {
    let mut names = selection.names();
    let Some(first) = names.next() else {
        return "All namespaces".to_owned();
    };
    match names.count() {
        0 => first.to_owned(),
        more => format!("{first} +{more}"),
    }
}
