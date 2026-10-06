//! The rows and the label as plain data.

use oxikube_app::session::namespaces::{NamespaceCatalog, NamespaceSource};
use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};

use crate::namespaces::model::{Row, build_rows, first_selectable, selection_label, step};

fn catalog(names: &[&str], source: NamespaceSource) -> NamespaceCatalog {
    let mut catalog = NamespaceCatalog::unlisted();
    for name in names {
        catalog.names.push((*name).to_owned());
    }
    catalog.source = source;
    catalog
}

fn favs(names: &[&str]) -> NamespaceFavourites {
    names.iter().copied().collect()
}

fn names(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Header(title) => format!("# {title}"),
            Row::All { checked } => format!("All{}", if *checked { " [x]" } else { "" }),
            Row::Namespace(r) => format!(
                "{}{}{}{}",
                r.name,
                if r.checked { " [x]" } else { "" },
                if r.favourite { " *" } else { "" },
                r.slot.map(|s| format!(" {s}")).unwrap_or_default()
            ),
            Row::Add { name } => format!("+ {name}"),
        })
        .collect()
}

#[test]
fn all_then_favourites_then_the_rest() {
    let rows = build_rows(
        "",
        &catalog(
            &["default", "dev", "prod", "stage"],
            NamespaceSource::Cluster,
        ),
        &NamespaceSelection::All,
        &favs(&["prod", "dev"]),
    );

    assert_eq!(
        names(&rows),
        [
            "All [x]",
            "# Favourites",
            "prod * 1",
            "dev * 2",
            "# Namespaces",
            "default",
            "stage"
        ]
    );
}

#[test]
fn ticked_namespaces_are_checked_and_all_is_not() {
    let rows = build_rows(
        "",
        &catalog(&["a", "b"], NamespaceSource::Cluster),
        &NamespaceSelection::from_names(["b"]),
        &NamespaceFavourites::new(),
    );

    assert_eq!(names(&rows), ["All", "# Namespaces", "a", "b [x]"]);
}

#[test]
fn the_search_filters_case_insensitively_and_hides_all() {
    let catalog = catalog(
        &["kube-system", "kube-public", "prod"],
        NamespaceSource::Cluster,
    );
    let rows = build_rows(
        "KUBE",
        &catalog,
        &NamespaceSelection::All,
        &favs(&["kube-system"]),
    );

    assert_eq!(
        names(&rows),
        [
            "# Favourites",
            "kube-system * 1",
            "# Namespaces",
            "kube-public"
        ]
    );
    let none = build_rows("zzz", &catalog, &NamespaceSelection::All, &favs(&[]));
    assert!(
        none.is_empty(),
        "nothing matches and the cluster lists its own: no Add row"
    );
}

#[test]
fn a_selected_or_favourite_name_the_cluster_does_not_list_is_still_shown() {
    let rows = build_rows(
        "",
        &catalog(&["a"], NamespaceSource::Cluster),
        &NamespaceSelection::from_names(["gone"]),
        &favs(&["old"]),
    );

    let listed = names(&rows);
    assert!(listed.contains(&"gone [x]".to_owned()), "{listed:?}");
    assert!(listed.contains(&"old * 1".to_owned()), "{listed:?}");
}

#[test]
fn a_cluster_that_cannot_list_offers_to_add_a_typed_name() {
    let forbidden = catalog(&["team-a"], NamespaceSource::Forbidden);
    let rows = build_rows("team-b", &forbidden, &NamespaceSelection::All, &favs(&[]));
    assert_eq!(names(&rows), ["+ team-b"]);

    let known = build_rows("team-a", &forbidden, &NamespaceSelection::All, &favs(&[]));
    assert_eq!(names(&known), ["team-a"], "already offered, so no Add row");

    let invalid = build_rows(
        "Not Valid!",
        &forbidden,
        &NamespaceSelection::All,
        &favs(&[]),
    );
    assert!(invalid.is_empty());
}

#[test]
fn the_label_summarises_the_selection() {
    assert_eq!(selection_label(&NamespaceSelection::All), "All namespaces");
    assert_eq!(selection_label(&NamespaceSelection::single("prod")), "prod");
    assert_eq!(
        selection_label(&NamespaceSelection::from_names(["b", "a", "c"])),
        "a +2"
    );
}

#[test]
fn highlight_skips_headers_and_stays_at_the_ends() {
    let rows = build_rows(
        "",
        &catalog(&["a", "b"], NamespaceSource::Cluster),
        &NamespaceSelection::All,
        &favs(&["a"]),
    );
    // All, # Favourites, a, # Namespaces, b
    assert_eq!(first_selectable(&rows, 1), Some(2));
    assert_eq!(step(&rows, 0, 1), 2, "the header is skipped");
    assert_eq!(step(&rows, 2, 1), 4);
    assert_eq!(step(&rows, 4, 1), 4, "stays on the last row");
    assert_eq!(step(&rows, 2, -1), 0);
    assert_eq!(step(&rows, 0, -1), 0, "stays on the first row");
}
