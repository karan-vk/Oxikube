//! Guard: every crate that registers a setting must be linked into the schema generator.
//!
//! `inventory` only sees registrations from crates linked into the generator binary, so a
//! setting registered anywhere else silently never reaches `settings.schema.json` (and the
//! schema's `additionalProperties: false` then flags the user's key as unknown). `--check`
//! compares the schema with the generator's own output, so it cannot see that gap; this
//! guard turns it into a loud failure until the generator links every settings crate (E05-S06b).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, bail};
use cargo_metadata::{DependencyKind, Metadata, Package};

/// The crate that defines `register_settings!`.
const SETTINGS_CRATE: &str = "oxikube_settings";

/// Fail when a workspace crate registers settings but is not linked into `generator`.
pub fn ensure_generator_links_every_settings_crate(
    metadata: &Metadata,
    generator: &str,
) -> Result<()> {
    let owners: BTreeSet<String> = metadata
        .workspace_packages()
        .into_iter()
        .filter(|pkg| depends_on_settings_crate(pkg) && registers_settings(pkg))
        .map(|pkg| pkg.name.to_string())
        .collect();
    let missing = unlinked(
        &owners,
        &link_closure(&workspace_graph(metadata, generator), generator),
    );
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "settings registered by {missing:?} would be missing from settings.schema.json: the \
         generator `{generator}` does not link them. Move the generator to a target that links \
         every settings crate (E05-S06b, issue #454) before adding settings there."
    )
}

/// Workspace dependency edges by crate name: normal dependencies for every crate, plus the
/// dev-dependencies of `generator` itself (an example links its package's dev-dependencies, but
/// dev-dependencies of its dependencies are not linked).
fn workspace_graph(metadata: &Metadata, generator: &str) -> BTreeMap<String, Vec<String>> {
    let members: BTreeSet<&str> = metadata
        .workspace_packages()
        .iter()
        .map(|pkg| pkg.name.as_str())
        .collect();
    metadata
        .workspace_packages()
        .iter()
        .map(|pkg| {
            let deps = pkg
                .dependencies
                .iter()
                .filter(|dep| members.contains(dep.name.as_str()))
                .filter(|dep| match dep.kind {
                    DependencyKind::Normal => true,
                    DependencyKind::Development => pkg.name.as_str() == generator,
                    _ => false,
                })
                .map(|dep| dep.name.clone())
                .collect();
            (pkg.name.to_string(), deps)
        })
        .collect()
}

/// Crates reachable from `root` (inclusive) in `graph`.
fn link_closure(graph: &BTreeMap<String, Vec<String>>, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_owned()];
    while let Some(name) = stack.pop() {
        if seen.insert(name.clone()) {
            stack.extend(graph.get(&name).into_iter().flatten().cloned());
        }
    }
    seen
}

/// The registering crates that are not in the linked set.
fn unlinked(owners: &BTreeSet<String>, linked: &BTreeSet<String>) -> Vec<String> {
    owners.difference(linked).cloned().collect()
}

/// Does any non-comment line of `source` invoke `register_settings!`?
fn invokes_register_settings(source: &str) -> bool {
    source
        .lines()
        .map(str::trim_start)
        .any(|line| !line.starts_with("//") && line.contains("register_settings!"))
}

/// Only a crate that depends on `oxikube_settings` can invoke its macro (this also skips the
/// settings crate itself, whose tests register fixtures, and files that merely mention the name).
fn depends_on_settings_crate(pkg: &Package) -> bool {
    pkg.dependencies
        .iter()
        .any(|dep| dep.name == SETTINGS_CRATE && dep.kind == DependencyKind::Normal)
}

/// Does any `.rs` file under the package's `src` register a setting?
fn registers_settings(pkg: &Package) -> bool {
    let Some(dir) = pkg.manifest_path.as_std_path().parent() else {
        return false;
    };
    source_files(&dir.join("src"))
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .any(|text| invokes_register_settings(&text))
}

fn source_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        edges
            .iter()
            .map(|(name, deps)| {
                (
                    (*name).to_owned(),
                    deps.iter().map(|d| (*d).to_owned()).collect(),
                )
            })
            .collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_settings_crate_the_generator_does_not_link_is_reported() {
        let graph = graph(&[
            ("oxikube_settings", &["oxikube_assets"]),
            ("oxikube_theme", &["oxikube_settings"]),
            ("oxikube_assets", &[]),
        ]);
        let linked = link_closure(&graph, "oxikube_settings");
        assert_eq!(
            unlinked(&set(&["oxikube_theme"]), &linked),
            ["oxikube_theme"]
        );
    }

    #[test]
    fn a_settings_crate_linked_directly_or_transitively_passes() {
        let graph = graph(&[
            ("generator", &["mid"]),
            ("mid", &["oxikube_theme"]),
            ("oxikube_theme", &["generator"]),
        ]);
        let linked = link_closure(&graph, "generator");
        assert!(unlinked(&set(&["oxikube_theme", "mid"]), &linked).is_empty());
    }

    #[test]
    fn only_real_invocations_count_as_registrations() {
        assert!(invokes_register_settings(
            "oxikube_settings::register_settings!(ThemeSettings);"
        ));
        assert!(invokes_register_settings("    register_settings!(A, B);"));
        assert!(!invokes_register_settings("// register_settings!(A);"));
        assert!(!invokes_register_settings("/// `register_settings!` docs"));
        assert!(!invokes_register_settings("fn plain() {}"));
    }

    /// The real workspace: every crate that registers a setting is linked into the generator.
    #[test]
    fn the_workspace_generator_links_every_settings_crate() {
        let metadata = cargo_metadata::MetadataCommand::new()
            .no_deps()
            .exec()
            .unwrap();
        ensure_generator_links_every_settings_crate(&metadata, "oxikube_settings").unwrap();
    }
}
