//! `cargo xtask gen-settings-schema [--check]`: write or verify `settings.schema.json`.
//!
//! The schema comes from the registered settings types, so it is produced by the app binary,
//! which links every crate that registers settings: `oxikube --print-settings-schema` prints it
//! (xtask does not depend on the GPUI-based crates, which keeps the pre-commit hook fast). The
//! output is canonical (keys sorted), so `--check` is a byte comparison; CI runs it so a
//! settings change without a regenerated schema fails. A crate that registers settings but is
//! not linked into the binary would be missing from the schema in a way `--check` cannot see, so
//! [`coverage`] compares the crates the binary reports (`--print-settings-crates`) with the
//! crates whose source invokes `register_settings!` and fails the command on any difference
//! (E05-S06b).

mod coverage;

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Where the schema lives, relative to the workspace root (mirrors
/// `oxikube_assets::SETTINGS_SCHEMA_PATH`).
pub const SCHEMA_PATH: &str = "crates/platform/oxikube_assets/assets/settings/settings.schema.json";

/// The package whose binary prints the schema (`bins/oxikube`).
const GENERATOR: &str = "oxikube";

/// Hidden `oxikube` flag that prints `settings.schema.json`.
const SCHEMA_FLAG: &str = "--print-settings-schema";

/// Hidden `oxikube` flag that prints the crates that registered settings, one per line.
const CRATES_FLAG: &str = "--print-settings-crates";

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
    let linked = run_generator(root, CRATES_FLAG)?;
    coverage::ensure_every_settings_crate_is_linked(&metadata, &linked, GENERATOR)?;
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

/// Run the app binary with one of the hidden print flags and return its stdout.
fn run_generator(root: &Path, flag: &str) -> Result<String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["run", "-q", "-p", GENERATOR, "--", flag])
        .output()
        .with_context(|| format!("running `oxikube {flag}`"))?;
    if !output.status.success() {
        bail!(
            "`oxikube {flag}` failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("generator output is not UTF-8")
}

/// The `--check` verdict: the checked-in text must equal the generated text.
pub fn check(existing: Option<&str>, generated: &str) -> Result<()> {
    match existing {
        None => bail!("{SCHEMA_PATH} is missing; run `cargo xtask gen-settings-schema`"),
        Some(existing) if existing != generated => bail!(
            "{SCHEMA_PATH} is stale (a settings type changed); run `cargo xtask gen-settings-schema` and commit the result"
        ),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_only_on_an_identical_schema() {
        let schema = "{\n  \"type\": \"object\"\n}\n";
        assert!(check(Some(schema), schema).is_ok());

        let stale = check(Some("{}\n"), schema).unwrap_err().to_string();
        assert!(stale.contains("stale"), "{stale}");

        let missing = check(None, schema).unwrap_err().to_string();
        assert!(missing.contains("missing"), "{missing}");
    }

    #[test]
    fn schema_path_matches_the_assets_crate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let assets =
            std::fs::read_to_string(root.join("crates/platform/oxikube_assets/src/settings.rs"))
                .unwrap();
        assert!(assets.contains(&format!("\"{SCHEMA_PATH}\"")));
        assert!(root.join(SCHEMA_PATH).exists());
    }
}
