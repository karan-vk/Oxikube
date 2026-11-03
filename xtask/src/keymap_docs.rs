//! `cargo xtask gen-keymap-docs [--check]`: write or verify `docs/keymap.md` (E21-S09).
//!
//! Same pattern as [`crate::keymap_schema`]: the table is rendered from the keymaps embedded in
//! the app and the command titles, so the app binary prints it (`oxikube --print-keymap-docs`)
//! and xtask only compares or writes the text. The output is deterministic, so `--check` is a
//! byte comparison and CI fails when a binding or a title changed without regenerating the file.

use anyhow::{Context, Result, bail};

use crate::settings_schema::run_generator;

/// Where the document lives, relative to the workspace root (mirrors
/// `oxikube_keymap::docs::KEYMAP_DOCS_PATH`).
pub const DOCS_PATH: &str = "docs/keymap.md";

/// Hidden `oxikube` flag that prints `docs/keymap.md`.
const DOCS_FLAG: &str = "--print-keymap-docs";

#[derive(clap::Args)]
pub struct Args {
    /// Fail if the checked-in document differs from a fresh generation instead of writing it.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: &Args) -> Result<()> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
    let root = metadata.workspace_root.as_std_path();
    let generated = run_generator(root, DOCS_FLAG)?;
    let path = root.join(DOCS_PATH);
    if args.check {
        let existing = std::fs::read_to_string(&path).ok();
        check(existing.as_deref(), &generated)?;
        println!("{DOCS_PATH} is up to date");
    } else {
        std::fs::write(&path, &generated).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {DOCS_PATH}");
    }
    Ok(())
}

/// The `--check` verdict: the checked-in text must equal the generated text.
pub fn check(existing: Option<&str>, generated: &str) -> Result<()> {
    match existing {
        None => bail!("{DOCS_PATH} is missing; run `cargo xtask gen-keymap-docs`"),
        Some(existing) if existing != generated => bail!(
            "{DOCS_PATH} is stale (a keymap or a command title changed); run `cargo xtask gen-keymap-docs` and commit the result"
        ),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_on_identical_text_and_fails_when_a_binding_changed() {
        let docs = "| `j` | `resource_table::SelectNext` |\n";
        assert!(check(Some(docs), docs).is_ok());
        let changed = docs.replace('j', "n");
        let stale = check(Some(docs), &changed).unwrap_err().to_string();
        assert!(stale.contains("stale"), "{stale}");
        assert!(
            check(None, docs)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
    }

    #[test]
    fn docs_path_matches_the_keymap_crate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let source =
            std::fs::read_to_string(root.join("crates/platform/oxikube_keymap/src/docs.rs"))
                .unwrap();
        assert!(source.contains(&format!("\"{DOCS_PATH}\"")));
    }
}
