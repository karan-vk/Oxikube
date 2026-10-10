//! The GPUI patch overlay (ADR 0017): `patches/gpui/<crate>-<version>/` holds unified diffs
//! against the exact pinned crate, `scripts/gpui-overlay.sh` builds `.gpui-overlay/<crate>-<version>/`
//! from the crates.io `.crate` plus those diffs, and `[patch.crates-io]` points the crate there.
//!
//! [`check`] is the pure part (manifest, lockfile, patch directories, README); `run` in the parent
//! module then asks the script (`--check`) whether every overlay is exactly its pinned crate plus
//! its patches, which also proves that the patches apply cleanly.

use std::collections::BTreeSet;

/// Where the overlay lives, relative to the workspace root.
pub const OVERLAY_DIR: &str = ".gpui-overlay";
/// Where the patches live, relative to the workspace root.
pub const PATCHES_DIR: &str = "patches/gpui";

/// One `patches/gpui/<crate>-<version>/` directory as found on disk.
#[derive(Debug, Clone)]
pub struct PatchDir {
    /// The directory name, `<crate>-<version>`.
    pub name: String,
    /// The `checksum` file's contents, if it exists.
    pub checksum: Option<String>,
    /// The `NNNN-<slug>.patch` files: name and contents, in apply order.
    pub patches: Vec<(String, String)>,
}

/// What the overlay check reads.
pub struct Inputs<'a> {
    /// The workspace `Cargo.toml`.
    pub manifest: &'a str,
    /// The workspace `Cargo.lock`.
    pub lock: &'a str,
    /// `patches/gpui/README.md` (empty when missing).
    pub readme: &'a str,
    /// Every directory under `patches/gpui/`.
    pub dirs: &'a [PatchDir],
    /// The exact `gpui-pre` version the workspace pins (from the pin check).
    pub pinned_gpui_pre: Option<&'a str>,
}

/// Every problem with the overlay's configuration; empty when it is consistent.
pub fn check(inputs: &Inputs<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let manifest: toml::Value = match toml::from_str(inputs.manifest) {
        Ok(v) => v,
        Err(e) => return vec![format!("Cargo.toml does not parse: {e}")],
    };
    let lock: toml::Value = match toml::from_str(inputs.lock) {
        Ok(v) => v,
        Err(e) => return vec![format!("Cargo.lock does not parse: {e}")],
    };

    let excluded = manifest
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(|e| e.as_array())
        .is_some_and(|e| e.iter().any(|v| v.as_str() == Some(OVERLAY_DIR)));
    if !inputs.dirs.is_empty() && !excluded {
        errors.push(format!(
            "[workspace] exclude must list \"{OVERLAY_DIR}\": the overlay crates are path crates \
             inside the workspace and would otherwise become members"
        ));
    }

    let mut dir_crates = BTreeSet::new();
    for dir in inputs.dirs {
        let Some((krate, version)) = split_name(&dir.name) else {
            errors.push(format!(
                "{PATCHES_DIR}/{}: name it <crate>-<version>, e.g. gpui-pre-macos-0.3.7",
                dir.name
            ));
            continue;
        };
        dir_crates.insert(krate.to_owned());
        check_dir(dir, krate, version, inputs, &lock, &mut errors);
        check_patch_entry(&manifest, krate, &dir.name, &mut errors);
    }

    // Every GPUI-family patch must come from the overlay: no git source, no other path.
    for table in patch_tables(&manifest) {
        let (source, entries) = table;
        for (name, entry) in entries {
            if !is_gpui_family(name) {
                continue;
            }
            if source != "crates-io" {
                errors.push(format!(
                    "[patch.{source}] {name}: GPUI crates are patched only through the overlay \
                     ([patch.crates-io] -> {OVERLAY_DIR}/), never from another source (ADR 0003, 0017)"
                ));
            } else if !dir_crates.contains(name.as_str()) {
                errors.push(format!(
                    "[patch.crates-io] {name} = {entry} has no {PATCHES_DIR}/{name}-<version>/; \
                     a GPUI crate is patched only by patch files applied to its pinned version"
                ));
            }
        }
    }
    errors
}

