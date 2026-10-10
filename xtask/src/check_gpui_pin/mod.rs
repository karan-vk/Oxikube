//! GPUI pin alignment check.
//!
//! `gpui-component` is compiled against exactly one `gpui-pre` snapshot. The
//! [`PAIRS`] mirrors the pairing table in docs/adr/0003-gpui-dependency.md (the
//! source of truth); bump all pins, the table and `PAIRS` together in one PR.
//!
//! It also checks the GPUI patch overlay (ADR 0017, [`overlay`]): every patch directory matches
//! the pinned version and has its `[patch.crates-io]` entry, and every overlay crate is exactly
//! its pinned crate plus its patches (`scripts/gpui-overlay.sh --check`, which re-applies them).

mod overlay;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod script_tests;

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use std::process::Command;

/// (gpui-pre version, gpui-component version, zed commit named by the snapshot)
const PAIRS: &[(&str, &str, &str)] = &[("0.3.7", "0.7.0", "1a28cff")];

const GPUI_PRE_PACKAGES: &[&str] = &["gpui-pre", "gpui-pre-platform", "gpui-pre-macros"];
const GPUI_KIT_PACKAGES: &[&str] = &["gpui-component", "gpui-base", "gpui-kit-assets"];

pub fn run() -> Result<()> {
    let text = fs::read_to_string("Cargo.toml").context("read workspace Cargo.toml")?;
    let lock = fs::read_to_string("Cargo.lock").context("read workspace Cargo.lock")?;
    let readme =
        fs::read_to_string(Path::new(overlay::PATCHES_DIR).join("README.md")).unwrap_or_default();
    let dirs = read_patch_dirs(Path::new(overlay::PATCHES_DIR))?;
    let pinned = pinned_gpui_pre(&text);
    let mut errors = match check(&text)? {
        Ok(summary) => {
            println!("{summary}");
            Vec::new()
        }
        Err(errors) => errors,
    };
    errors.extend(overlay::check(&overlay::Inputs {
        manifest: &text,
        lock: &lock,
        readme: &readme,
        dirs: &dirs,
        pinned_gpui_pre: pinned.as_deref(),
    }));
    if !dirs.is_empty() {
        // The script rebuilds each overlay from its pinned crate in a scratch dir, re-applies the
        // patches (failing when one does not apply) and compares the result with `.gpui-overlay/`.
        let status = Command::new("bash")
            .args(["scripts/gpui-overlay.sh", "--check"])
            .status()
            .context("run scripts/gpui-overlay.sh --check (needs bash, tar, git, curl)")?;
        if !status.success() {
            errors.push(
                "the GPUI overlay is not its pinned crates plus their patches (see above); \
                 run scripts/gpui-overlay.sh"
                    .into(),
            );
        } else {
            let names: Vec<_> = dirs.iter().map(|d| d.name.as_str()).collect();
            println!(
                "check-gpui-pin: overlay {} = pinned crates + patches/gpui OK",
                names.join(", ")
            );
        }
    }
    if errors.is_empty() {
        return Ok(());
    }
    for e in &errors {
        eprintln!("error: {e}");
    }
    bail!("check-gpui-pin: {} problem(s)", errors.len())
}

/// The directories under `patches/gpui/` with their checksum and patch files.
fn read_patch_dirs(root: &Path) -> Result<Vec<overlay::PatchDir>> {
    let mut dirs = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(dirs);
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let mut patches = Vec::new();
        for file in fs::read_dir(&path)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            let numbered = name.len() > 5 && name.as_bytes()[..4].iter().all(u8::is_ascii_digit);
            if numbered && name.ends_with(".patch") {
                patches.push((name, fs::read_to_string(file.path())?));
            }
        }
        patches.sort();
        dirs.push(overlay::PatchDir {
            name: entry.file_name().to_string_lossy().into_owned(),
            checksum: fs::read_to_string(path.join("checksum")).ok(),
            patches,
        });
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(dirs)
}

/// The exact version `gpui-pre` is pinned at (`=x.y.z` without the `=`), if it is pinned exactly.
fn pinned_gpui_pre(manifest: &str) -> Option<String> {
    let doc: toml::Value = toml::from_str(manifest).ok()?;
    let deps = doc.get("workspace")?.get("dependencies")?.as_table()?;
    deps.iter().find_map(|(key, val)| {
        let tbl = val.as_table();
        let package = tbl
            .and_then(|t| t.get("package"))
            .and_then(|p| p.as_str())
            .unwrap_or(key);
        let version = tbl
            .and_then(|t| t.get("version"))
            .and_then(|v| v.as_str())
            .or_else(|| val.as_str())?;
        (package == "gpui-pre")
            .then(|| version.strip_prefix('=').map(str::to_owned))
            .flatten()
    })
}

