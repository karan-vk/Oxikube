//! The overlay's configuration checks (ADR 0017), on the real repository and on small fixtures.

use std::path::Path;

use super::overlay::{Inputs, PatchDir, check, split_name};
use super::{pinned_gpui_pre, read_patch_dirs};

const SUM: &str = "5a43af845b260b09393e923c4f1e1a67a10fcfc847b4815192bbfd02ed9fe725";

const PATCH: &str = "Fix something.\nUpstream: zed-industries/zed draft\nPatch: GPL-3.0-or-later\n\n\
diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n";

const MANIFEST: &str = r#"
[workspace]
members = ["bins/oxikube"]
exclude = [".gpui-overlay"]

[workspace.dependencies]
gpui = { package = "gpui-pre", version = "=0.3.7" }

[patch.crates-io]
gpui-pre-macos = { path = ".gpui-overlay/gpui-pre-macos-0.3.7" }
"#;

const LOCK: &str = r#"
version = 4

[[package]]
name = "gpui-pre-macos"
version = "0.3.7"

[[package]]
name = "gpui-pre-apple"
version = "0.3.7"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00053551517815ab169fda28fff78dbebf2712722593c7671c511785cb3f79d5"
"#;

const README: &str = "| gpui-pre-macos-0.3.7/0001-fix.patch | why | upstream |";

fn dir(name: &str) -> PatchDir {
    PatchDir {
        name: name.into(),
        checksum: Some(format!("{SUM}\n")),
        patches: vec![("0001-fix.patch".into(), PATCH.into())],
    }
}

fn errors_for(manifest: &str, lock: &str, readme: &str, dirs: &[PatchDir]) -> Vec<String> {
    check(&Inputs {
        manifest,
        lock,
        readme,
        dirs,
        pinned_gpui_pre: pinned_gpui_pre(manifest).as_deref(),
    })
}

fn assert_has(errors: &[String], needle: &str) {
    assert!(
        errors.iter().any(|e| e.contains(needle)),
        "expected an error containing `{needle}`, got {errors:#?}"
    );
}

#[test]
fn the_real_repository_overlay_is_consistent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let readme = std::fs::read_to_string(root.join("patches/gpui/README.md")).unwrap();
    let dirs = read_patch_dirs(&root.join("patches/gpui")).unwrap();
    assert!(!dirs.is_empty(), "the overlay has patch directories");
    let errors = errors_for(&manifest, &lock, &readme, &dirs);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_consistent_fixture_passes() {
    let errors = errors_for(MANIFEST, LOCK, README, &[dir("gpui-pre-macos-0.3.7")]);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_patch_dir_for_another_version_than_the_pin_is_rejected() {
    let manifest = MANIFEST.replace("gpui-pre-macos-0.3.7", "gpui-pre-macos-0.3.6");
    let errors = errors_for(&manifest, LOCK, README, &[dir("gpui-pre-macos-0.3.6")]);
    assert_has(&errors, "pins gpui-pre =0.3.7, not 0.3.6");
    assert_has(
        &errors,
        "Cargo.lock resolves gpui-pre-macos 0.3.7, not 0.3.6",
    );
}

#[test]
fn the_patch_entry_must_point_at_the_overlay_and_nowhere_else() {
    let manifest = MANIFEST.replace(
        ".gpui-overlay/gpui-pre-macos-0.3.7",
        "../zed/crates/gpui_macos",
    );
    let errors = errors_for(&manifest, LOCK, README, &[dir("gpui-pre-macos-0.3.7")]);
    assert_has(
        &errors,
        "must be exactly { path = \".gpui-overlay/gpui-pre-macos-0.3.7\" }",
    );

    let manifest = MANIFEST.replace("[patch.crates-io]\ngpui-pre-macos", "[patch.crates-io]\nx");
    let errors = errors_for(&manifest, LOCK, README, &[dir("gpui-pre-macos-0.3.7")]);
    assert_has(&errors, "[patch.crates-io] has no gpui-pre-macos");
}

#[test]
fn a_gpui_patch_without_patch_files_or_from_git_is_rejected() {
    let manifest = format!(
        "{MANIFEST}gpui-pre-apple = {{ path = \".gpui-overlay/gpui-pre-apple-0.3.7\" }}\n\n\
         [patch.\"https://github.com/zed-industries/zed\"]\ngpui = {{ path = \"x\" }}\n"
    );
    let errors = errors_for(&manifest, LOCK, README, &[dir("gpui-pre-macos-0.3.7")]);
    assert_has(&errors, "gpui-pre-apple = ");
    assert_has(&errors, "has no patches/gpui/gpui-pre-apple-<version>/");
    assert_has(&errors, "never from another source");
}

#[test]
fn the_overlay_must_be_used_and_kept_out_of_the_workspace() {
    let lock = LOCK.replacen(
        "name = \"gpui-pre-macos\"\nversion = \"0.3.7\"\n",
        "name = \"gpui-pre-macos\"\nversion = \"0.3.7\"\nsource = \"registry+x\"\n",
        1,
    );
    let manifest = MANIFEST.replace("exclude = [\".gpui-overlay\"]\n", "");
    let errors = errors_for(&manifest, &lock, README, &[dir("gpui-pre-macos-0.3.7")]);
    assert_has(&errors, "overlay is not used");
    assert_has(&errors, "[workspace] exclude must list \".gpui-overlay\"");
}

#[test]
fn a_patch_needs_its_header_its_readme_row_and_a_checksum() {
    let mut bad = dir("gpui-pre-macos-0.3.7");
    bad.checksum = Some("not-a-sum".into());
    bad.patches = vec![("0002-other.patch".into(), "diff --git a/x b/x\n".into())];
    let errors = errors_for(MANIFEST, LOCK, README, &[bad]);
    assert_has(&errors, "checksum is not a sha256");
    assert_has(&errors, "must name `Upstream:`");
    assert_has(&errors, "must name `GPL-3.0-or-later`");
    assert_has(
        &errors,
        "README.md does not list gpui-pre-macos-0.3.7/0002-other.patch",
    );

    let mut empty = dir("gpui-pre-macos-0.3.7");
    empty.checksum = None;
    empty.patches.clear();
    let errors = errors_for(MANIFEST, LOCK, README, &[empty]);
    assert_has(&errors, "checksum is missing");
    assert_has(&errors, "has no NNNN-<slug>.patch");
}

#[test]
fn only_gpui_crates_with_a_version_are_patched() {
    assert_eq!(
        split_name("gpui-pre-macos-0.3.7"),
        Some(("gpui-pre-macos", "0.3.7"))
    );
    assert_eq!(split_name("gpui-pre-macos"), None);
    assert_eq!(split_name("gpui-pre-macos-0.3"), None);
    let errors = errors_for(MANIFEST, LOCK, README, &[dir("serde-1.0.0")]);
    assert_has(&errors, "only GPUI crates");
    let errors = errors_for(MANIFEST, LOCK, README, &[dir("gpui-pre-macos")]);
    assert_has(&errors, "name it <crate>-<version>");
}
