//! Guard: every crate that registers a setting must be linked into the schema generator.
//!
//! `inventory` only sees registrations from crates linked into the generator binary, so a
//! setting registered anywhere else silently never reaches `settings.schema.json` (and the
//! schema's `additionalProperties: false` then flags the user's key as unknown). `--check`
//! compares the schema with the generator's own output, so it cannot see that gap. This guard
//! finds the crates whose source invokes `register_settings!`, asks the generator which crates
//! actually registered something, and fails on any crate in the first set but not the second.
//! Comparing against what the real binary reports (rather than reasoning about the dependency
//! graph) also catches a crate that is a dependency but never referenced, which the linker drops.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Result, bail};
use cargo_metadata::{DependencyKind, Metadata, Package};

/// The crate that defines `register_settings!`.
const SETTINGS_CRATE: &str = "oxikube_settings";

/// Fail when a workspace crate registers settings but `linked` (the generator's own list of
/// registering crates, one per line) does not contain it.
pub fn ensure_every_settings_crate_is_linked(
    metadata: &Metadata,
    linked: &str,
    generator: &str,
) -> Result<()> {
    let owners: BTreeSet<String> = metadata
        .workspace_packages()
        .into_iter()
        .filter(|pkg| depends_on_settings_crate(pkg) && registers_settings(pkg))
        .map(|pkg| pkg.name.to_string())
        .collect();
    let missing = unlinked(&owners, &parse_linked(linked));
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "settings registered by {missing:?} would be missing from settings.schema.json: the \
         generator `{generator}` does not link them. Depend on each crate from `{generator}` and \
         call its `init` (or otherwise reference it) so the linker keeps its registrations."
    )
}

/// The crate names in the generator's `--print-settings-crates` output.
fn parse_linked(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The registering crates that the generator did not report.
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

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_settings_crate_the_generator_does_not_report_is_missing() {
        let linked = parse_linked("oxikube_logging\noxikube_workspace\n");
        assert_eq!(
            unlinked(&set(&["oxikube_logging", "oxikube_theme"]), &linked),
            ["oxikube_theme"]
        );
    }

    #[test]
    fn every_reported_settings_crate_passes() {
        let linked = parse_linked("oxikube_logging\n\n oxikube_theme \n");
        assert!(unlinked(&set(&["oxikube_logging", "oxikube_theme"]), &linked).is_empty());
        // Extra crates the sources do not mention (test fixtures, transitive) are fine.
        assert!(unlinked(&set(&[]), &linked).is_empty());
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

    /// The real workspace: the app binary reports every crate whose source registers a setting,
    /// and the guard fails when one is withheld from the generator's list.
    #[test]
    fn the_app_binary_links_every_settings_crate() {
        let metadata = cargo_metadata::MetadataCommand::new()
            .no_deps()
            .exec()
            .unwrap();
        let root = metadata.workspace_root.as_std_path();
        let linked = super::super::run_generator(root, super::super::CRATES_FLAG).unwrap();
        ensure_every_settings_crate_is_linked(&metadata, &linked, "oxikube").unwrap();

        let without_theme: String = linked
            .lines()
            .filter(|line| *line != "oxikube_theme")
            .map(|line| format!("{line}\n"))
            .collect();
        let err = ensure_every_settings_crate_is_linked(&metadata, &without_theme, "oxikube")
            .unwrap_err()
            .to_string();
        assert!(err.contains("oxikube_theme"), "{err}");
    }
}