fn check_dir(
    dir: &PatchDir,
    krate: &str,
    version: &str,
    inputs: &Inputs<'_>,
    lock: &toml::Value,
    errors: &mut Vec<String>,
) {
    let at = format!("{PATCHES_DIR}/{}", dir.name);
    if !is_gpui_family(krate) {
        errors.push(format!(
            "{at}: only GPUI crates (gpui-pre*) are patched by the overlay"
        ));
    }
    if krate.starts_with("gpui-pre")
        && let Some(pinned) = inputs.pinned_gpui_pre
        && version != pinned
    {
        errors.push(format!(
            "{at}: the workspace pins gpui-pre ={pinned}, not {version}. At a pin bump, delete the \
             patches upstream has shipped and rebase the rest onto the new version (rename the \
             directory and its [patch.crates-io] path)"
        ));
    }
    match &dir.checksum {
        Some(sum) if is_sha256(sum.trim()) => {}
        Some(_) => errors.push(format!("{at}/checksum is not a sha256 hex digest")),
        None => errors.push(format!(
            "{at}/checksum is missing (the sha256 of {}.crate from Cargo.lock / the crates.io index)",
            dir.name
        )),
    }
    if dir.patches.is_empty() {
        errors.push(format!(
            "{at} has no NNNN-<slug>.patch; delete the directory and its [patch.crates-io] entry"
        ));
    }
    for (file, text) in &dir.patches {
        let header = text.split("\ndiff --git ").next().unwrap_or_default();
        if !text.contains("diff --git ") {
            errors.push(format!("{at}/{file} is not a git-style unified diff"));
        }
        for needle in ["Upstream:", "GPL-3.0-or-later"] {
            if !header.contains(needle) {
                errors.push(format!(
                    "{at}/{file}: its header (before the diff) must name `{needle}` (see \
                     {PATCHES_DIR}/README.md)"
                ));
            }
        }
        if !inputs.readme.contains(file.as_str()) {
            errors.push(format!(
                "{PATCHES_DIR}/README.md does not list {}/{file} (why it exists, upstream status)",
                dir.name
            ));
        }
    }
    match locked(lock, krate) {
        None => errors.push(format!("{at}: Cargo.lock has no {krate}")),
        Some(entries) => {
            if !entries.iter().any(|(v, _)| v == version) {
                let found: Vec<_> = entries.iter().map(|(v, _)| v.as_str()).collect();
                errors.push(format!(
                    "{at}: Cargo.lock resolves {krate} {}, not {version}",
                    found.join(", ")
                ));
            }
            if entries
                .iter()
                .any(|(v, source)| v == version && source.is_some())
            {
                errors.push(format!(
                    "{at}: Cargo.lock still takes {krate} {version} from the registry, so the \
                     overlay is not used (check the [patch.crates-io] entry, then `cargo update -p {krate}`)"
                ));
            }
        }
    }
}

fn check_patch_entry(manifest: &toml::Value, krate: &str, dir: &str, errors: &mut Vec<String>) {
    let want = format!("{OVERLAY_DIR}/{dir}");
    let entry = manifest
        .get("patch")
        .and_then(|p| p.get("crates-io"))
        .and_then(|p| p.get(krate));
    let Some(entry) = entry else {
        errors.push(format!(
            "[patch.crates-io] has no {krate} = {{ path = \"{want}\" }} for {PATCHES_DIR}/{dir}"
        ));
        return;
    };
    let table = entry.as_table();
    let path = table.and_then(|t| t.get("path")).and_then(|p| p.as_str());
    if path != Some(want.as_str()) || table.is_some_and(|t| t.len() != 1) {
        errors.push(format!(
            "[patch.crates-io] {krate} must be exactly {{ path = \"{want}\" }}, found {entry}"
        ));
    }
}

/// `[patch.<source>]` tables: source name and entries.
fn patch_tables(manifest: &toml::Value) -> Vec<(&str, Vec<(&String, &toml::Value)>)> {
    let Some(patch) = manifest.get("patch").and_then(|p| p.as_table()) else {
        return Vec::new();
    };
    patch
        .iter()
        .filter_map(|(source, entries)| {
            Some((source.as_str(), entries.as_table()?.iter().collect()))
        })
        .collect()
}

/// Every `[[package]]` of `name` in the lockfile: version and source (none for a path crate).
fn locked(lock: &toml::Value, name: &str) -> Option<Vec<(String, Option<String>)>> {
    let entries: Vec<_> = lock
        .get("package")?
        .as_array()?
        .iter()
        .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
        .map(|p| {
            let version = p
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let source = p.get("source").and_then(|s| s.as_str()).map(str::to_owned);
            (version.to_owned(), source)
        })
        .collect();
    (!entries.is_empty()).then_some(entries)
}

/// `gpui-pre-macos-0.3.7` -> (`gpui-pre-macos`, `0.3.7`).
pub fn split_name(name: &str) -> Option<(&str, &str)> {
    let (krate, version) = name.rsplit_once('-')?;
    let parts: Vec<_> = version.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (numeric && !krate.is_empty()).then_some((krate, version))
}

/// The crates the overlay may patch: the `gpui-pre` snapshot family and the gpui-kit crates
/// pinned with it.
fn is_gpui_family(name: &str) -> bool {
    name.starts_with("gpui")
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