/// Pure check over the workspace `Cargo.toml` text. The outer `Result` is a parse failure,
/// the inner one is the verdict: the success line to print, or the list of problems.
fn check(manifest: &str) -> Result<std::result::Result<String, Vec<String>>> {
    let doc: toml::Value = toml::from_str(manifest)?;
    let deps = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table())
        .context("[workspace.dependencies] missing")?;

    let mut pre_versions = Vec::new();
    let mut kit_versions = Vec::new();
    for (key, val) in deps {
        // `name = "=1.2.3"` (plain string) or `name = { package = "...", version = "..." }`.
        let tbl = val.as_table();
        let package = tbl
            .and_then(|t| t.get("package"))
            .and_then(|p| p.as_str())
            .unwrap_or(key);
        let version = tbl
            .and_then(|t| t.get("version"))
            .and_then(|v| v.as_str())
            .or_else(|| val.as_str());
        if GPUI_PRE_PACKAGES.contains(&package) {
            pre_versions.push((package.to_string(), version.map(str::to_string)));
        } else if GPUI_KIT_PACKAGES.contains(&package) {
            kit_versions.push((package.to_string(), version.map(str::to_string)));
        }
    }

    let mut errors = Vec::new();
    let exact = |pkg: &str, v: &Option<String>, errors: &mut Vec<String>| -> Option<String> {
        match v {
            Some(v) if v.starts_with('=') => Some(v[1..].to_string()),
            Some(v) => {
                errors.push(format!(
                    "{pkg}: version `{v}` must be an exact `=x.y.z` pin"
                ));
                None
            }
            None => {
                errors.push(format!("{pkg}: missing version"));
                None
            }
        }
    };
    let pre: Vec<_> = pre_versions
        .iter()
        .filter_map(|(p, v)| exact(p, v, &mut errors))
        .collect();
    let kit: Vec<_> = kit_versions
        .iter()
        .filter_map(|(p, v)| exact(p, v, &mut errors))
        .collect();
    if pre.is_empty() {
        errors.push("no gpui-pre dependency found".into());
    }
    if kit.is_empty() {
        errors.push("no gpui-component dependency found".into());
    }
    if pre.windows(2).any(|w| w[0] != w[1]) {
        errors.push(format!("gpui-pre-* versions differ: {pre:?}"));
    }
    if kit.windows(2).any(|w| w[0] != w[1]) {
        errors.push(format!("gpui-kit crate versions differ: {kit:?}"));
    }
    let mut summary = String::new();
    if let (Some(p), Some(k)) = (pre.first(), kit.first()) {
        match PAIRS.iter().find(|(pv, kv, _)| pv == p && kv == k) {
            Some((_, _, zed)) => {
                summary = format!(
                    "check-gpui-pin: gpui-pre ={p} ↔ gpui-component ={k} (snapshot of zed@{zed}) OK"
                );
            }
            None => errors.push(format!(
                "gpui-pre ={p} and gpui-component ={k} are not a known pairing; update PAIRS in xtask/src/check_gpui_pin/mod.rs and the table in docs/adr/0003-gpui-dependency.md"
            )),
        }
    }
    Ok(if errors.is_empty() {
        Ok(summary)
    } else {
        Err(errors)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(pre: &str, platform: &str, component: &str, base: &str) -> String {
        format!(
            r#"
[workspace.dependencies]
anyhow = "1"
gpui = {{ package = "gpui-pre", version = "{pre}" }}
gpui_platform = {{ package = "gpui-pre-platform", version = "{platform}", features = ["x11"] }}
gpui-component = "{component}"
gpui-base = {{ version = "{base}" }}
gpui-kit-assets = "{component}"
"#
        )
    }

    fn errors(manifest: &str) -> Vec<String> {
        check(manifest).unwrap().unwrap_err()
    }

    #[test]
    fn exact_aligned_pair_passes_and_names_the_zed_commit() {
        let ok = check(&manifest("=0.3.7", "=0.3.7", "=0.7.0", "=0.7.0"))
            .unwrap()
            .unwrap();
        assert!(ok.contains("zed@1a28cff"), "{ok}");
        assert!(ok.contains("gpui-pre =0.3.7") && ok.contains("gpui-component =0.7.0"));
    }

    #[test]
    fn real_workspace_manifest_passes() {
        let text = include_str!("../../../Cargo.toml");
        assert!(check(text).unwrap().is_ok());
    }

    #[test]
    fn caret_or_bare_version_is_rejected() {
        for v in ["0.3", "0.3.7", "^0.3.7", "~0.3.7"] {
            let e = errors(&manifest(v, "=0.3.7", "=0.7.0", "=0.7.0"));
            assert!(
                e.iter().any(|m| m.contains("must be an exact")),
                "{v}: {e:?}"
            );
        }
        let e = errors(&manifest("=0.3.7", "=0.3.7", "0.7.0", "=0.7.0"));
        assert!(
            e.iter()
                .any(|m| m.contains("gpui-component: version `0.7.0` must be an exact"))
        );
    }

    #[test]
    fn plain_string_entries_are_checked_too() {
        let m = "[workspace.dependencies]\ngpui-pre = \"0.3\"\ngpui-component = \"=0.7.0\"\n";
        let e = errors(m);
        assert!(
            e.iter()
                .any(|m| m.contains("gpui-pre: version `0.3` must be an exact")),
            "{e:?}"
        );
    }

    #[test]
    fn unknown_pairing_is_rejected() {
        let e = errors(&manifest("=0.3.7", "=0.3.7", "=0.6.6", "=0.6.6"));
        assert!(e.iter().any(|m| m.contains("not a known pairing")), "{e:?}");
    }

    #[test]
    fn mixed_versions_within_a_family_are_rejected() {
        let e = errors(&manifest("=0.3.7", "=0.3.6", "=0.7.0", "=0.7.0"));
        assert!(
            e.iter().any(|m| m.contains("gpui-pre-* versions differ")),
            "{e:?}"
        );
        let e = errors(&manifest("=0.3.7", "=0.3.7", "=0.7.0", "=0.6.6"));
        assert!(
            e.iter()
                .any(|m| m.contains("gpui-kit crate versions differ")),
            "{e:?}"
        );
    }

    #[test]
    fn missing_families_are_rejected() {
        let e = errors("[workspace.dependencies]\nanyhow = \"1\"\n");
        assert!(e.iter().any(|m| m.contains("no gpui-pre dependency")));
        assert!(e.iter().any(|m| m.contains("no gpui-component dependency")));
    }
}
