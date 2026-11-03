//! `cargo xtask gen-keymap-schema [--check]`: write or verify `keymap.schema.json` (E21-S03).
//!
//! Same pattern as [`crate::settings_schema`]: the schema lists the actions registered by every
//! crate linked into the app, so the app binary prints it (`oxikube --print-keymap-schema`) and
//! xtask only compares or writes the text. The output is canonical (keys sorted, actions
//! sorted), so `--check` is a byte comparison and CI fails when an action was added or renamed
//! without regenerating the file.

use anyhow::{Context, Result, bail};

use crate::settings_schema::run_generator;

/// Where the schema lives, relative to the workspace root (mirrors
/// `oxikube_assets::KEYMAP_SCHEMA_PATH`).
pub const SCHEMA_PATH: &str = "crates/platform/oxikube_assets/assets/keymaps/keymap.schema.json";

/// Hidden `oxikube` flag that prints `keymap.schema.json`.
const SCHEMA_FLAG: &str = "--print-keymap-schema";

#[derive(clap::Args)]
pub struct Args {
    /// Fail if the checked-in schema differs from a fresh generation instead of writing it.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: &Args) -> Result<()> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
    let root = metadata.workspace_root.as_std_path();
    let generated = run_generator(root, SCHEMA_FLAG)?;
    serde_json::from_str::<serde_json::Value>(&generated)
        .context("generator printed invalid JSON")?;
    let path = root.join(SCHEMA_PATH);
    if args.check {
        let existing = std::fs::read_to_string(&path).ok();
        check(existing.as_deref(), &generated)?;
        println!("{SCHEMA_PATH} is up to date");
    } else {
        std::fs::write(&path, &generated).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {SCHEMA_PATH}");
    }
    Ok(())
}

/// The `--check` verdict: the checked-in text must equal the generated text.
pub fn check(existing: Option<&str>, generated: &str) -> Result<()> {
    match existing {
        None => bail!("{SCHEMA_PATH} is missing; run `cargo xtask gen-keymap-schema`"),
        Some(existing) if existing != generated => bail!(
            "{SCHEMA_PATH} is stale (an action changed); run `cargo xtask gen-keymap-schema` and commit the result"
        ),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_only_on_an_identical_schema() {
        let schema = "{\n  \"type\": \"array\"\n}\n";
        assert!(check(Some(schema), schema).is_ok());
        assert!(
            check(Some("{}\n"), schema)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert!(
            check(None, schema)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
    }

    #[test]
    fn schema_path_matches_the_assets_crate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let assets =
            std::fs::read_to_string(root.join("crates/platform/oxikube_assets/src/keymap.rs"))
                .unwrap();
        assert!(assets.contains(&format!("\"{SCHEMA_PATH}\"")));
        assert!(root.join(SCHEMA_PATH).exists());
    }
}
